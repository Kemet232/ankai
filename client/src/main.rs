// ANKAI client — native desktop UI shell (Slint, per ADR-0002).
//
// Default entry point: a minimal window + navigation skeleton (sidebar with
// placeholder destinations, swappable content pane). This is intentionally
// bare. Per ADR-0002's "Spike / prototype plan" gate
// (docs/adr/0002-native-ui-stack.md), native-shell work must not go much
// beyond a window + navigation skeleton until two spikes land: (1) low-end-
// hardware frame-time profiling and (2) accessibility validation with real
// screen readers. Neither is done yet — see that ADR's "Spike 1 results"
// section and PROGRESS.md. Do not add visual polish (glass/blur,
// translucency, animation) here until that gate clears; see
// client/ui/app.slint for the full rationale.
//
// Legacy/debug path: the throwaway glass/blur + drag-reorder rendering
// spike from ADR-0002 spike 1 is still available for reference, gated
// behind `--spike` (or ANKAI_SPIKE_DEBUG=1), since it's not the default
// shell anymore. See client/ui/spike-glass-blur.slint — it is not part of
// the real app.

slint::include_modules!();

use std::cell::RefCell;
use std::rc::Rc;

use slint::Model;

/// This device's `Db`/`AnkaiMlsProvider` handles, looked up by the P2P
/// receive loop's UI-thread callback (see [`MESSAGING_HANDLES`]) rather
/// than captured directly inside it.
///
/// Why this indirection exists: `slint::invoke_from_event_loop` (how the
/// tokio-driven receive loop below hands a message back to the UI thread)
/// requires its closure to be `Send + 'static`, but `Rc<Db>`/
/// `Rc<AnkaiMlsProvider>` are deliberately `Rc`, not `Arc` — `Db` wraps a
/// `rusqlite::Connection`, which is `Send` but not `Sync`, so it's only
/// ever touched from the single UI thread that owns it (same pattern the
/// Settings/Communities panes already use). A thread-local, populated once
/// at startup before `app.run()` and read back only from closures that are
/// guaranteed (by `invoke_from_event_loop`'s own contract) to run on that
/// same UI thread, lets the cross-thread closure itself stay `Send` (it
/// only carries the `EndpointId`/`Vec<u8>` it received, both `Send`) while
/// the actual decrypt-and-persist work still runs against the single
/// long-lived `Db`/`AnkaiMlsProvider` the rest of the app already uses.
#[derive(Clone)]
struct MessagingHandles {
    db: Rc<ankai_core::db::Db>,
    mls_provider: Rc<ankai_core::mls_provider::AnkaiMlsProvider>,
}

thread_local! {
    static MESSAGING_HANDLES: RefCell<Option<MessagingHandles>> = const { RefCell::new(None) };
}

/// Opens (creating if necessary) ANKAI's local encrypted database in this
/// platform's standard app-data directory, keyed by the device-local
/// passphrase from `ankai_core::keychain` (OS secure storage, per
/// ADR-0004). Panics on failure — without a working local DB there's
/// nothing useful the app can do, so there's no graceful degraded mode to
/// fall back to here.
fn open_local_db() -> ankai_core::db::Db {
    let dirs = directories::ProjectDirs::from("com", "ankai", "ANKAI")
        .expect("no valid app data directory for this platform/user");
    std::fs::create_dir_all(dirs.data_dir()).expect("failed to create app data directory");

    let passphrase = ankai_core::keychain::device_db_passphrase()
        .expect("failed to obtain device DB encryption key from OS secure storage");

    ankai_core::db::Db::open(dirs.data_dir().join("ankai.sqlite"), &passphrase)
        .expect("failed to open local encrypted database")
}

/// Truncates a `messaging::peer_id_for`-style hex peer id to a short,
/// display-friendly prefix — the same idea as iroh's own `fmt_short`, for
/// the string peer ids this module works with instead of raw `EndpointId`s.
fn short_peer_id(peer_id: &str) -> &str {
    &peer_id[..peer_id.len().min(10)]
}

fn main() -> Result<(), slint::PlatformError> {
    // Prefer the Skia renderer for full-effects mode per ADR-0002. If Skia
    // can't be selected (missing at compile time, or backend init fails at
    // runtime on this machine), fall back to Slint's default backend
    // selection (FemtoVG/OpenGL, then software) rather than hard-erroring —
    // this mirrors the "Potato mode" fallback path the ADR calls for.
    if let Err(err) = slint::BackendSelector::new()
        .renderer_name("skia".into())
        .select()
    {
        eprintln!(
            "ankai-client: Skia renderer unavailable ({err}); falling back to Slint's default backend/renderer selection."
        );
    }

    let spike_debug = std::env::args().any(|arg| arg == "--spike")
        || std::env::var("ANKAI_SPIKE_DEBUG").is_ok_and(|v| v == "1");

    if spike_debug {
        // Legacy/debug path only — see module doc comment above and
        // client/ui/spike-glass-blur.slint.
        let spike = SpikeGlassBlurWindow::new()?;
        return spike.run();
    }

    let db = Rc::new(open_local_db());

    // First-run local identity: generates and persists an account/device
    // id plus a real device signature keypair the first time this device's
    // DB is opened, or reloads the same one on every run after that. See
    // ankai_core::identity's module doc comment for what this is (and
    // isn't) yet — no server-issued account, no account-root identity key.
    let mls_provider = Rc::new(
        ankai_core::mls_provider::AnkaiMlsProvider::load(&db).expect("failed to load MLS storage"),
    );
    let device = ankai_core::identity::load_or_create_device(&db, &mls_provider)
        .expect("failed to load or create local device identity");
    mls_provider
        .flush(&db)
        .expect("failed to persist MLS storage");

    MESSAGING_HANDLES.with(|handles| {
        *handles.borrow_mut() = Some(MessagingHandles {
            db: db.clone(),
            mls_provider: mls_provider.clone(),
        });
    });

    let app = AppWindow::new()?;
    app.set_account_id(device.account.0.clone().into());
    app.set_device_id(device.id.0.clone().into());

    let saved_display_name = db
        .get_setting("display_name")
        .expect("failed to read display_name setting")
        .unwrap_or_default();
    app.set_display_name(saved_display_name.into());

    let db_for_save = db.clone();
    app.on_save_display_name(move |name| {
        if let Err(err) = db_for_save.set_setting("display_name", &name) {
            eprintln!("ankai-client: failed to save display name: {err}");
        }
    });

    let communities = ankai_core::communities::list(&db).expect("failed to list communities");
    let community_names: Vec<slint::SharedString> =
        communities.into_iter().map(|c| c.name.into()).collect();
    let community_model = std::rc::Rc::new(slint::VecModel::from(community_names));
    app.set_community_names(slint::ModelRc::from(community_model.clone()));

    let db_for_communities = db.clone();
    let app_weak = app.as_weak();
    app.on_create_community(move |name| {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        match ankai_core::communities::create(&db_for_communities, name) {
            Ok(community) => {
                community_model.push(community.name.into());
                if let Some(app) = app_weak.upgrade() {
                    app.set_new_community_name("".into());
                }
            }
            Err(err) => eprintln!("ankai-client: failed to create community: {err}"),
        }
    });

    // P2P messaging (ankai_core::p2p / ankai_core::messaging): real MLS
    // (RFC 9420, via OpenMLS) end-to-end encryption over a 2-member group
    // per peer, with plaintext persisted to the encrypted local DB after
    // decryption. See ankai_core::messaging's module doc comment for what
    // this still deliberately doesn't do (no discovery service, no
    // multi-conversation UI). Slint's event loop owns the main thread and
    // is not async, so a dedicated tokio runtime drives the networking; the
    // two talk to each other via `slint::Weak` (documented `Send`) and
    // `slint::invoke_from_event_loop`. All MLS/DB work stays on the UI
    // thread (see `MessagingHandles`'s doc comment) — the tokio side only
    // ever moves raw, already-encrypted bytes.
    let p2p_runtime = tokio::runtime::Runtime::new().expect("failed to start P2P runtime");
    let p2p_node = std::sync::Arc::new(
        p2p_runtime
            .block_on(ankai_core::p2p::P2pNode::bind())
            .expect("failed to bind local P2P endpoint"),
    );

    // This device's standing invite: its dialable address plus a freshly
    // built KeyPackage (MLS's prekey equivalent), so a peer who pastes it
    // can start (or receive) a real MLS group with this device. See
    // ankai_core::messaging's "Group setup without a directory" doc section
    // for why this exists instead of a real directory/discovery service.
    // Regenerated every startup rather than tracked/reused/pruned — a real
    // client would maintain a small pool of unused KeyPackages and rotate
    // them; Phase 1 just leaves old, never-consumed ones sitting harmlessly
    // in MLS storage (same "revisit before the audit gate" bucket as
    // `mls_provider`'s whole-blob persistence tradeoff).
    let own_key_package = ankai_core::identity::create_key_package(&device, &mls_provider)
        .expect("failed to build this device's MLS key package")
        .key_package
        .expect("create_key_package always returns Some");
    mls_provider
        .flush(&db)
        .expect("failed to persist MLS storage");
    let own_invite = ankai_core::messaging::PeerInvite {
        addr: p2p_node.addr(),
        key_package: own_key_package,
    };
    let own_invite_text = ankai_core::messaging::format_peer_invite(&own_invite)
        .expect("failed to encode own P2P/MLS invite");
    app.set_own_peer_address(own_invite_text.into());

    // Load this device's entire persisted message history (oldest first)
    // into the same most-recent-first model shape the live send/receive
    // paths already push into — same idea as Settings loading
    // `display_name` and Communities loading its list at startup.
    let history =
        ankai_core::messaging::list_messages(&db).expect("failed to load message history");
    let message_log_model =
        std::rc::Rc::new(slint::VecModel::from(Vec::<slint::SharedString>::new()));
    for stored in history {
        let sender = match stored.direction {
            ankai_core::messaging::Direction::Sent => "you".to_string(),
            ankai_core::messaging::Direction::Received => {
                short_peer_id(&stored.peer_id).to_string()
            }
        };
        message_log_model.insert(0, format!("{sender}: {}", stored.content).into());
    }
    app.set_message_log(slint::ModelRc::from(message_log_model));

    // Background receive loop, spawned for the lifetime of the app. Each
    // incoming connection's raw bytes and sender id are handed to the UI
    // thread via invoke_from_event_loop, where MessagingHandles::decrypt
    // does the actual MLS decrypt + persist — see this file's
    // MessagingHandles doc comment for why that split exists.
    let node_for_accept = p2p_node.clone();
    let app_weak_for_accept = app.as_weak();
    p2p_runtime.spawn(async move {
        let result =
            ankai_core::messaging::receive_messages(&node_for_accept, move |from, bytes| {
                let app_weak = app_weak_for_accept.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(app) = app_weak.upgrade() else {
                        return;
                    };
                    let handles = MESSAGING_HANDLES.with(|h| h.borrow().clone());
                    let Some(handles) = handles else {
                        eprintln!("ankai-client: messaging handles not initialized yet");
                        return;
                    };

                    let result = ankai_core::messaging::decrypt_incoming(
                        &handles.db,
                        &handles.mls_provider,
                        from,
                        bytes,
                    );
                    if let Err(err) = handles.mls_provider.flush(&handles.db) {
                        eprintln!("ankai-client: failed to persist MLS state: {err}");
                    }

                    match result {
                        Ok(Some(msg)) => {
                            let peer_id = ankai_core::messaging::peer_id_for(msg.from);
                            let line = format!("{}: {}", short_peer_id(&peer_id), msg.text);
                            if let Some(model) = app
                                .get_message_log()
                                .as_any()
                                .downcast_ref::<slint::VecModel<slint::SharedString>>()
                            {
                                model.insert(0, line.into());
                            }
                        }
                        // A Welcome establishing a new group — nothing
                        // user-visible yet, this device just joined.
                        Ok(None) => {}
                        Err(err) => {
                            eprintln!("ankai-client: failed to process incoming message: {err}");
                        }
                    }
                });
            })
            .await;
        if let Err(err) = result {
            eprintln!("ankai-client: message receive loop ended: {err}");
        }
    });

    let node_for_send = p2p_node.clone();
    let db_for_send = db.clone();
    let mls_provider_for_send = mls_provider.clone();
    let device_for_send = device.clone();
    let app_weak_for_send = app.as_weak();
    let p2p_handle = p2p_runtime.handle().clone();
    app.on_send_message(move |peer_invite_text, message_text| {
        let message_text = message_text.trim().to_string();
        if message_text.is_empty() {
            return;
        }

        let invite = match ankai_core::messaging::parse_peer_invite(&peer_invite_text) {
            Ok(invite) => invite,
            Err(err) => {
                if let Some(app) = app_weak_for_send.upgrade() {
                    app.set_send_status(format!("Couldn't parse peer invite: {err}").into());
                }
                return;
            }
        };

        // Encryption (real MLS: group setup on first contact, then
        // `create_message`) and persistence both happen synchronously here
        // on the UI thread, where `db`/`mls_provider` already live — only
        // the resulting already-encrypted bytes cross to the tokio runtime
        // for the actual network send.
        let payloads = match ankai_core::messaging::encrypt_and_log_outgoing(
            &db_for_send,
            &mls_provider_for_send,
            &device_for_send,
            &invite,
            &message_text,
        ) {
            Ok(payloads) => payloads,
            Err(err) => {
                if let Some(app) = app_weak_for_send.upgrade() {
                    app.set_send_status(format!("Failed to encrypt message: {err}").into());
                }
                return;
            }
        };
        if let Err(err) = mls_provider_for_send.flush(&db_for_send) {
            eprintln!("ankai-client: failed to persist MLS state: {err}");
        }

        let node = node_for_send.clone();
        let addr = invite.addr.clone();
        let app_weak = app_weak_for_send.clone();
        p2p_handle.spawn(async move {
            let mut result = Ok(());
            for payload in &payloads {
                result = ankai_core::messaging::send_message(&node, addr.clone(), payload).await;
                if result.is_err() {
                    break;
                }
            }
            let _ = slint::invoke_from_event_loop(move || {
                let Some(app) = app_weak.upgrade() else {
                    return;
                };
                match result {
                    Ok(()) => {
                        app.set_send_status("".into());
                        app.set_message_input("".into());
                        if let Some(model) = app
                            .get_message_log()
                            .as_any()
                            .downcast_ref::<slint::VecModel<slint::SharedString>>()
                        {
                            model.insert(0, format!("you: {message_text}").into());
                        }
                    }
                    Err(err) => {
                        app.set_send_status(format!("Failed to send: {err}").into());
                    }
                }
            });
        });
    });

    app.run()
}
