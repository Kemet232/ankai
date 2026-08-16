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

// Separate, additive generated-code module for the FloatingPanel demo (see
// client/ui/floating-panel-demo.slint, client/build.rs) — included via an
// explicit path rather than a second `slint::include_modules!()` call,
// since that macro can only ever point at one file per crate (see
// build.rs's comment). Debug-only; not part of the real app flow.
mod floating_panel_demo {
    #![allow(clippy::all)]
    include!(concat!(env!("OUT_DIR"), "/floating_panel_demo.rs"));
}

mod directory;
mod images;

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

/// Derives the single uppercase letter shown in the Profile pane's avatar
/// placeholder circle (see `ui/app.slint`'s Profile header) from the saved
/// display name. Slint 1.17 has no string-slicing/char-at builtin (only
/// whole-string operations like `to-uppercase`/`is-empty`), so this one
/// piece of presentation logic — "first letter of the name, or a fallback
/// when there's no name yet" — has to live here instead of in the .slint
/// file with everything else. Falls back to "A" (for ANKAI) when the name
/// is empty, rather than showing a blank circle.
fn initial_letter(name: &str) -> String {
    name.trim()
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "A".to_string())
}

/// Recomputes the Profile pane's Top 8 UI state (`featured-communities`/
/// `unfeatured-communities`) fresh from `core::top8`/`core::communities` and
/// pushes it into the running `AppWindow`. Called once at startup and again
/// after every mutating Top 8 callback below — simplest way to keep the two
/// lists (featured vs. everything else) consistent with each other and with
/// the real persisted state, without hand-rolling incremental model diffs
/// for what is, in practice, an infrequent user action.
fn refresh_top8(app: &AppWindow, db: &ankai_core::db::Db) {
    let all_communities = ankai_core::communities::list(db).unwrap_or_else(|err| {
        eprintln!("ankai-client: failed to list communities for top8 refresh: {err}");
        Vec::new()
    });
    let featured = ankai_core::top8::get_top8(db).unwrap_or_else(|err| {
        eprintln!("ankai-client: failed to load top8: {err}");
        Vec::new()
    });

    let featured_ids: std::collections::HashSet<String> =
        featured.iter().map(|c| c.id.clone()).collect();

    let featured_refs: Vec<CommunityRef> = featured
        .into_iter()
        .map(|c| CommunityRef {
            id: c.id.into(),
            name: c.name.into(),
        })
        .collect();
    let unfeatured_refs: Vec<CommunityRef> = all_communities
        .into_iter()
        .filter(|c| !featured_ids.contains(c.id.as_str()))
        .map(|c| CommunityRef {
            id: c.id.into(),
            name: c.name.into(),
        })
        .collect();

    app.set_featured_communities(slint::ModelRc::from(Rc::new(slint::VecModel::from(
        featured_refs,
    ))));
    app.set_unfeatured_communities(slint::ModelRc::from(Rc::new(slint::VecModel::from(
        unfeatured_refs,
    ))));

    refresh_stats(app, db);
}

/// Picks the title to show for a watchlist entry: prefers the cached English
/// title, falls back to romaji, and (only if AniList had neither on file —
/// rare but real, see `AnimeSummary`'s doc comment) a last-resort id-based
/// label rather than an empty string.
fn watchlist_title(entry: &ankai_core::anime::WatchlistEntry) -> String {
    entry
        .title_english
        .clone()
        .or_else(|| entry.title_romaji.clone())
        .unwrap_or_else(|| format!("AniList #{}", entry.anilist_id))
}

/// Formats a watchlist entry's progress line. `episode_count` is `None` for
/// currently-airing shows AniList hasn't settled a final count for yet (see
/// `ankai_core::anime`'s doc comment) — in that case there's no denominator
/// to show, so this reads "N episodes watched" instead of "N / ? episodes".
fn watchlist_progress(entry: &ankai_core::anime::WatchlistEntry) -> String {
    match entry.episode_count {
        Some(total) => format!("{} / {total} episodes", entry.watched_episodes),
        None => format!("{} episodes watched", entry.watched_episodes),
    }
}

/// Recomputes the My Page "Currently Watching" module from
/// `ankai_core::anime`'s real local watchlist, filtered to
/// `WatchStatus::Watching` only (see that module's doc comment — this is
/// deliberately not a full watchlist browser). Called once at startup and
/// again after any watchlist mutation, same "recompute fresh from core"
/// shape as `refresh_top8`.
///
/// Each entry's cached `cover_image_url` (real AniList CDN URL) starts out
/// unfetched (`cover: default`, `has_cover: false`) and is filled in
/// asynchronously afterward via `client::images::load_cover_image`, run on
/// `handle` — same "text now, image once it loads" shape
/// `spawn_anime_refresh` already uses for the Home dashboard's cards.
fn refresh_watchlist(app: &AppWindow, db: &ankai_core::db::Db, handle: &tokio::runtime::Handle) {
    let watching =
        ankai_core::anime::list_watchlist(db, Some(ankai_core::anime::WatchStatus::Watching))
            .unwrap_or_else(|err| {
                eprintln!("ankai-client: failed to list watching anime: {err}");
                Vec::new()
            });

    let cards: Vec<WatchlistCard> = watching
        .iter()
        .map(|entry| WatchlistCard {
            anilist_id: entry.anilist_id as i32,
            title: watchlist_title(entry).into(),
            progress: watchlist_progress(entry).into(),
            cover: slint::Image::default(),
            has_cover: false,
        })
        .collect();

    app.set_watching_anime(slint::ModelRc::from(Rc::new(slint::VecModel::from(cards))));

    for entry in &watching {
        let Some(url) = entry.cover_image_url.clone() else {
            continue;
        };
        let anilist_id = entry.anilist_id as i32;
        let app_weak = app.as_weak();
        images::load_cover_image(url, handle.clone(), app_weak, move |app, image| {
            set_watchlist_cover(app.get_watching_anime(), anilist_id, image);
        });
    }

    refresh_stats(app, db);
}

/// Updates a single row of the "Currently Watching" model in place once its
/// real cover image finishes loading — matched by `anilist_id` rather than
/// index, since the model can in principle be reloaded (a fresh
/// `refresh_watchlist`) while an earlier load's fetch is still in flight.
/// A no-op if the row is no longer present (same "just don't apply it"
/// tolerance `spawn_friend_presence_checks` already has for its own
/// in-flight-vs-reloaded race).
fn set_watchlist_cover(model: slint::ModelRc<WatchlistCard>, anilist_id: i32, image: slint::Image) {
    let Some(vec_model) = model
        .as_any()
        .downcast_ref::<slint::VecModel<WatchlistCard>>()
    else {
        return;
    };
    for i in 0..vec_model.row_count() {
        if let Some(mut row) = vec_model.row_data(i) {
            if row.anilist_id == anilist_id {
                row.cover = image;
                row.has_cover = true;
                vec_model.set_row_data(i, row);
                break;
            }
        }
    }
}

/// Recomputes the My Page Friends section (accepted friends + pending
/// *incoming* requests) from `ankai_core::friends`'s real tables. Called
/// once at startup, after accepting/declining a request, and whenever a
/// friends-protocol message arrives over P2P (see the receive loop below).
/// Presence (`FriendCard::status`) always resets to `""` ("not checked")
/// here — the list just changed, so any previously fetched presence no
/// longer necessarily reflects who's actually in it; `on_refresh_friends_presence`
/// re-checks it live, on demand.
fn refresh_friends(app: &AppWindow, db: &ankai_core::db::Db) {
    let friends = ankai_core::friends::list_friends(db).unwrap_or_else(|err| {
        eprintln!("ankai-client: failed to list friends: {err}");
        Vec::new()
    });
    let pending = ankai_core::friends::list_pending_requests(db).unwrap_or_else(|err| {
        eprintln!("ankai-client: failed to list pending friend requests: {err}");
        Vec::new()
    });

    let friend_cards: Vec<FriendCard> = friends
        .into_iter()
        .map(|f| FriendCard {
            device_id: f.device_id.0.into(),
            account_id: f.account_id.0.into(),
            status: "".into(),
        })
        .collect();
    let pending_cards: Vec<FriendCard> = pending
        .into_iter()
        .map(|f| FriendCard {
            device_id: f.device_id.0.into(),
            account_id: f.account_id.0.into(),
            status: "".into(),
        })
        .collect();

    app.set_friends_list(slint::ModelRc::from(Rc::new(slint::VecModel::from(
        friend_cards,
    ))));
    app.set_pending_friend_requests(slint::ModelRc::from(Rc::new(slint::VecModel::from(
        pending_cards,
    ))));

    refresh_stats(app, db);
}

/// Recomputes the Stats tab's real counts. Every number here comes from the
/// same `core::` list calls already used elsewhere in this file — nothing
/// is estimated. Called from `refresh_top8`/`refresh_watchlist`/
/// `refresh_friends` so it always reflects the latest state without every
/// mutating callback needing to remember to call it directly.
fn refresh_stats(app: &AppWindow, db: &ankai_core::db::Db) {
    let communities_count = ankai_core::communities::list(db)
        .map(|v| v.len())
        .unwrap_or(0);
    let watching_count =
        ankai_core::anime::list_watchlist(db, Some(ankai_core::anime::WatchStatus::Watching))
            .map(|v| v.len())
            .unwrap_or(0);
    let completed_count =
        ankai_core::anime::list_watchlist(db, Some(ankai_core::anime::WatchStatus::Completed))
            .map(|v| v.len())
            .unwrap_or(0);
    let planned_count =
        ankai_core::anime::list_watchlist(db, Some(ankai_core::anime::WatchStatus::Planned))
            .map(|v| v.len())
            .unwrap_or(0);
    let dropped_count =
        ankai_core::anime::list_watchlist(db, Some(ankai_core::anime::WatchStatus::Dropped))
            .map(|v| v.len())
            .unwrap_or(0);
    let friends_count = ankai_core::friends::list_friends(db)
        .map(|v| v.len())
        .unwrap_or(0);

    app.set_stat_communities_count(communities_count as i32);
    app.set_stat_watching_count(watching_count as i32);
    app.set_stat_completed_count(completed_count as i32);
    app.set_stat_planned_count(planned_count as i32);
    app.set_stat_dropped_count(dropped_count as i32);
    app.set_stat_friends_count(friends_count as i32);
}

/// Truncates a `messaging::peer_id_for`-style hex peer id to a short,
/// display-friendly prefix — the same idea as iroh's own `fmt_short`, for
/// the string peer ids this module works with instead of raw `EndpointId`s.
fn short_peer_id(peer_id: &str) -> &str {
    &peer_id[..peer_id.len().min(10)]
}

// ---------------------------------------------------------------------
// Home dashboard wiring (see `ui/app.slint`'s AnimeRef/RecentPostRef/
// FriendRef doc comments for what each field is and isn't). Every
// function here either reads real local state (`refresh_recent_posts`/
// `refresh_friends`, plain `core::forum_posts`/`core::friends` DB reads,
// same synchronous shape as `refresh_top8` above) or kicks off a real
// async network call on the shared tokio runtime and hands the result
// back to the UI thread via `slint::invoke_from_event_loop` — the same
// pattern already established by the P2P messaging section further down
// this file. Nothing here blocks the UI thread on a network call.
// ---------------------------------------------------------------------

/// Converts one AniList `AnimeSummary` into the Home dashboard's compact
/// title/score display shape. English title is preferred over romaji when
/// both exist (falls back to romaji, then a literal placeholder if AniList
/// had neither) — resolved here rather than in `.slint`, which has no
/// such string-fallback expression.
fn anime_to_ref(anime: ankai_core::anime::AnimeSummary) -> AnimeRef {
    let title = anime
        .title_english
        .or(anime.title_romaji)
        .unwrap_or_else(|| "Untitled".to_string());
    let score = match anime.average_score {
        Some(score) => format!("Score: {score}"),
        None => "Not yet rated".to_string(),
    };
    AnimeRef {
        id: anime.id as i32,
        title: title.into(),
        score: score.into(),
        cover: slint::Image::default(),
        has_cover: false,
    }
}

/// Updates a single row of a Home dashboard anime model (trending or
/// popular — both share `AnimeRef`'s shape) in place once its real cover
/// image finishes loading. Matched by AniList id rather than index, same
/// "reload can race an in-flight fetch, tolerate it" shape as
/// `set_watchlist_cover`/`spawn_friend_presence_checks`.
fn set_anime_cover(model: slint::ModelRc<AnimeRef>, id: i32, image: slint::Image) {
    let Some(vec_model) = model.as_any().downcast_ref::<slint::VecModel<AnimeRef>>() else {
        return;
    };
    for i in 0..vec_model.row_count() {
        if let Some(mut row) = vec_model.row_data(i) {
            if row.id == id {
                row.cover = image;
                row.has_cover = true;
                vec_model.set_row_data(i, row);
                break;
            }
        }
    }
}

/// Fetches AniList's real "trending" and "popular" rankings
/// (`core::anime::trending_anime`/`popular_anime`) and pushes them into the
/// Home dashboard's models once each completes. Spawned on `handle` (the
/// same tokio runtime the P2P/messaging side already runs on) rather than
/// called synchronously: AniList is a real third-party dependency with a
/// real rate limit and occasional real outages (see `core::anime`'s module
/// doc comment), so a slow or failed request must not stall the window at
/// startup or freeze the UI thread on a refresh click.
fn spawn_anime_refresh(handle: tokio::runtime::Handle, app_weak: slint::Weak<AppWindow>) {
    let app_weak_trending = app_weak.clone();
    let handle_trending = handle.clone();
    handle.spawn(async move {
        let result = ankai_core::anime::trending_anime(10).await;
        let _ = slint::invoke_from_event_loop(move || {
            let Some(app) = app_weak_trending.upgrade() else {
                return;
            };
            match result {
                Ok(list) => {
                    app.set_anime_status("".into());
                    // Cover art (see `AnimeRef`'s doc comment) loads
                    // asynchronously after the text-only cards are already
                    // on screen: collect (id, url) pairs before `list` is
                    // consumed by `anime_to_ref` below, then kick off one
                    // real fetch per entry that actually has a cover URL.
                    let covers: Vec<(i32, String)> = list
                        .iter()
                        .filter_map(|a| a.cover_image_url.clone().map(|url| (a.id as i32, url)))
                        .collect();
                    let refs: Vec<AnimeRef> = list.into_iter().map(anime_to_ref).collect();
                    app.set_trending_anime(slint::ModelRc::from(Rc::new(slint::VecModel::from(
                        refs,
                    ))));
                    for (id, url) in covers {
                        let app_weak = app_weak_trending.clone();
                        images::load_cover_image(
                            url,
                            handle_trending.clone(),
                            app_weak,
                            move |app, image| {
                                set_anime_cover(app.get_trending_anime(), id, image);
                            },
                        );
                    }
                }
                Err(err) => {
                    eprintln!("ankai-client: failed to load trending anime: {err}");
                    app.set_anime_status(format!("Couldn't load trending anime: {err}").into());
                }
            }
        });
    });

    let app_weak_popular = app_weak.clone();
    let handle_popular = handle.clone();
    handle.spawn(async move {
        let result = ankai_core::anime::popular_anime(10).await;
        let _ = slint::invoke_from_event_loop(move || {
            let Some(app) = app_weak_popular.upgrade() else {
                return;
            };
            match result {
                Ok(list) => {
                    let covers: Vec<(i32, String)> = list
                        .iter()
                        .filter_map(|a| a.cover_image_url.clone().map(|url| (a.id as i32, url)))
                        .collect();
                    let refs: Vec<AnimeRef> = list.into_iter().map(anime_to_ref).collect();
                    app.set_popular_anime(slint::ModelRc::from(Rc::new(slint::VecModel::from(
                        refs,
                    ))));
                    for (id, url) in covers {
                        let app_weak = app_weak_popular.clone();
                        images::load_cover_image(
                            url,
                            handle_popular.clone(),
                            app_weak,
                            move |app, image| {
                                set_anime_cover(app.get_popular_anime(), id, image);
                            },
                        );
                    }
                }
                Err(err) => {
                    eprintln!("ankai-client: failed to load popular anime: {err}");
                    app.set_anime_status(format!("Couldn't load popular anime: {err}").into());
                }
            }
        });
    });
}

/// Reloads the Home dashboard's "Hot Discussions" model from
/// `core::forum_posts::list_recent_across_communities` — a plain, synchronous
/// local-DB read (no network involved, unlike the anime functions above),
/// same shape as `refresh_top8`. Capped at 12 entries: a dashboard widget,
/// not infinite scroll.
fn refresh_recent_posts(app: &AppWindow, db: &ankai_core::db::Db) {
    let recent =
        ankai_core::forum_posts::list_recent_across_communities(db, 12).unwrap_or_else(|err| {
            eprintln!("ankai-client: failed to load recent posts for Home: {err}");
            Vec::new()
        });
    let refs: Vec<RecentPostRef> = recent
        .into_iter()
        .map(|p| RecentPostRef {
            community_name: p.community_name.into(),
            content: p.content.into(),
        })
        .collect();
    app.set_recent_posts(slint::ModelRc::from(Rc::new(slint::VecModel::from(refs))));
}

/// Reloads the Home dashboard's "Friend Activity" model from
/// `core::friends::list_friends` — a plain, synchronous local-DB read.
/// Every row starts `online: false` regardless of any previous check;
/// callers that want a live presence result should follow up with
/// [`spawn_friend_presence_checks`] using the returned `Vec<Friend>` (kept
/// separate so the caller doesn't need a second DB read just to get the
/// real `Friend` values `check_presence` needs).
fn refresh_home_friends(
    app: &AppWindow,
    db: &ankai_core::db::Db,
) -> Vec<ankai_core::friends::Friend> {
    let friends = ankai_core::friends::list_friends(db).unwrap_or_else(|err| {
        eprintln!("ankai-client: failed to load friends for Home: {err}");
        Vec::new()
    });
    let refs: Vec<FriendRef> = friends
        .iter()
        .map(|f| FriendRef {
            device_id: f.device_id.0.clone().into(),
            account_id: f.account_id.0.clone().into(),
            online: false,
        })
        .collect();
    app.set_friends(slint::ModelRc::from(Rc::new(slint::VecModel::from(refs))));
    friends
}

/// Kicks off a real `core::friends::check_presence` call for each of
/// `friends`, one independent async task per friend (each with its own
/// ~3s timeout, per that function's doc comment) so one slow/unreachable
/// friend can't delay the others or block the UI thread. Each task updates
/// only its own row in the `friends` model in place once its real result
/// comes back — matching-by-device-id rather than by index, since the
/// model could in principle be reloaded (via refresh-home) while checks
/// from a previous load are still in flight.
fn spawn_friend_presence_checks(
    friends: Vec<ankai_core::friends::Friend>,
    node: std::sync::Arc<ankai_core::p2p::P2pNode>,
    handle: tokio::runtime::Handle,
    app_weak: slint::Weak<AppWindow>,
) {
    for friend in friends {
        let node = node.clone();
        let app_weak = app_weak.clone();
        let device_id = friend.device_id.0.clone();
        handle.spawn(async move {
            let online = ankai_core::friends::check_presence(&node, &friend).await;
            let _ = slint::invoke_from_event_loop(move || {
                let Some(app) = app_weak.upgrade() else {
                    return;
                };
                let friends_model = app.get_friends();
                let Some(model) = friends_model
                    .as_any()
                    .downcast_ref::<slint::VecModel<FriendRef>>()
                else {
                    return;
                };
                for i in 0..model.row_count() {
                    if let Some(mut row) = model.row_data(i) {
                        if row.device_id == device_id {
                            row.online = online;
                            model.set_row_data(i, row);
                            break;
                        }
                    }
                }
            });
        });
    }
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

    // Separate, additive debug-only path proving out the FloatingPanel
    // component (client/ui/floating-panel.slint) — an "MSN-Messenger-
    // style" floating/draggable window paradigm, not yet wired into the
    // real app shell. See client/ui/floating-panel-demo.slint and
    // PROGRESS.md session 13. Mirrors --spike/ANKAI_SPIKE_DEBUG above,
    // deliberately kept as its own separate flag rather than folded into
    // --spike.
    let floating_demo = std::env::args().any(|arg| arg == "--floating-demo")
        || std::env::var("ANKAI_FLOATING_DEMO").is_ok_and(|v| v == "1");
    if floating_demo {
        let demo = floating_panel_demo::FloatingPanelDemoWindow::new()?;
        return demo.run();
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
    app.set_display_name_initial(initial_letter(&saved_display_name).into());
    app.set_display_name(saved_display_name.into());

    let db_for_save = db.clone();
    let app_weak_for_name = app.as_weak();
    app.on_save_display_name(move |name| {
        if let Err(err) = db_for_save.set_setting("display_name", &name) {
            eprintln!("ankai-client: failed to save display name: {err}");
        }
        if let Some(app) = app_weak_for_name.upgrade() {
            app.set_display_name_initial(initial_letter(&name).into());
        }
    });

    let communities = ankai_core::communities::list(&db).expect("failed to list communities");
    let (community_names, community_ids): (Vec<slint::SharedString>, Vec<slint::SharedString>) =
        communities
            .into_iter()
            .map(|c| (c.name.into(), c.id.into()))
            .unzip();
    let community_name_model = std::rc::Rc::new(slint::VecModel::from(community_names));
    let community_id_model = std::rc::Rc::new(slint::VecModel::from(community_ids));
    app.set_community_names(slint::ModelRc::from(community_name_model.clone()));
    app.set_community_ids(slint::ModelRc::from(community_id_model.clone()));

    let db_for_communities = db.clone();
    let app_weak = app.as_weak();
    app.on_create_community(move |name| {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        match ankai_core::communities::create(&db_for_communities, name) {
            Ok(community) => {
                community_name_model.push(community.name.into());
                community_id_model.push(community.id.into());
                if let Some(app) = app_weak.upgrade() {
                    app.set_new_community_name("".into());
                    // A newly created community isn't featured yet, but it
                    // should immediately show up as an "add to Top 8"
                    // candidate on the Profile pane.
                    refresh_top8(&app, &db_for_communities);
                }
            }
            Err(err) => eprintln!("ankai-client: failed to create community: {err}"),
        }
    });

    // Forum posts within a community (ankai_core::forum_posts — see that
    // module's doc comment for scope: flat, append-only, local-only text,
    // no replies/editing/authorship/moderation). Selecting a community from
    // the list loads its posts (oldest first); posting appends both to the
    // DB and to the live model, same "create + append" shape as
    // create-community above.
    let community_posts_model =
        std::rc::Rc::new(slint::VecModel::from(Vec::<slint::SharedString>::new()));
    app.set_community_posts(slint::ModelRc::from(community_posts_model.clone()));

    let db_for_open_community = db.clone();
    let app_weak_for_open_community = app.as_weak();
    let community_posts_model_for_open = community_posts_model.clone();
    app.on_open_community(move |id, name| {
        let posts = match ankai_core::forum_posts::list_posts(&db_for_open_community, &id) {
            Ok(posts) => posts,
            Err(err) => {
                eprintln!("ankai-client: failed to list posts for community {id}: {err}");
                return;
            }
        };
        community_posts_model_for_open.set_vec(
            posts
                .into_iter()
                .map(|p| slint::SharedString::from(p.content))
                .collect::<Vec<_>>(),
        );
        if let Some(app) = app_weak_for_open_community.upgrade() {
            app.set_selected_community_id(id);
            app.set_selected_community_name(name);
            app.set_new_post_content("".into());
        }
    });

    let db_for_create_post = db.clone();
    let app_weak_for_create_post = app.as_weak();
    app.on_create_post(move |community_id, content| {
        let content = content.trim();
        if content.is_empty() {
            return;
        }
        match ankai_core::forum_posts::create_post(&db_for_create_post, &community_id, content) {
            Ok(post) => {
                community_posts_model.push(post.content.into());
                if let Some(app) = app_weak_for_create_post.upgrade() {
                    app.set_new_post_content("".into());
                }
            }
            Err(err) => {
                eprintln!("ankai-client: failed to create post in community {community_id}: {err}")
            }
        }
    });

    // Top 8 featured communities (core::top8): see that module's doc
    // comment and app.slint's featured-communities/unfeatured-communities
    // properties for full scope. Each callback here does the real core
    // mutation, then recomputes both lists from scratch via refresh_top8 —
    // same "real callback -> real core call -> real persisted state" shape
    // as Settings' display-name field and Communities' create button, just
    // with a full-state refresh instead of an incremental model push since
    // reordering/removal can touch more than one row at once.
    refresh_top8(&app, &db);

    let db_for_feature = db.clone();
    let app_weak_for_feature = app.as_weak();
    app.on_feature_community(move |community_id| {
        if let Err(err) = ankai_core::top8::add_to_top8(&db_for_feature, &community_id) {
            eprintln!("ankai-client: failed to feature community: {err}");
        }
        if let Some(app) = app_weak_for_feature.upgrade() {
            refresh_top8(&app, &db_for_feature);
        }
    });

    let db_for_unfeature = db.clone();
    let app_weak_for_unfeature = app.as_weak();
    app.on_unfeature_community(move |community_id| {
        if let Err(err) = ankai_core::top8::remove_from_top8(&db_for_unfeature, &community_id) {
            eprintln!("ankai-client: failed to unfeature community: {err}");
        }
        if let Some(app) = app_weak_for_unfeature.upgrade() {
            refresh_top8(&app, &db_for_unfeature);
        }
    });

    let db_for_move_up = db.clone();
    let app_weak_for_move_up = app.as_weak();
    app.on_move_featured_community_up(move |community_id| {
        if let Err(err) = ankai_core::top8::move_up(&db_for_move_up, &community_id) {
            eprintln!("ankai-client: failed to move featured community up: {err}");
        }
        if let Some(app) = app_weak_for_move_up.upgrade() {
            refresh_top8(&app, &db_for_move_up);
        }
    });

    let db_for_move_down = db.clone();
    let app_weak_for_move_down = app.as_weak();
    app.on_move_featured_community_down(move |community_id| {
        if let Err(err) = ankai_core::top8::move_down(&db_for_move_down, &community_id) {
            eprintln!("ankai-client: failed to move featured community down: {err}");
        }
        if let Some(app) = app_weak_for_move_down.upgrade() {
            refresh_top8(&app, &db_for_move_down);
        }
    });

    // "Currently Watching" (ankai_core::anime's real local watchlist,
    // filtered to WatchStatus::Watching — see refresh_watchlist's doc
    // comment). No mutation callbacks yet: this pane is read-only display
    // of whatever's already on the watchlist, since there's no "search
    // AniList and add" UI built in this pass — that's real, separate future
    // work (this module's own AniList search/trending/popular functions are
    // fully built and tested, just not wired into any UI screen yet).
    // Deferred until after the P2P/tokio runtime is set up below —
    // refresh_watchlist needs a `tokio::runtime::Handle` to kick off real
    // async cover-art fetches (see client::images), same runtime everything
    // else in this file already shares.
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

    // Now that a tokio runtime/handle exists, finish "Currently Watching"'s
    // setup (see the comment above this block) — real cover-art fetches
    // (client::images) get kicked off asynchronously on this same runtime.
    refresh_watchlist(&app, &db, p2p_runtime.handle());

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

    // Friends (ankai_core::friends): a real accepted-friends list plus real
    // pending *incoming* requests, sharing this same P2pNode with messaging
    // (see that module's "Wire dispatch" doc section). There is
    // deliberately no "send a friend request" UI wired here — sending would
    // need a way to pick a target device (the same look-up-by-Device-ID/
    // username machinery Messages already has), and this pass scopes that
    // out to keep the surface reviewable; accepting/declining an incoming
    // request (which core::friends fully supports) is what's wired.
    refresh_friends(&app, &db);

    let node_for_accept_friend = p2p_node.clone();
    let db_for_accept_friend = db.clone();
    let mls_provider_for_accept_friend = mls_provider.clone();
    let device_for_accept_friend = device.clone();
    let app_weak_for_accept_friend = app.as_weak();
    let p2p_handle_for_accept_friend = p2p_runtime.handle().clone();
    app.on_accept_friend_request(move |device_id_text| {
        let device_id = ankai_core::identity::DeviceId(device_id_text.to_string());
        // Run to completion on the UI thread via Handle::block_on rather
        // than tokio::spawn: accept_friend_request interleaves synchronous
        // Db writes with one async P2P send inside a single async fn (see
        // its doc comment), so the Db/MLS-provider `Rc`s it needs can't
        // safely cross into a spawned task (see this file's
        // MessagingHandles doc comment on why `Db` isn't `Send`/`Sync`
        // across threads). This does mean the UI blocks for the duration of
        // that one P2P send attempt (no timeout on it, unlike
        // check_presence's) — same accepted tradeoff as this file's
        // existing directory-publish `block_on` call at startup.
        let result =
            p2p_handle_for_accept_friend.block_on(ankai_core::friends::accept_friend_request(
                &node_for_accept_friend,
                &db_for_accept_friend,
                &device_for_accept_friend,
                &mls_provider_for_accept_friend,
                &device_id,
            ));
        match result {
            Ok(_outcome) => {
                if let Some(app) = app_weak_for_accept_friend.upgrade() {
                    refresh_friends(&app, &db_for_accept_friend);
                }
            }
            Err(err) => eprintln!("ankai-client: failed to accept friend request: {err}"),
        }
    });

    let db_for_decline_friend = db.clone();
    let app_weak_for_decline_friend = app.as_weak();
    app.on_decline_friend_request(move |device_id_text| {
        let device_id = ankai_core::identity::DeviceId(device_id_text.to_string());
        if let Err(err) =
            ankai_core::friends::decline_friend_request(&db_for_decline_friend, &device_id)
        {
            eprintln!("ankai-client: failed to decline friend request: {err}");
        }
        if let Some(app) = app_weak_for_decline_friend.upgrade() {
            refresh_friends(&app, &db_for_decline_friend);
        }
    });

    // Real, on-demand presence checks (ankai_core::friends::check_presence)
    // against every accepted friend. Unlike accept above, check_presence
    // takes no Db/MLS-provider reference (just `&P2pNode` and an owned
    // `Friend`), so it's safe to run as real spawned tasks that report back
    // via invoke_from_event_loop — same shape as messaging's send/receive
    // paths.
    let node_for_presence = p2p_node.clone();
    let db_for_presence = db.clone();
    let app_weak_for_presence = app.as_weak();
    let p2p_handle_for_presence = p2p_runtime.handle().clone();
    app.on_refresh_friends_presence(move || {
        let friends = match ankai_core::friends::list_friends(&db_for_presence) {
            Ok(friends) => friends,
            Err(err) => {
                eprintln!("ankai-client: failed to list friends for presence check: {err}");
                return;
            }
        };

        if let Some(app) = app_weak_for_presence.upgrade() {
            if let Some(model) = app
                .get_friends_list()
                .as_any()
                .downcast_ref::<slint::VecModel<FriendCard>>()
            {
                for i in 0..model.row_count() {
                    if let Some(mut row) = model.row_data(i) {
                        row.status = "checking...".into();
                        model.set_row_data(i, row);
                    }
                }
            }
        }

        for friend in friends {
            let node = node_for_presence.clone();
            let app_weak = app_weak_for_presence.clone();
            p2p_handle_for_presence.spawn(async move {
                let online = ankai_core::friends::check_presence(&node, &friend).await;
                let device_id = friend.device_id.0.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(app) = app_weak.upgrade() else {
                        return;
                    };
                    let friends_list = app.get_friends_list();
                    let Some(model) = friends_list
                        .as_any()
                        .downcast_ref::<slint::VecModel<FriendCard>>()
                    else {
                        return;
                    };
                    for i in 0..model.row_count() {
                        let Some(mut row) = model.row_data(i) else {
                            continue;
                        };
                        if row.device_id.as_str() == device_id.as_str() {
                            row.status = if online { "online" } else { "offline" }.into();
                            model.set_row_data(i, row);
                            break;
                        }
                    }
                });
            });
        }
    });

    // Experimental, opt-in ADR-0008 directory integration (Status:
    // Proposed — see docs/adr/0008-identity-discovery-service.md and
    // ankai_core::messaging's module doc comment). `directory_client` stays
    // `None`, and nothing here ever makes a network call to any directory
    // server, unless a human explicitly set ANKAI_DIRECTORY_URL. When set,
    // this device publishes its current KeyPackage + EndpointAddr (the same
    // pair `own_invite` above already bundles for manual pasting) so a peer
    // who knows this device's short DeviceId can look it up instead.
    // Publish failure (e.g. no server running at that URL) is logged and
    // otherwise ignored — the manual-paste flow keeps working regardless.
    let directory_url = directory::configured_directory_url();
    let directory_client = directory_url.as_ref().map(|url| {
        let signer = ankai_core::identity::device_signer(&device, &mls_provider)
            .expect("device signature key must exist to sign directory requests");
        std::sync::Arc::new(ankai_directory_server::HttpDirectoryClient::new(
            url.clone(),
            device.id.clone(),
            signer,
        ))
    });

    if let Some(client) = &directory_client {
        use ankai_core::directory::DirectoryService;
        let device_id = device.id.clone();
        let key_package_to_publish = own_invite.key_package.clone();
        let addr_to_publish = own_invite.addr.clone();
        let publish_result = p2p_runtime.block_on(async {
            client
                .publish_key_package(&device_id, key_package_to_publish)
                .await?;
            client
                .publish_endpoint_addr(&device_id, addr_to_publish)
                .await
        });
        match publish_result {
            Ok(()) => println!(
                "ankai-client: published this device's KeyPackage + EndpointAddr to the directory server at {} (experimental — ADR-0008 is still Proposed)",
                directory_url.unwrap_or_default()
            ),
            Err(err) => eprintln!(
                "ankai-client: failed to publish to the directory server (continuing without it, manual paste still works): {err}"
            ),
        }
    }
    app.set_directory_enabled(directory_client.is_some());

    // This device's own directory username, if it's ever successfully
    // claimed one — purely local display state (see app.slint's
    // claimed-username doc comment): the directory server remains the real
    // source of truth for who owns it, this is just "what did we last
    // successfully claim, so the Settings field isn't blank on the next
    // run." Saved under its own settings key, distinct from display_name,
    // since the two aren't the same thing (see client::directory's doc
    // comment on why usernames aren't folded into display_name).
    let saved_username = db
        .get_setting("directory_username")
        .expect("failed to read directory_username setting")
        .unwrap_or_default();
    app.set_claimed_username(saved_username.into());

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

                    // Friends-protocol messages carry a fixed magic prefix
                    // real MLS wire bytes never start with (see
                    // ankai_core::friends's "Wire dispatch" doc section), so
                    // trying friends::handle_incoming first and falling
                    // through to messaging::decrypt_incoming only when it
                    // reports "not ours" (Ok(None)) safely dispatches both
                    // message kinds over this one shared P2pNode/accept_loop
                    // with zero changes to messaging.rs itself.
                    match ankai_core::friends::handle_incoming(
                        &handles.db,
                        &handles.mls_provider,
                        &bytes,
                    ) {
                        Ok(Some(_event)) => {
                            // A new pending request or a confirmed accept —
                            // either way, the Friends section's data
                            // changed; refresh_friends recomputes both
                            // lists fresh rather than hand-patching one row.
                            refresh_friends(&app, &handles.db);
                            return;
                        }
                        Ok(None) => {
                            // Not a friends-protocol message; fall through
                            // to messaging below, unchanged.
                        }
                        Err(err) => {
                            eprintln!(
                                "ankai-client: failed to process incoming friend message: {err}"
                            );
                            return;
                        }
                    }

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

    // Experimental, opt-in directory-lookup-by-DeviceId callback — see the
    // `directory_client` setup above and client::directory's doc comment.
    // Only wired if directory_client is Some (i.e. ANKAI_DIRECTORY_URL was
    // set); the corresponding UI section is hidden otherwise (see
    // app.slint's `directory-enabled`), so this is unreachable when the
    // integration is off. On success this fills peer-address-input with
    // the same invite-blob text a manual paste would produce, so
    // on_send_message above (unchanged) is exactly what runs next — no
    // separate send path for directory-sourced peers. Usernames
    // (claim-username / lookup-peer-by-username, both below) are wired
    // alongside this, under the same directory_client.is_some() gate — see
    // ankai_directory_server::username's module doc comment for what a
    // username here is and isn't (device-scoped, not account-scoped).
    if let Some(client) = directory_client {
        let own_device_id = device.id.clone();

        {
            let client = client.clone();
            let app_weak_for_lookup = app.as_weak();
            let p2p_handle_for_lookup = p2p_runtime.handle().clone();
            app.on_lookup_peer_by_device_id(move |device_id_text| {
                let device_id_text = device_id_text.trim().to_string();
                if device_id_text.is_empty() {
                    return;
                }
                let peer = ankai_core::identity::DeviceId(device_id_text.clone());
                let client = client.clone();
                let app_weak = app_weak_for_lookup.clone();
                p2p_handle_for_lookup.spawn(async move {
                    let result = directory::lookup_peer_invite(&client, &peer).await;
                    let _ = slint::invoke_from_event_loop(move || {
                        let Some(app) = app_weak.upgrade() else {
                            return;
                        };
                        match result {
                            Ok(Some(invite)) => {
                                match ankai_core::messaging::format_peer_invite(&invite) {
                                    Ok(text) => {
                                        app.set_peer_address_input(text.into());
                                        app.set_lookup_status(
                                            format!("Found {device_id_text} in the directory — invite filled in below.")
                                                .into(),
                                        );
                                    }
                                    Err(err) => {
                                        app.set_lookup_status(
                                            format!("Found {device_id_text} but failed to encode its invite: {err}")
                                                .into(),
                                        );
                                    }
                                }
                            }
                            Ok(None) => {
                                app.set_lookup_status(
                                    format!("No published KeyPackage/EndpointAddr found for {device_id_text}.")
                                        .into(),
                                );
                            }
                            Err(err) => {
                                app.set_lookup_status(format!("Directory lookup failed: {err}").into());
                            }
                        }
                    });
                });
            });
        }

        // Look up a peer by username: resolves to a DeviceId via the
        // directory server, then goes through client::directory's
        // lookup_peer_invite_by_username, which itself calls the exact
        // same lookup_peer_invite as the Device-ID path above — not a
        // separate/forked lookup path.
        {
            let client = client.clone();
            let app_weak_for_lookup = app.as_weak();
            let p2p_handle_for_lookup = p2p_runtime.handle().clone();
            app.on_lookup_peer_by_username(move |username_text| {
                let username_text = username_text.trim().to_string();
                if username_text.is_empty() {
                    return;
                }
                let client = client.clone();
                let app_weak = app_weak_for_lookup.clone();
                p2p_handle_for_lookup.spawn(async move {
                    let result = directory::lookup_peer_invite_by_username(&client, &username_text).await;
                    let _ = slint::invoke_from_event_loop(move || {
                        let Some(app) = app_weak.upgrade() else {
                            return;
                        };
                        match result {
                            Ok(Some(invite)) => {
                                match ankai_core::messaging::format_peer_invite(&invite) {
                                    Ok(text) => {
                                        app.set_peer_address_input(text.into());
                                        app.set_lookup_status(
                                            format!("Found {username_text} in the directory — invite filled in below.")
                                                .into(),
                                        );
                                    }
                                    Err(err) => {
                                        app.set_lookup_status(
                                            format!("Found {username_text} but failed to encode its invite: {err}")
                                                .into(),
                                        );
                                    }
                                }
                            }
                            Ok(None) => {
                                app.set_lookup_status(
                                    format!("No device found for username \"{username_text}\".").into(),
                                );
                            }
                            Err(err) => {
                                app.set_lookup_status(format!("Username lookup failed: {err}").into());
                            }
                        }
                    });
                });
            });
        }

        // Claim (or update) this device's own username. The actual claim
        // is a real signed request to the directory server (see
        // client::directory::claim_own_username /
        // HttpDirectoryClient::claim_username); on success the claimed name
        // is also saved locally (via the MESSAGING_HANDLES thread-local,
        // the same UI-thread-only Db access pattern the receive loop uses —
        // Db isn't Send, so it can't be captured directly into this
        // tokio-spawned async block) purely so the Settings field shows it
        // again on the next run.
        {
            let client = client.clone();
            let device_id = own_device_id.clone();
            let app_weak_for_claim = app.as_weak();
            let p2p_handle_for_claim = p2p_runtime.handle().clone();
            app.on_claim_username(move |username_text| {
                let username_text = username_text.trim().to_string();
                if username_text.is_empty() {
                    return;
                }
                let client = client.clone();
                let device_id = device_id.clone();
                let app_weak = app_weak_for_claim.clone();
                p2p_handle_for_claim.spawn(async move {
                    let result = directory::claim_own_username(&client, &device_id, &username_text).await;
                    let _ = slint::invoke_from_event_loop(move || {
                        let Some(app) = app_weak.upgrade() else {
                            return;
                        };
                        match result {
                            Ok(()) => {
                                let handles = MESSAGING_HANDLES.with(|h| h.borrow().clone());
                                if let Some(handles) = handles {
                                    if let Err(err) =
                                        handles.db.set_setting("directory_username", &username_text)
                                    {
                                        eprintln!(
                                            "ankai-client: failed to save claimed username locally: {err}"
                                        );
                                    }
                                }
                                app.set_claimed_username(username_text.clone().into());
                                app.set_username_input("".into());
                                app.set_username_status(format!("Claimed \"{username_text}\".").into());
                            }
                            Err(err) => {
                                app.set_username_status(
                                    format!("Failed to claim \"{username_text}\": {err}").into(),
                                );
                            }
                        }
                    });
                });
            });
        }
    }

    // Home dashboard: recent posts and friends are plain synchronous local-
    // DB reads (like the rest of this app's startup loads above); trending/
    // popular anime and friend presence are real network calls, kicked off
    // asynchronously on the same tokio runtime the P2P/messaging side above
    // already uses (see spawn_anime_refresh/spawn_friend_presence_checks'
    // doc comments) so neither a slow AniList response nor an unreachable
    // friend can stall the window. refresh-home re-runs all four so a
    // failed AniList load (rate limit, an outage — see core::anime's module
    // doc comment) or newly-added friends/posts can be retried without
    // restarting the app.
    refresh_recent_posts(&app, &db);
    let initial_friends = refresh_home_friends(&app, &db);
    spawn_anime_refresh(p2p_runtime.handle().clone(), app.as_weak());
    spawn_friend_presence_checks(
        initial_friends,
        p2p_node.clone(),
        p2p_runtime.handle().clone(),
        app.as_weak(),
    );

    {
        let db_for_refresh = db.clone();
        let p2p_node_for_refresh = p2p_node.clone();
        let p2p_handle_for_refresh = p2p_runtime.handle().clone();
        let app_weak_for_refresh = app.as_weak();
        app.on_refresh_home(move || {
            let Some(app) = app_weak_for_refresh.upgrade() else {
                return;
            };
            refresh_recent_posts(&app, &db_for_refresh);
            let friends = refresh_home_friends(&app, &db_for_refresh);
            spawn_anime_refresh(p2p_handle_for_refresh.clone(), app.as_weak());
            spawn_friend_presence_checks(
                friends,
                p2p_node_for_refresh.clone(),
                p2p_handle_for_refresh.clone(),
                app.as_weak(),
            );
        });
    }

    // Home is the default landing pane (see ui/app.slint's selected-index
    // doc comment) — set explicitly here too, rather than relying solely on
    // that property's initial value, so this stays correct even if
    // nav-items/home-index are reordered again later.
    app.set_selected_index(app.get_home_index());

    app.run()
}
