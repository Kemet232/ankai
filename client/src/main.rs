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

use slint::Model;

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

    let db = std::rc::Rc::new(open_local_db());

    // First-run local identity: generates and persists an account/device
    // id plus a real device signature keypair the first time this device's
    // DB is opened, or reloads the same one on every run after that. See
    // ankai_core::identity's module doc comment for what this is (and
    // isn't) yet — no server-issued account, no account-root identity key.
    let mls_provider =
        ankai_core::mls_provider::AnkaiMlsProvider::load(&db).expect("failed to load MLS storage");
    let device = ankai_core::identity::load_or_create_device(&db, &mls_provider)
        .expect("failed to load or create local device identity");
    mls_provider
        .flush(&db)
        .expect("failed to persist MLS storage");

    let app = AppWindow::new()?;
    app.set_account_id(device.account.0.into());
    app.set_device_id(device.id.0.into());

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

    let hangouts = ankai_core::hangouts::list(&db).expect("failed to list hangouts");
    let hangout_names: Vec<slint::SharedString> =
        hangouts.into_iter().map(|h| h.name.into()).collect();
    let hangout_model = std::rc::Rc::new(slint::VecModel::from(hangout_names));
    app.set_hangout_names(slint::ModelRc::from(hangout_model.clone()));

    let db_for_hangouts = db.clone();
    let app_weak_for_hangouts = app.as_weak();
    app.on_create_hangout(move |name| {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        match ankai_core::hangouts::create(&db_for_hangouts, name) {
            Ok(hangout) => {
                hangout_model.push(hangout.name.into());
                if let Some(app) = app_weak_for_hangouts.upgrade() {
                    app.set_new_hangout_name("".into());
                }
            }
            Err(err) => eprintln!("ankai-client: failed to create hangout: {err}"),
        }
    });

    // P2P messaging (ankai_core::p2p / ankai_core::messaging) — see those
    // modules' doc comments for what this stub deliberately does not do yet
    // (no discovery, no E2EE beyond QUIC transport encryption, no
    // persistence — messages live only in message_log's in-memory model for
    // this run). Slint's event loop owns the main thread and is not async,
    // so a dedicated tokio runtime drives the networking; the two talk to
    // each other via `slint::Weak` (documented `Send`) and
    // `slint::invoke_from_event_loop`, never by sharing UI model types
    // across threads.
    let p2p_runtime = tokio::runtime::Runtime::new().expect("failed to start P2P runtime");
    let p2p_node = std::sync::Arc::new(
        p2p_runtime
            .block_on(ankai_core::p2p::P2pNode::bind())
            .expect("failed to bind local P2P endpoint"),
    );

    let own_peer_address = ankai_core::messaging::format_peer_address(&p2p_node.addr())
        .expect("failed to encode own P2P address");
    app.set_own_peer_address(own_peer_address.into());

    let message_log_model =
        std::rc::Rc::new(slint::VecModel::from(Vec::<slint::SharedString>::new()));
    app.set_message_log(slint::ModelRc::from(message_log_model));

    // Background receive loop, spawned for the lifetime of the app. Each
    // incoming message is handed to the UI thread via invoke_from_event_loop
    // rather than touched directly here, since Slint's model types aren't
    // Send and this closure runs on a tokio worker thread.
    let node_for_accept = p2p_node.clone();
    let app_weak_for_accept = app.as_weak();
    p2p_runtime.spawn(async move {
        let result = ankai_core::messaging::receive_messages(&node_for_accept, move |msg| {
            let app_weak = app_weak_for_accept.clone();
            let line = format!("{}: {}", msg.from.fmt_short(), msg.text);
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = app_weak.upgrade() {
                    if let Some(model) = app
                        .get_message_log()
                        .as_any()
                        .downcast_ref::<slint::VecModel<slint::SharedString>>()
                    {
                        model.insert(0, line.into());
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
    let app_weak_for_send = app.as_weak();
    let p2p_handle = p2p_runtime.handle().clone();
    app.on_send_message(move |peer_address_text, message_text| {
        let message_text = message_text.trim().to_string();
        if message_text.is_empty() {
            return;
        }

        let addr = match ankai_core::messaging::parse_peer_address(&peer_address_text) {
            Ok(addr) => addr,
            Err(err) => {
                if let Some(app) = app_weak_for_send.upgrade() {
                    app.set_send_status(format!("Couldn't parse peer address: {err}").into());
                }
                return;
            }
        };

        let node = node_for_send.clone();
        let app_weak = app_weak_for_send.clone();
        p2p_handle.spawn(async move {
            let result = ankai_core::messaging::send_message(&node, addr, &message_text).await;
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
