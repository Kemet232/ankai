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
mod playback;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use slint::Model;

const CINEMETA_MANIFEST_URL: &str = "https://v3-cinemeta.strem.io/manifest.json";
const ANIME_KITSU_MANIFEST_URL: &str = "https://anime-kitsu.strem.fun/manifest.json";

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

struct SocialServices {
    mls_provider: Rc<ankai_core::mls_provider::AnkaiMlsProvider>,
    device: ankai_core::identity::Device,
    node: std::sync::Arc<ankai_core::p2p::P2pNode>,
    own_invite: ankai_core::messaging::PeerInvite,
    own_invite_text: String,
}

thread_local! {
    // App-wide DB access for UI-thread callbacks that return from async work.
    // This remains available even when optional MLS/P2P startup fails.
    static DB_HANDLE: RefCell<Option<Rc<ankai_core::db::Db>>> = const { RefCell::new(None) };
    static MESSAGING_HANDLES: RefCell<Option<MessagingHandles>> = const { RefCell::new(None) };
}

// The full `AnimeSummary` list behind the Home dashboard's last real
// Trending Now load — kept around (beyond the trimmed `AnimeRef` the UI
// actually renders) purely so the hero card's real "Watch Now" button can
// look one up by id and call `ankai_core::anime::add_to_watchlist` with a
// real `AnimeSummary`, not just a title string. Same thread-local-bridge
// shape as `MESSAGING_HANDLES` above and for the same reason: written from
// inside a `slint::invoke_from_event_loop` closure (see
// `spawn_anime_refresh`) and read back from `on_watch_now`'s callback —
// both guaranteed to run on the single UI thread, so a thread-local
// `RefCell` is safe without needing the closures themselves to be `Sync`.
thread_local! {
    static HOME_TRENDING_ANIME: RefCell<Vec<ankai_core::anime::AnimeSummary>> =
        const { RefCell::new(Vec::new()) };
    static HOME_POPULAR_ANIME: RefCell<Vec<ankai_core::anime::AnimeSummary>> =
        const { RefCell::new(Vec::new()) };
    static STREMIO_MEDIA: RefCell<Vec<ankai_core::stremio::MetaPreview>> = const { RefCell::new(Vec::new()) };
    static STREMIO_STREAMS: RefCell<Vec<ankai_core::stremio::Stream>> = const { RefCell::new(Vec::new()) };
    static STREMIO_ADDONS: RefCell<Vec<StremioAddon>> = const { RefCell::new(Vec::new()) };
    static STREMIO_MEDIA_SOURCES: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    static STREMIO_EPISODES: RefCell<Vec<ankai_core::stremio::Video>> = const { RefCell::new(Vec::new()) };
    static RESUME_PROGRESS: RefCell<Vec<ankai_core::playback_progress::PlaybackProgress>> = const { RefCell::new(Vec::new()) };
    static VIDEO_PLAYER: RefCell<Option<playback::Player>> = const { RefCell::new(None) };
    static VIDEO_SURFACE: RefCell<Option<playback::BoundedVideoSurface>> = const { RefCell::new(None) };
    static LAST_PLAYBACK_URL: RefCell<Option<String>> = const { RefCell::new(None) };
    static STREMIO_SELECTED_CONTEXT: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
    static PENDING_PLAYBACK_KEY: RefCell<Option<ankai_core::playback_progress::PlaybackKey>> = const { RefCell::new(None) };
    static ACTIVE_PLAYBACK_KEY: RefCell<Option<ankai_core::playback_progress::PlaybackKey>> = const { RefCell::new(None) };
    static PENDING_PLAYBACK_METADATA: RefCell<ankai_core::playback_progress::PlaybackMetadata> =
        RefCell::new(ankai_core::playback_progress::PlaybackMetadata::default());
    static ACTIVE_PLAYBACK_METADATA: RefCell<ankai_core::playback_progress::PlaybackMetadata> =
        RefCell::new(ankai_core::playback_progress::PlaybackMetadata::default());
    static PENDING_RESUME_SECONDS: RefCell<Option<f64>> = const { RefCell::new(None) };
    // Set only by an `ankai://video?...` deep link (see `on_open_deep_link`):
    // the specific episode/video id to auto-select once the title's meta
    // fetch (open_meta_by_id) populates STREMIO_EPISODES. Consumed (taken)
    // the moment that fetch completes, whether or not the id was found.
    static PENDING_DEEP_LINK_VIDEO: RefCell<Option<String>> = const { RefCell::new(None) };
    static LAST_PROGRESS_SAVE: RefCell<Option<std::time::Instant>> = const { RefCell::new(None) };

    // Network completions return to the Slint event loop out of order. Each
    // independently replaceable UI model owns a generation counter so only
    // the most recently issued request (and its image children) may mutate
    // that model. These counters are event-loop local by design: requests are
    // issued and checked only on Slint's UI thread.
    static STREMIO_SEARCH_GENERATION: RequestGeneration = const { RequestGeneration::new() };
    static STREMIO_DETAIL_GENERATION: RequestGeneration = const { RequestGeneration::new() };
    static STREMIO_MEDIA_GENERATION: RequestGeneration = const { RequestGeneration::new() };
    static STREMIO_META_GENERATION: RequestGeneration = const { RequestGeneration::new() };
    static ADDON_MANAGER_GENERATION: RequestGeneration = const { RequestGeneration::new() };
    static HOME_ANIME_GENERATION: RequestGeneration = const { RequestGeneration::new() };
    static WATCHLIST_GENERATION: RequestGeneration = const { RequestGeneration::new() };
    static LETTERBOXD_GENERATION: RequestGeneration = const { RequestGeneration::new() };
    static RESUME_GENERATION: RequestGeneration = const { RequestGeneration::new() };
    static STREMIO_ADDON_LOAD_SEQUENCE: RequestGeneration = const { RequestGeneration::new() };
    static STREMIO_ADDON_LOAD_GENERATIONS: RefCell<std::collections::HashMap<String, u64>> =
        RefCell::new(std::collections::HashMap::new());
}

// S21 (docs/roadmaps/stremio-competitive-parity.md): catalog-aware Board
// state. `BOARD_CATALOGS` is every eligible catalog across every loaded
// addon (ankai_core::board::board_catalogs, recomputed whenever STREMIO_ADDONS
// changes); `BOARD_PAGE_STATE` is each catalog's own skip-pagination state
// (ankai_core::board::CatalogPageState), keyed by `board_row_key`; `BOARD_FILTER`
// is the user's current type/addon/catalog/genre selection. `stremio-media`/
// `STREMIO_MEDIA_SOURCES` remain the flat item list `open-stremio-media`,
// `open-stremio-episode` and playback already index into — the Board UI now
// populates that flat list from per-catalog rows (or from merged search
// results while a search is active) instead of one ad hoc merged shelf, but
// every downstream selection/streams/playback callback is unchanged.
// `BOARD_MEMORY_CACHE` is the in-memory tier of the stale-while-revalidate
// cache (ankai_core::catalog_cache); the disk tier goes through DB_HANDLE, so
// it is only ever touched on the UI thread, same as every other DB access in
// this file — see MESSAGING_HANDLES's doc comment above for why.
#[derive(Debug, Clone, Default)]
struct BoardFilter {
    selected_type: Option<String>,
    selected_addon_url: Option<String>,
    selected_catalog_key: Option<String>,
    selected_genre: Option<String>,
}

thread_local! {
    static BOARD_CATALOGS: RefCell<Vec<ankai_core::board::BoardCatalogRef>> =
        const { RefCell::new(Vec::new()) };
    static BOARD_PAGE_STATE: RefCell<std::collections::HashMap<String, ankai_core::board::CatalogPageState>> =
        RefCell::new(std::collections::HashMap::new());
    static BOARD_FILTER: RefCell<BoardFilter> = RefCell::new(BoardFilter::default());
    static BOARD_MEMORY_CACHE: ankai_core::catalog_cache::MemoryCatalogCache =
        ankai_core::catalog_cache::MemoryCatalogCache::default();
    static BOARD_SEARCH_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

/// The stable identity a Board row (and its pagination/loading state) is
/// keyed by — same shape as [`ankai_core::catalog_cache::cache_key`] with no
/// extras, since a row's identity does not change as its pages accumulate.
fn board_row_key(catalog: &ankai_core::board::BoardCatalogRef) -> String {
    ankai_core::catalog_cache::cache_key(
        &catalog.key.addon_manifest_url,
        &catalog.key.media_type,
        &catalog.key.catalog_id,
        &[],
    )
}

/// A tiny event-loop-local last-request-wins token source.
///
/// `issue` is monotonically increasing for the practical lifetime of the
/// process. It wraps without panicking after `u64::MAX` requests and skips the
/// initial zero token.
struct RequestGeneration(Cell<u64>);

impl RequestGeneration {
    const fn new() -> Self {
        Self(Cell::new(0))
    }

    fn issue(&self) -> u64 {
        let next = self.0.get().wrapping_add(1).max(1);
        self.0.set(next);
        next
    }

    fn is_current(&self, token: u64) -> bool {
        self.0.get() == token
    }

    fn current(&self) -> u64 {
        self.0.get()
    }
}

fn issue_addon_load(url: &str) -> u64 {
    let token = STREMIO_ADDON_LOAD_SEQUENCE.with(RequestGeneration::issue);
    STREMIO_ADDON_LOAD_GENERATIONS.with(|requests| {
        requests.borrow_mut().insert(url.to_owned(), token);
    });
    token
}

fn is_current_addon_load(url: &str, token: u64) -> bool {
    STREMIO_ADDON_LOAD_GENERATIONS.with(|requests| requests.borrow().get(url) == Some(&token))
}

#[derive(Clone)]
struct StremioAddon {
    manifest_url: String,
    client: ankai_core::stremio::AddonClient,
    name: String,
    manifest: ankai_core::stremio::Manifest,
}

fn stremio_stream_kind(stream: &ankai_core::stremio::Stream) -> &'static str {
    use ankai_core::stremio::StreamTarget;

    match stream.target() {
        Ok(StreamTarget::DirectUrl(_)) => "Direct",
        Ok(StreamTarget::YouTube(_)) => "YouTube",
        Ok(StreamTarget::BitTorrent { .. }) => "Torrent",
        Ok(StreamTarget::Nzb { .. }) => "NZB",
        Ok(StreamTarget::Rar(_)) => "RAR",
        Ok(StreamTarget::Zip(_)) => "ZIP",
        Ok(StreamTarget::SevenZip(_)) => "7-Zip",
        Ok(StreamTarget::Tgz(_)) => "TGZ",
        Ok(StreamTarget::Tar(_)) => "TAR",
        Ok(StreamTarget::ExternalUrl(_)) => "External",
        Err(_) => "Invalid",
    }
}

/// Coarse resolution tier parsed from a stream's advertised `name`/`title`,
/// best (0) to worst/unknown (5). Real Stremio-ecosystem addons (Torrentio
/// and friends) pack this into free-text release-name-style labels — e.g.
/// "Torrentio\n1080p HEVC" or "[YTS] Show.S01E02.2160p.WEB-DL" — so this is
/// deliberately a plain substring scan for the handful of tokens that
/// actually show up in practice, not a general release-name parser. Same
/// "simple and predictable over clever-but-opaque" precedent as
/// `rank_search_results_by_relevance` below.
fn stream_quality_tier(stream: &ankai_core::stremio::Stream) -> u8 {
    let text = format!(
        "{} {}",
        stream.name.as_deref().unwrap_or(""),
        stream.title.as_deref().unwrap_or("")
    )
    .to_lowercase();
    if text.contains("2160p") || text.contains("4k") || text.contains("uhd") {
        0
    } else if text.contains("1080p") {
        1
    } else if text.contains("720p") {
        2
    } else if text.contains("480p") {
        3
    } else if text.contains("360p") {
        4
    } else {
        5
    }
}

/// Stable-sorts so a stream this build can actually start playing on click
/// (`StreamSource::Direct` — see `Stream::source`'s doc comment) always comes
/// before torrent/YouTube/NZB/archive/external streams, which are honestly
/// labelled but not resolvable yet, and — among Direct streams — ranks
/// higher resolutions first via `stream_quality_tier` above. The first/
/// primary "PLAY FROM" button (and the automatic best-stream playback this
/// ranking now also drives — see the `invoke_activate_stremio_stream(0)`
/// call sites) should be something that both works and is the best quality
/// on offer, not just whichever addon answered first.
fn rank_streams_for_playability(streams: &mut [ankai_core::stremio::Stream]) {
    streams.sort_by_key(|stream| {
        let not_direct = !matches!(
            stream.source(),
            Ok(ankai_core::stremio::StreamSource::Direct(_))
        );
        (not_direct, stream_quality_tier(stream))
    });
}

/// True when the highest-ranked stream (index 0 after `rank_streams_for_
/// playability`) is one this build can actually start playing immediately.
/// Drives the auto-play call sites: if the top pick isn't `Direct` (only
/// torrent/YouTube/NZB/etc. came back), autoplay is skipped and the
/// existing honest "no playable stream"/"a torrent resolver is required"
/// messaging is left to do its job instead of silently no-oping.
fn top_stream_is_playable(streams: &[ankai_core::stremio::Stream]) -> bool {
    streams.first().is_some_and(|stream| {
        matches!(
            stream.source(),
            Ok(ankai_core::stremio::StreamSource::Direct(_))
        )
    })
}

/// Ranks merged search results by textual closeness to the query so the
/// title a user actually meant is what appears first (and lands in the
/// primary card position), rather than whichever addon/catalog happened to
/// answer first. Tiers, most to least relevant: exact match, starts-with,
/// contains — each tier stable-sorted internally to preserve addon-priority
/// order. A plain heuristic (case-insensitive string comparison), not a full
/// fuzzy scorer — deliberately simple and predictable over clever-but-opaque.
fn rank_search_results_by_relevance(query: &str, items: &mut [ankai_core::board::AttributedItem]) {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return;
    }
    items.sort_by_key(|entry| {
        let name = entry.item.name.to_lowercase();
        if name == query {
            0
        } else if name.starts_with(&query) {
            1
        } else if name.contains(&query) {
            2
        } else {
            3
        }
    });
}

fn show_stremio_media(
    app: &AppWindow,
    media: Vec<ankai_core::stremio::MetaPreview>,
    sources: Vec<usize>,
    image_handle: &tokio::runtime::Handle,
) {
    let generation = STREMIO_MEDIA_GENERATION.with(RequestGeneration::issue);
    let cards = media
        .iter()
        .map(|item| StremioMediaRef {
            title: item.name.clone().into(),
            detail: format!(
                "{} · {}",
                item.media_type,
                item.release_info.as_deref().unwrap_or("Unknown release")
            )
            .into(),
            poster: slint::Image::default(),
            has_poster: false,
        })
        .collect::<Vec<_>>();
    let posters = media
        .iter()
        .enumerate()
        .filter_map(|(index, item)| item.poster.clone().map(|url| (index, item.id.clone(), url)))
        .collect::<Vec<_>>();
    STREMIO_MEDIA.with(|slot| *slot.borrow_mut() = media);
    STREMIO_MEDIA_SOURCES.with(|slot| *slot.borrow_mut() = sources);
    app.set_stremio_media(slint::ModelRc::from(Rc::new(slint::VecModel::from(cards))));
    for (index, media_id, url) in posters {
        let expected_url = url.clone();
        images::load_cover_image(
            url,
            image_handle.clone(),
            app.as_weak(),
            move |app, image| {
                if !STREMIO_MEDIA_GENERATION.with(|state| state.is_current(generation)) {
                    return;
                }
                let still_matches = STREMIO_MEDIA.with(|media| {
                    media.borrow().get(index).is_some_and(|item| {
                        item.id == media_id && item.poster.as_deref() == Some(expected_url.as_str())
                    })
                });
                if !still_matches {
                    return;
                }
                let model = app.get_stremio_media();
                let Some(model) = model
                    .as_any()
                    .downcast_ref::<slint::VecModel<StremioMediaRef>>()
                else {
                    return;
                };
                if let Some(mut row) = model.row_data(index) {
                    row.poster = image;
                    row.has_poster = true;
                    model.set_row_data(index, row);
                }
            },
        );
    }
}

fn show_stremio_meta(
    app: &AppWindow,
    meta: &ankai_core::stremio::Meta,
    image_handle: &tokio::runtime::Handle,
) {
    let generation = STREMIO_META_GENERATION.with(RequestGeneration::issue);
    app.set_stremio_selected_title(meta.name.clone().into());
    app.set_stremio_selected_description(meta.description.clone().unwrap_or_default().into());
    app.set_stremio_selected_meta_line(
        format!(
            "{}{}",
            meta.media_type,
            if meta.genres.is_empty() {
                String::new()
            } else {
                format!("  ·  {}", meta.genres.join("  ·  "))
            }
        )
        .into(),
    );
    app.set_stremio_selected_cast(meta.cast.join(", ").into());

    let episode_cards = meta
        .videos
        .iter()
        .map(|episode| StremioEpisodeRef {
            title: episode
                .title
                .clone()
                .unwrap_or_else(|| match (episode.season, episode.episode) {
                    (Some(season), Some(number)) => format!("S{season:02}E{number:02}"),
                    (_, Some(number)) => format!("Episode {number}"),
                    _ => "Episode".into(),
                })
                .into(),
            detail: match (episode.season, episode.episode, episode.released.as_deref()) {
                (Some(season), Some(number), Some(released)) => {
                    format!("S{season:02}E{number:02}  ·  {released}")
                }
                (Some(season), Some(number), None) => {
                    format!("Season {season}  ·  Episode {number}")
                }
                (_, _, Some(released)) => released.to_owned(),
                _ => String::new(),
            }
            .into(),
            thumbnail: slint::Image::default(),
            has_thumbnail: false,
        })
        .collect::<Vec<_>>();
    let thumbnails = meta
        .videos
        .iter()
        .enumerate()
        .filter_map(|(index, episode)| {
            episode
                .thumbnail
                .clone()
                .map(|url| (index, episode.id.clone(), url))
        })
        .collect::<Vec<_>>();
    STREMIO_EPISODES.with(|slot| *slot.borrow_mut() = meta.videos.clone());
    app.set_stremio_episodes(slint::ModelRc::from(Rc::new(slint::VecModel::from(
        episode_cards,
    ))));

    for (index, episode_id, url) in thumbnails {
        let expected_url = url.clone();
        images::load_cover_image(
            url,
            image_handle.clone(),
            app.as_weak(),
            move |app, image| {
                if !STREMIO_META_GENERATION.with(|state| state.is_current(generation)) {
                    return;
                }
                let still_matches = STREMIO_EPISODES.with(|episodes| {
                    episodes.borrow().get(index).is_some_and(|episode| {
                        episode.id == episode_id
                            && episode.thumbnail.as_deref() == Some(expected_url.as_str())
                    })
                });
                if !still_matches {
                    return;
                }
                let model = app.get_stremio_episodes();
                let Some(model) = model
                    .as_any()
                    .downcast_ref::<slint::VecModel<StremioEpisodeRef>>()
                else {
                    return;
                };
                if let Some(mut row) = model.row_data(index) {
                    row.thumbnail = image;
                    row.has_thumbnail = true;
                    model.set_row_data(index, row);
                }
            },
        );
    }
}

fn refresh_addon_manager(
    app: &AppWindow,
    db: &ankai_core::db::Db,
    image_handle: &tokio::runtime::Handle,
) {
    let generation = ADDON_MANAGER_GENERATION.with(RequestGeneration::issue);
    match ankai_core::addons::list(db) {
        Ok(addons) => {
            let total = addons.len();
            let logo_urls = addons
                .iter()
                .enumerate()
                .filter_map(|(index, addon)| {
                    addon
                        .logo
                        .clone()
                        .map(|url| (index, addon.manifest_url.clone(), url))
                })
                .collect::<Vec<_>>();
            let cards = addons
                .into_iter()
                .enumerate()
                .map(|(index, addon)| {
                    let mut roles = Vec::new();
                    if addon.roles.catalog {
                        roles.push("Catalog");
                    }
                    if addon.roles.addon_catalog {
                        roles.push("Addon directory");
                    }
                    if addon.roles.meta {
                        roles.push("Metadata");
                    }
                    if addon.roles.stream {
                        roles.push("Streams");
                    }
                    if addon.roles.subtitles {
                        roles.push("Subtitles");
                    }
                    let (health_state, health_message) = match addon.health.state {
                        ankai_core::addons::AddonHealthState::Healthy => {
                            ("healthy", "Manifest checked successfully".to_owned())
                        }
                        ankai_core::addons::AddonHealthState::Failing => (
                            "failing",
                            addon
                                .health
                                .last_error
                                .clone()
                                .unwrap_or_else(|| "Latest manifest check failed".into()),
                        ),
                        ankai_core::addons::AddonHealthState::Unknown => {
                            ("unknown", "Not checked in this session".to_owned())
                        }
                    };
                    let is_builtin = matches!(
                        addon.manifest_url.as_str(),
                        CINEMETA_MANIFEST_URL | ANIME_KITSU_MANIFEST_URL
                    );
                    AddonCard {
                        id: addon.manifest_url.clone().into(),
                        name: if is_builtin {
                            format!("{}  ·  Built in", addon.name).into()
                        } else {
                            addon.name.into()
                        },
                        version: addon.version.into(),
                        manifest_url: addon.manifest_url.into(),
                        logo: slint::Image::default(),
                        has_logo: false,
                        resource_roles: if roles.is_empty() {
                            "No declared resources".into()
                        } else {
                            roles.join("  ·  ").into()
                        },
                        media_types: addon.types.join("  ·  ").into(),
                        enabled: addon.enabled,
                        health_state: health_state.into(),
                        health_message: health_message.into(),
                        priority: addon.priority as i32 + 1,
                        can_move_up: index > 0,
                        can_move_down: index + 1 < total,
                        removable: !is_builtin,
                    }
                })
                .collect::<Vec<_>>();
            app.set_installed_addons(slint::ModelRc::from(Rc::new(slint::VecModel::from(cards))));
            app.set_addon_manager_state("ready".into());
            app.set_addon_manager_error("".into());

            for (index, manifest_url, url) in logo_urls {
                images::load_cover_image(
                    url,
                    image_handle.clone(),
                    app.as_weak(),
                    move |app, image| {
                        if !ADDON_MANAGER_GENERATION.with(|state| state.is_current(generation)) {
                            return;
                        }
                        let model = app.get_installed_addons();
                        let Some(model) =
                            model.as_any().downcast_ref::<slint::VecModel<AddonCard>>()
                        else {
                            return;
                        };
                        if let Some(mut row) = model.row_data(index) {
                            if row.id.as_str() != manifest_url {
                                return;
                            }
                            row.logo = image;
                            row.has_logo = true;
                            model.set_row_data(index, row);
                        }
                    },
                );
            }
        }
        Err(error) => {
            app.set_addon_manager_state("error".into());
            app.set_addon_manager_error(error.to_string().into());
        }
    }
}

fn reload_enabled_addons(app: &AppWindow, db: &ankai_core::db::Db) {
    // A registry reload clears the catalog. Any search that was issued
    // against the previous addon snapshot must not repopulate it afterward.
    STREMIO_SEARCH_GENERATION.with(RequestGeneration::issue);
    STREMIO_DETAIL_GENERATION.with(RequestGeneration::issue);
    STREMIO_MEDIA_GENERATION.with(RequestGeneration::issue);
    STREMIO_ADDONS.with(|slot| slot.borrow_mut().clear());
    STREMIO_MEDIA.with(|slot| slot.borrow_mut().clear());
    STREMIO_MEDIA_SOURCES.with(|slot| slot.borrow_mut().clear());
    STREMIO_STREAMS.with(|slot| slot.borrow_mut().clear());
    STREMIO_EPISODES.with(|slot| slot.borrow_mut().clear());
    BOARD_CATALOGS.with(|slot| slot.borrow_mut().clear());
    BOARD_PAGE_STATE.with(|slot| slot.borrow_mut().clear());
    BOARD_FILTER.with(|slot| *slot.borrow_mut() = BoardFilter::default());
    BOARD_SEARCH_ACTIVE.with(|active| active.set(false));
    app.set_stremio_media(slint::ModelRc::default());
    app.set_stremio_stream_names(slint::ModelRc::default());
    app.set_stremio_episodes(slint::ModelRc::default());
    app.set_stremio_selected_title("".into());
    app.set_board_rows(slint::ModelRc::default());
    app.set_board_search_active(false);

    match ankai_core::addons::list(db) {
        Ok(addons) => {
            for addon in addons.into_iter().filter(|addon| addon.enabled) {
                app.invoke_load_stremio_addon(addon.manifest_url.into());
            }
        }
        Err(error) => app.set_stremio_status(format!("Couldn't reload addons: {error}").into()),
    }
}

/// Recomputes [`BOARD_CATALOGS`] from every currently loaded addon's live
/// manifest plus the persisted registry's enabled/priority state. Cheap and
/// synchronous (no network): call after any change to `STREMIO_ADDONS` or the
/// addon registry, before [`rebuild_board_ui`].
fn rebuild_board_catalogs(db: &ankai_core::db::Db) {
    let manifests: Vec<(String, ankai_core::stremio::Manifest)> = STREMIO_ADDONS.with(|slot| {
        slot.borrow()
            .iter()
            .map(|addon| (addon.manifest_url.clone(), addon.manifest.clone()))
            .collect()
    });
    let installed = ankai_core::addons::list(db).unwrap_or_default();
    let catalogs = ankai_core::board::board_catalogs(&installed, &manifests);
    BOARD_CATALOGS.with(|slot| *slot.borrow_mut() = catalogs);
}

/// Applies [`BOARD_FILTER`] to [`BOARD_CATALOGS`]: a specific selected
/// catalog always wins (letting an otherwise extra-gated catalog, e.g. one
/// that requires a genre, be browsed once explicitly chosen); otherwise only
/// passive catalogs (no required extra — see
/// [`ankai_core::board::BoardCatalogRef::is_passive`]) matching the selected
/// type/addon are shown as Board rows.
fn visible_board_rows<'a>(
    catalogs: &'a [ankai_core::board::BoardCatalogRef],
    filter: &BoardFilter,
) -> Vec<&'a ankai_core::board::BoardCatalogRef> {
    if let Some(selected_key) = &filter.selected_catalog_key {
        return catalogs
            .iter()
            .filter(|catalog| &board_row_key(catalog) == selected_key)
            .collect();
    }
    catalogs
        .iter()
        .filter(|catalog| catalog.is_passive)
        .filter(|catalog| {
            filter
                .selected_type
                .as_ref()
                .is_none_or(|media_type| *media_type == catalog.media_type)
        })
        .filter(|catalog| {
            filter
                .selected_addon_url
                .as_ref()
                .is_none_or(|url| *url == catalog.key.addon_manifest_url)
        })
        .collect()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

/// Renders a [`ankai_core::board::CacheNotice`] as one honest, human-readable
/// disclosure line — never silently hidden when a row is showing stale data.
fn format_cache_notice(notice: &ankai_core::board::CacheNotice) -> String {
    let elapsed = unix_now().saturating_sub(notice.fetched_at_unix);
    let age = if elapsed < 60 {
        "moments ago".to_string()
    } else if elapsed < 3600 {
        format!("{} min ago", elapsed / 60)
    } else if elapsed < 86_400 {
        format!("{} h ago", elapsed / 3600)
    } else {
        format!("{} d ago", elapsed / 86_400)
    };
    format!(
        "Showing cached results from {age} — live refresh failed: {}",
        notice.live_error
    )
}

/// Rebuilds every Board-visible thing from current state: the flat
/// `stremio-media`/sources list (via [`show_stremio_media`], so poster
/// loading/generation-guarding stays exactly as before), the per-catalog
/// `board-rows` model, and the type/addon/catalog/genre selector option
/// lists — all generated from [`BOARD_CATALOGS`], never hand-maintained. Also
/// kicks off a fetch for any visible row that has never been loaded. Does
/// nothing to `stremio-media` while [`BOARD_SEARCH_ACTIVE`] is true — a live
/// search result list owns that flat list until the search is cleared.
fn rebuild_board_ui(
    app: &AppWindow,
    runtime: &tokio::runtime::Handle,
    image_handle: &tokio::runtime::Handle,
) {
    let catalogs = BOARD_CATALOGS.with(|slot| slot.borrow().clone());
    let filter = BOARD_FILTER.with(|slot| slot.borrow().clone());
    let visible = visible_board_rows(&catalogs, &filter);

    // Selector option lists are always generated from the *full* catalog
    // set (not just what's currently visible), so narrowing one filter never
    // hides the others' remaining choices.
    let types = ankai_core::board::available_types(&catalogs);
    let addons = ankai_core::board::available_addons(&catalogs);
    let catalog_options: Vec<(String, String)> = catalogs
        .iter()
        .map(|catalog| {
            (
                format!("{}  ·  {}", catalog.catalog_name, catalog.addon_name),
                board_row_key(catalog),
            )
        })
        .collect();
    let genre_options: Vec<String> = filter
        .selected_catalog_key
        .as_ref()
        .and_then(|key| {
            catalogs
                .iter()
                .find(|catalog| &board_row_key(catalog) == key)
        })
        .map(|catalog| catalog.genres.clone())
        .unwrap_or_default();

    app.set_board_types(slint::ModelRc::from(std::rc::Rc::new(
        slint::VecModel::from(
            std::iter::once(filter_option("All types", ""))
                .chain(types.into_iter().map(|t| filter_option(&t, &t)))
                .collect::<Vec<_>>(),
        ),
    )));
    app.set_board_selected_type(filter.selected_type.clone().unwrap_or_default().into());
    app.set_board_addons(slint::ModelRc::from(std::rc::Rc::new(
        slint::VecModel::from(
            std::iter::once(filter_option("All addons", ""))
                .chain(
                    addons
                        .into_iter()
                        .map(|(url, name)| filter_option(&name, &url)),
                )
                .collect::<Vec<_>>(),
        ),
    )));
    app.set_board_selected_addon(filter.selected_addon_url.clone().unwrap_or_default().into());
    app.set_board_catalogs(slint::ModelRc::from(std::rc::Rc::new(
        slint::VecModel::from(
            std::iter::once(filter_option("All catalogs", ""))
                .chain(
                    catalog_options
                        .into_iter()
                        .map(|(label, key)| filter_option(&label, &key)),
                )
                .collect::<Vec<_>>(),
        ),
    )));
    app.set_board_selected_catalog(
        filter
            .selected_catalog_key
            .clone()
            .unwrap_or_default()
            .into(),
    );
    app.set_board_genres(slint::ModelRc::from(std::rc::Rc::new(
        slint::VecModel::from(
            std::iter::once(filter_option("All genres", ""))
                .chain(genre_options.into_iter().map(|g| filter_option(&g, &g)))
                .collect::<Vec<_>>(),
        ),
    )));
    app.set_board_selected_genre(filter.selected_genre.clone().unwrap_or_default().into());

    if BOARD_SEARCH_ACTIVE.with(Cell::get) {
        return;
    }

    // Mark any never-loaded visible row as loading up front, so the flat
    // item list and the row headers both reflect it in one paint rather than
    // flickering in a frame later.
    let mut to_fetch = Vec::new();
    BOARD_PAGE_STATE.with(|slot| {
        let mut states = slot.borrow_mut();
        for catalog in &visible {
            let key = board_row_key(catalog);
            let state = states
                .entry(key.clone())
                .or_insert_with(ankai_core::board::CatalogPageState::new);
            if !state.initialized && !state.loading && state.error.is_none() {
                state.loading = true;
                to_fetch.push(key);
            }
        }
    });

    let addon_index_for = |url: &str| {
        STREMIO_ADDONS.with(|slot| {
            slot.borrow()
                .iter()
                .position(|addon| addon.manifest_url == url)
        })
    };

    let mut flat_items = Vec::new();
    let mut flat_sources = Vec::new();
    let mut rows = Vec::new();
    BOARD_PAGE_STATE.with(|slot| {
        let states = slot.borrow();
        for catalog in &visible {
            let key = board_row_key(catalog);
            let state = states.get(&key).cloned().unwrap_or_default();
            let start = flat_items.len() as i32;
            let addon_index =
                addon_index_for(&catalog.key.addon_manifest_url).unwrap_or(usize::MAX);
            for item in &state.items {
                flat_items.push(item.clone());
                flat_sources.push(addon_index);
            }
            rows.push(BoardRowRef {
                row_key: key.into(),
                addon_name: catalog.addon_name.clone().into(),
                catalog_name: catalog.catalog_name.clone().into(),
                media_type: catalog.media_type.clone().into(),
                item_start: start,
                item_count: state.items.len() as i32,
                loading: state.loading,
                initialized: state.initialized,
                has_more: state.has_more,
                error_message: state.error.clone().unwrap_or_default().into(),
                cache_notice: state
                    .cache_notice
                    .as_ref()
                    .map(format_cache_notice)
                    .unwrap_or_default()
                    .into(),
            });
        }
    });

    show_stremio_media(app, flat_items, flat_sources, image_handle);
    app.set_board_rows(slint::ModelRc::from(std::rc::Rc::new(
        slint::VecModel::from(rows),
    )));

    for key in to_fetch {
        fetch_board_row(runtime, app.as_weak(), image_handle.clone(), key);
    }
}

fn filter_option(label: &str, value: &str) -> FilterOption {
    FilterOption {
        label: label.into(),
        value: value.into(),
    }
}

/// Fetches the next page for one Board row (skip = its current
/// `next_skip`) and, on completion, applies the stale-while-revalidate
/// policy documented on [`ankai_core::board::fetch_catalog_page`] — this
/// client-side version implements the same policy but split across the
/// event-loop/background-runtime boundary every other network call in this
/// file already uses (the live fetch runs on `runtime`; the memory/disk
/// cache reads and writes run back on the UI thread, since [`ankai_core::db::Db`]
/// is `Send` but not `Sync` and is only ever touched from there — see
/// `MESSAGING_HANDLES`'s doc comment).
fn fetch_board_row(
    runtime: &tokio::runtime::Handle,
    app_weak: slint::Weak<AppWindow>,
    image_handle: tokio::runtime::Handle,
    row_key: String,
) {
    let catalog = BOARD_CATALOGS.with(|slot| {
        slot.borrow()
            .iter()
            .find(|catalog| board_row_key(catalog) == row_key)
            .cloned()
    });
    let Some(catalog) = catalog else {
        return;
    };
    let client = STREMIO_ADDONS.with(|slot| {
        slot.borrow()
            .iter()
            .find(|addon| addon.manifest_url == catalog.key.addon_manifest_url)
            .map(|addon| addon.client.clone())
    });
    let Some(client) = client else {
        return;
    };

    let skip = BOARD_PAGE_STATE.with(|slot| {
        slot.borrow()
            .get(&row_key)
            .map(|s| s.next_skip)
            .unwrap_or(0)
    });
    let filter = BOARD_FILTER.with(|slot| slot.borrow().clone());
    let mut extra: Vec<(String, String)> = Vec::new();
    if skip > 0 {
        extra.push(("skip".into(), skip.to_string()));
    }
    if filter.selected_catalog_key.as_deref() == Some(row_key.as_str()) {
        if let Some(genre) = &filter.selected_genre {
            if !genre.is_empty() {
                extra.push(("genre".into(), genre.clone()));
            }
        }
    }

    let addon_url = catalog.key.addon_manifest_url.clone();
    let media_type = catalog.key.media_type.clone();
    let catalog_id = catalog.key.catalog_id.clone();
    let row_key_for_task = row_key.clone();
    let runtime_for_apply = runtime.clone();
    runtime.spawn(async move {
        let extra_refs: Vec<(&str, &str)> = extra
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let result = client
            .catalog_page_with_extra(&media_type, &catalog_id, &extra_refs)
            .await;
        let _ = slint::invoke_from_event_loop(move || {
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            apply_board_fetch_result(
                &app,
                &runtime_for_apply,
                &addon_url,
                &media_type,
                &catalog_id,
                &row_key_for_task,
                &extra,
                result,
                &image_handle,
            );
        });
    });
}

/// Applies one board row's live-fetch result on the UI thread: on success,
/// writes through to both cache tiers; on failure, falls back to whichever
/// cache tier has this exact key (memory first, then disk) and tags the
/// applied page with an honest [`ankai_core::board::CacheNotice`]; with
/// neither a live result nor a cache hit, records the error on the row.
/// Always ends by calling [`rebuild_board_ui`] so the change is visible.
#[allow(clippy::too_many_arguments)]
fn apply_board_fetch_result(
    app: &AppWindow,
    runtime: &tokio::runtime::Handle,
    addon_url: &str,
    media_type: &str,
    catalog_id: &str,
    row_key: &str,
    extra: &[(String, String)],
    result: Result<ankai_core::stremio::CatalogPage, ankai_core::Error>,
    image_handle: &tokio::runtime::Handle,
) {
    let extra_refs: Vec<(&str, &str)> = extra
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let cache_key =
        ankai_core::catalog_cache::cache_key(addon_url, media_type, catalog_id, &extra_refs);

    let (items, cache_notice) = match result {
        Ok(page) => {
            let cached = ankai_core::catalog_cache::CachedCatalogPage {
                items: page.metas.clone(),
                fetched_at_unix: unix_now(),
                cache_max_age_secs: page.cache_max_age,
            };
            BOARD_MEMORY_CACHE.with(|cache| cache.put(cache_key.clone(), cached.clone()));
            DB_HANDLE.with(|db| {
                if let Some(db) = db.borrow().as_ref() {
                    if let Err(error) = ankai_core::catalog_cache::put(
                        db, &cache_key, addon_url, media_type, catalog_id, &cached,
                    ) {
                        eprintln!("ankai-client: failed to write catalog cache: {error}");
                    }
                }
            });
            (page.metas, None)
        }
        Err(error) => {
            let memory_hit = BOARD_MEMORY_CACHE.with(|cache| cache.get(&cache_key));
            let cached = memory_hit.or_else(|| {
                DB_HANDLE.with(|db| {
                    db.borrow()
                        .as_ref()
                        .and_then(|db| {
                            ankai_core::catalog_cache::get(db, &cache_key)
                                .ok()
                                .flatten()
                        })
                        .map(|lookup| lookup.page)
                })
            });
            match cached {
                Some(cached) => (
                    cached.items,
                    Some(ankai_core::board::CacheNotice {
                        fetched_at_unix: cached.fetched_at_unix,
                        live_error: error.to_string(),
                    }),
                ),
                None => {
                    BOARD_PAGE_STATE.with(|slot| {
                        slot.borrow_mut()
                            .entry(row_key.to_string())
                            .or_insert_with(ankai_core::board::CatalogPageState::new)
                            .apply_error(error.to_string());
                    });
                    rebuild_board_ui(app, runtime, image_handle);
                    return;
                }
            }
        }
    };

    BOARD_PAGE_STATE.with(|slot| {
        slot.borrow_mut()
            .entry(row_key.to_string())
            .or_insert_with(ankai_core::board::CatalogPageState::new)
            .apply_page(items, cache_notice);
    });
    rebuild_board_ui(app, runtime, image_handle);
}

/// Opens a title's Details view (metadata + streams) by explicit
/// `(type, id)`, for the `ankai://detail`/`ankai://video` deep-link routes
/// rather than an index into the flat `stremio-media` list a card click
/// would supply (see `on_open_stremio_media` for that path — the two
/// deliberately share `show_stremio_meta`/`STREMIO_STREAMS`/
/// `STREMIO_EPISODES` so playback and resume work identically either way).
/// `addon_hint` restricts the search to one addon when the link named one;
/// otherwise every loaded addon that declares `meta`/`stream` support for
/// this type/id is queried, same eligibility rule `open-stremio-media` uses.
fn open_meta_by_id(
    app: &AppWindow,
    runtime: &tokio::runtime::Handle,
    media_type: String,
    id: String,
    addon_hint: Option<String>,
) {
    let generation = STREMIO_DETAIL_GENERATION.with(RequestGeneration::issue);
    let addons: Vec<StremioAddon> = STREMIO_ADDONS.with(|slot| {
        slot.borrow()
            .iter()
            .filter(|addon| {
                addon_hint
                    .as_ref()
                    .is_none_or(|hint| *hint == addon.manifest_url)
            })
            .cloned()
            .collect()
    });
    if addons.is_empty() {
        app.set_shell_notice("No matching addon is installed for that link.".into());
        return;
    }
    let provider = addon_hint.clone().unwrap_or_else(|| {
        addons
            .iter()
            .find(|addon| addon.manifest.supports_resource("meta", &media_type, &id))
            .map(|addon| addon.manifest_url.clone())
            .unwrap_or_else(|| "stremio".into())
    });
    STREMIO_SELECTED_CONTEXT.with(|context| {
        *context.borrow_mut() = Some((provider.clone(), id.clone()));
    });
    PENDING_PLAYBACK_KEY.with(|key| {
        *key.borrow_mut() = Some(ankai_core::playback_progress::PlaybackKey::movie(
            provider,
            id.clone(),
        ));
    });
    PENDING_PLAYBACK_METADATA.with(|metadata| {
        *metadata.borrow_mut() = ankai_core::playback_progress::PlaybackMetadata::default()
    });
    app.set_stremio_selected_title("Loading…".into());
    app.set_stremio_selected_description("".into());
    app.set_stremio_selected_meta_line("".into());
    app.set_stremio_selected_cast("".into());
    app.set_stremio_selected_has_poster(false);
    app.set_stremio_episodes(slint::ModelRc::default());
    app.set_stremio_status("Loading title…".into());
    app.set_stremio_loading(true);

    let app_weak = app.as_weak();
    let image_handle = runtime.clone();
    runtime.spawn(async move {
        let mut streams = Vec::new();
        let mut errors = Vec::new();
        let mut selected_meta = None;
        for addon in &addons {
            if selected_meta.is_none() && addon.manifest.supports_resource("meta", &media_type, &id)
            {
                match addon.client.meta(&media_type, &id).await {
                    Ok(meta) => selected_meta = Some(meta),
                    Err(err) => errors.push(format!("{} metadata: {err}", addon.name)),
                }
            }
            if !addon.manifest.supports_resource("stream", &media_type, &id) {
                continue;
            }
            match addon.client.streams(&media_type, &id).await {
                Ok(mut addon_streams) => {
                    for stream in &mut addon_streams {
                        if stream.name.is_none() {
                            stream.name = Some(addon.name.clone());
                        }
                    }
                    streams.extend(addon_streams);
                }
                Err(err) => errors.push(format!("{}: {err}", addon.name)),
            }
        }
        let _ = slint::invoke_from_event_loop(move || {
            if !STREMIO_DETAIL_GENERATION.with(|state| state.is_current(generation)) {
                return;
            }
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            app.set_stremio_loading(false);
            if let Some(meta) = selected_meta.as_ref() {
                show_stremio_meta(&app, meta, &image_handle);
                PENDING_PLAYBACK_METADATA.with(|metadata| {
                    let mut metadata = metadata.borrow_mut();
                    metadata.title = Some(meta.name.clone());
                    metadata.poster_url = meta.poster.clone();
                });
                if let Some(poster_url) = meta.poster.clone() {
                    images::load_cover_image(
                        poster_url,
                        image_handle.clone(),
                        app.as_weak(),
                        |app, image| {
                            app.set_stremio_selected_poster(image);
                            app.set_stremio_selected_has_poster(true);
                        },
                    );
                }
            } else {
                STREMIO_EPISODES.with(|slot| slot.borrow_mut().clear());
            }
            if streams.is_empty() && !errors.is_empty() {
                app.set_stremio_status(format!("Error: {}", errors.join(" | ")).into());
            } else {
                rank_streams_for_playability(&mut streams);
                let autoplay = top_stream_is_playable(&streams);
                let labels = streams
                    .iter()
                    .map(|stream| {
                        format!(
                            "{} — {}",
                            stremio_stream_kind(stream),
                            stream
                                .title
                                .as_deref()
                                .or(stream.name.as_deref())
                                .unwrap_or("Unnamed stream")
                        )
                        .into()
                    })
                    .collect::<Vec<slint::SharedString>>();
                let count = streams.len();
                STREMIO_STREAMS.with(|slot| *slot.borrow_mut() = streams);
                app.set_stremio_stream_names(slint::ModelRc::from(Rc::new(slint::VecModel::from(
                    labels,
                ))));
                app.set_stremio_status(format!("Found {count} streams.").into());
                // Play the best-ranked stream automatically — the user
                // shouldn't have to pick from "PLAY FROM" for the common
                // case; that row stays populated below as a manual fallback
                // (e.g. after closing the player) if this pick is wrong.
                // Skipped when a `video` deep link is about to replace this
                // meta-level stream list with the actual episode's streams
                // just below, so this doesn't race/misplay the wrong item.
                if autoplay && !PENDING_DEEP_LINK_VIDEO.with(|slot| slot.borrow().is_some()) {
                    app.invoke_activate_stremio_stream(0);
                }
            }
            // A `video` deep link names a specific episode; auto-select it
            // now that STREMIO_EPISODES is populated, same as clicking it.
            if let Some(video_id) = PENDING_DEEP_LINK_VIDEO.with(|slot| slot.borrow_mut().take()) {
                let index = STREMIO_EPISODES
                    .with(|slot| slot.borrow().iter().position(|video| video.id == video_id));
                match index {
                    Some(index) => app.invoke_open_stremio_episode(index as i32),
                    None => app.set_shell_notice(
                        "That episode wasn't found in this title's video list.".into(),
                    ),
                }
            }
        });
    });
}

fn playback_time_label(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "--:--".into();
    }
    let total = seconds.round() as u64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn sync_player_state(app: &AppWindow, state: &playback::PlayerState) {
    use playback::{PlaybackPhase, TrackKind};

    app.set_player_has_media(state.has_media);
    if !state.has_media
        || matches!(
            state.phase,
            PlaybackPhase::Idle | PlaybackPhase::Loading | PlaybackPhase::Error
        )
    {
        app.set_player_video_ready(false);
    }
    app.set_player_playing(state.phase == PlaybackPhase::Playing);
    app.set_player_loading(state.phase == PlaybackPhase::Loading);
    app.set_player_buffering(state.phase == PlaybackPhase::Buffering);
    app.set_player_seeking(state.seeking);
    app.set_player_muted(state.muted);
    app.set_player_position(state.position_seconds.unwrap_or(0.0) as f32);
    app.set_player_duration(state.duration_seconds.unwrap_or(0.0) as f32);
    // mpv's cache percentage is not a buffered-duration value. Until the
    // adapter observes a real demuxer cache duration, keep the secondary
    // timeline hidden instead of presenting the playhead as buffered media.
    app.set_player_buffered(0.0);
    app.set_player_buffering_percent(state.buffering_percent.unwrap_or(-1.0) as f32);
    app.set_player_volume(state.volume as f32);
    app.set_player_speed(state.speed as f32);
    app.set_player_elapsed_label(
        state
            .position_seconds
            .map(playback_time_label)
            .unwrap_or_else(|| "00:00".into())
            .into(),
    );
    app.set_player_duration_label(
        state
            .duration_seconds
            .map(playback_time_label)
            .unwrap_or_else(|| "--:--".into())
            .into(),
    );
    app.set_player_error(state.last_error.clone().unwrap_or_default().into());
    if let Some(title) = state
        .media_title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
    {
        app.set_player_title(title.into());
    }

    let to_track_ref = |track: &playback::MediaTrack| {
        i32::try_from(track.id).ok().map(|id| PlayerTrackRef {
            id,
            label: track
                .title
                .clone()
                .or_else(|| track.language.clone())
                .unwrap_or_else(|| format!("Track {}", track.id))
                .into(),
            language: track.language.clone().unwrap_or_default().into(),
            codec: track.codec.clone().unwrap_or_default().into(),
            selected: track.selected,
            external: track.external,
            forced: track.forced,
        })
    };
    let audio = state
        .tracks
        .iter()
        .filter(|track| matches!(track.kind, TrackKind::Audio))
        .filter_map(to_track_ref)
        .collect::<Vec<_>>();
    let subtitles = state
        .tracks
        .iter()
        .filter(|track| matches!(track.kind, TrackKind::Subtitle))
        .filter_map(to_track_ref)
        .collect::<Vec<_>>();
    app.set_player_audio_tracks(slint::ModelRc::from(Rc::new(slint::VecModel::from(audio))));
    app.set_player_subtitle_tracks(slint::ModelRc::from(Rc::new(slint::VecModel::from(
        subtitles,
    ))));
}

fn persist_player_progress(state: &playback::PlayerState, force: bool) {
    let (Some(position), Some(duration)) = (state.position_seconds, state.duration_seconds) else {
        return;
    };
    if !(position.is_finite()
        && duration.is_finite()
        && position >= 0.0
        && duration > 0.0
        && position <= duration)
    {
        return;
    }

    let due = LAST_PROGRESS_SAVE.with(|last_save| {
        let last_save = last_save.borrow();
        force
            || last_save
                .as_ref()
                .is_none_or(|saved| saved.elapsed() >= std::time::Duration::from_secs(5))
    });
    if !due {
        return;
    }
    let Some(key) = ACTIVE_PLAYBACK_KEY.with(|key| key.borrow().clone()) else {
        return;
    };
    let metadata = ACTIVE_PLAYBACK_METADATA.with(|metadata| metadata.borrow().clone());
    let completed = state.phase == playback::PlaybackPhase::Ended || position / duration >= 0.98;
    let result = DB_HANDLE.with(|db| {
        let db = db.borrow();
        let Some(db) = db.as_ref() else {
            return Ok(None);
        };
        ankai_core::playback_progress::save_with_metadata(
            db,
            &key,
            if completed { duration } else { position },
            duration,
            completed,
            &metadata,
        )
        .map(Some)
    });
    match result {
        Ok(Some(_)) => LAST_PROGRESS_SAVE.with(|last_save| {
            *last_save.borrow_mut() = Some(std::time::Instant::now());
        }),
        Ok(None) => {}
        Err(error) => eprintln!("ankai-client: failed to save playback progress: {error}"),
    }
}

/// A time-of-day-appropriate greeting prefix for the Home dashboard's
/// header ("good morning"/"good afternoon"/"good evening"), from this
/// device's real local wall-clock time via `chrono::Local` — computed once
/// at startup, not kept live while the app stays open (matches
/// `display-name-initial`'s "startup snapshot" posture). The boundaries
/// (5/12/18) are an ordinary, non-authoritative convention, not anything
/// this module claims is precise.
fn time_of_day_greeting() -> &'static str {
    use chrono::Timelike;
    match chrono::Local::now().hour() {
        5..=11 => "good morning",
        12..=17 => "good afternoon",
        _ => "good evening",
    }
}

/// Opens (creating if necessary) ANKAI's local encrypted database in this
/// platform's standard app-data directory, keyed by the device-local
/// passphrase from `ankai_core::keychain` (OS secure storage, per
/// ADR-0004). A working local DB is required, but failure is returned as a
/// normal platform error rather than panicking during process startup.
fn open_local_db() -> Result<ankai_core::db::Db, slint::PlatformError> {
    let dirs = directories::ProjectDirs::from("com", "ankai", "ANKAI").ok_or_else(|| {
        slint::PlatformError::Other(
            "couldn't locate an application data directory for this user".into(),
        )
    })?;
    std::fs::create_dir_all(dirs.data_dir()).map_err(|error| {
        slint::PlatformError::Other(format!(
            "couldn't create the ANKAI application data directory: {error}"
        ))
    })?;

    let passphrase = ankai_core::keychain::device_db_passphrase().map_err(|error| {
        slint::PlatformError::Other(format!(
            "couldn't access the device database key in secure storage: {error}"
        ))
    })?;

    ankai_core::db::Db::open(dirs.data_dir().join("ankai.sqlite"), &passphrase).map_err(|error| {
        slint::PlatformError::Other(format!(
            "couldn't open the encrypted local database: {error}"
        ))
    })
}

/// Builds the optional social-networking stack. Catalogs, metadata, cover
/// images and playback use the shared runtime directly and do not depend on
/// this succeeding.
fn initialize_social_services(
    db: &Rc<ankai_core::db::Db>,
    runtime: &tokio::runtime::Runtime,
) -> Result<SocialServices, String> {
    let mls_provider = Rc::new(
        ankai_core::mls_provider::AnkaiMlsProvider::load(db)
            .map_err(|error| format!("couldn't load encrypted messaging state: {error}"))?,
    );
    let device = ankai_core::identity::load_or_create_device(db, &mls_provider)
        .map_err(|error| format!("couldn't load this device's social identity: {error}"))?;
    mls_provider
        .flush(db)
        .map_err(|error| format!("couldn't persist this device's social identity: {error}"))?;

    let node = std::sync::Arc::new(
        runtime
            .block_on(ankai_core::p2p::P2pNode::bind())
            .map_err(|error| format!("couldn't bind the local peer-to-peer endpoint: {error}"))?,
    );
    let own_key_package = ankai_core::identity::create_key_package(&device, &mls_provider)
        .map_err(|error| format!("couldn't build this device's secure invite: {error}"))?
        .key_package
        .ok_or_else(|| "secure invite creation returned no key package".to_string())?;
    mls_provider
        .flush(db)
        .map_err(|error| format!("couldn't persist this device's secure invite: {error}"))?;
    let own_invite = ankai_core::messaging::PeerInvite {
        addr: node.addr(),
        key_package: own_key_package,
    };
    let own_invite_text = ankai_core::messaging::format_peer_invite(&own_invite)
        .map_err(|error| format!("couldn't encode this device's secure invite: {error}"))?;

    Ok(SocialServices {
        mls_provider,
        device,
        node,
        own_invite,
        own_invite_text,
    })
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

fn open_external_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };

    command.arg(url).spawn().map(|_| ())
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
    let generation = WATCHLIST_GENERATION.with(RequestGeneration::issue);
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
            if !WATCHLIST_GENERATION.with(|state| state.is_current(generation)) {
                return;
            }
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

/// Recomputes the Messages pane's conversation list/switcher
/// (`ankai_core::messaging::list_conversations`) into the `ConversationRef`
/// rows `app.slint`'s switcher renders. Called at startup and after every
/// send/receive that could have created a conversation or changed its most
/// recent message. There is no friendlier "known name" for a conversation's
/// peer yet (no directory-backed identity binding exists at this layer —
/// see `messaging.rs`'s doc comment), so `display_label` is the same
/// truncated peer id `short_peer_id` already produces for the flat history
/// view.
fn refresh_conversations(app: &AppWindow, db: &ankai_core::db::Db) {
    let conversations = ankai_core::messaging::list_conversations(db).unwrap_or_else(|error| {
        eprintln!("ankai-client: failed to list conversations: {error}");
        Vec::new()
    });

    let rows: Vec<ConversationRef> = conversations
        .into_iter()
        .map(|c| {
            let label = short_peer_id(&c.peer_id).to_string();
            let preview = match (c.last_direction, c.last_message) {
                (Some(ankai_core::messaging::Direction::Sent), Some(text)) => {
                    format!("you: {text}")
                }
                (Some(ankai_core::messaging::Direction::Received), Some(text)) => {
                    format!("{label}: {text}")
                }
                _ => "No messages yet".to_string(),
            };
            ConversationRef {
                peer_id: c.peer_id.into(),
                display_label: label.into(),
                preview: preview.into(),
                timestamp: c.last_activity_at.into(),
            }
        })
        .collect();

    app.set_conversations(slint::ModelRc::from(Rc::new(slint::VecModel::from(rows))));
}

/// Builds the "most recent first, sender: text" display lines for exactly
/// one conversation, filtered client-side from this device's full flat
/// history (`ankai_core::messaging::list_messages`) by peer id — see that
/// module's doc comment on why `list_messages` itself stays unscoped rather
/// than this file reaching around it with a second near-identical query.
fn conversation_message_lines(db: &ankai_core::db::Db, peer_id: &str) -> Vec<slint::SharedString> {
    let history = ankai_core::messaging::list_messages(db).unwrap_or_else(|error| {
        eprintln!("ankai-client: failed to load message history: {error}");
        Vec::new()
    });

    let mut lines: Vec<slint::SharedString> = Vec::new();
    for stored in history.into_iter().filter(|m| m.peer_id == peer_id) {
        let sender = match stored.direction {
            ankai_core::messaging::Direction::Sent => "you".to_string(),
            ankai_core::messaging::Direction::Received => {
                short_peer_id(&stored.peer_id).to_string()
            }
        };
        lines.insert(0, format!("{sender}: {}", stored.content).into());
    }
    lines
}

/// Selects `peer_id` as the Messages pane's active conversation: sets the
/// switcher's selection state and reloads `message-log` scoped to that
/// peer alone. Called on a real `select-conversation` click, and also
/// after a successful send (see `on_send_message` below) so starting a
/// brand-new conversation focuses it immediately instead of leaving the
/// switcher on whatever was selected before — matching the human
/// directive's "create-or-select-and-focus a conversation" requirement.
fn select_conversation(app: &AppWindow, db: &ankai_core::db::Db, peer_id: &str) {
    app.set_selected_conversation_peer_id(peer_id.into());
    app.set_selected_conversation_label(short_peer_id(peer_id).into());
    let lines = conversation_message_lines(db, peer_id);
    app.set_message_log(slint::ModelRc::from(Rc::new(slint::VecModel::from(lines))));
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
    let generation = HOME_ANIME_GENERATION.with(RequestGeneration::issue);
    if let Some(app) = app_weak.upgrade() {
        app.set_home_anime_state("loading".into());
        app.set_anime_status("".into());
        app.set_popular_anime_state("loading".into());
        app.set_popular_anime_status("".into());
    }
    let app_weak_trending = app_weak.clone();
    let handle_trending = handle.clone();
    handle.spawn(async move {
        let result = ankai_core::anime::trending_anime(10).await;
        let _ = slint::invoke_from_event_loop(move || {
            if !HOME_ANIME_GENERATION.with(|state| state.is_current(generation)) {
                return;
            }
            let Some(app) = app_weak_trending.upgrade() else {
                return;
            };
            match result {
                Ok(list) => {
                    app.set_anime_status("".into());
                    app.set_home_anime_state("ready".into());
                    // Cover art (see `AnimeRef`'s doc comment) loads
                    // asynchronously after the text-only cards are already
                    // on screen: collect (id, url) pairs before `list` is
                    // consumed by `anime_to_ref` below, then kick off one
                    // real fetch per entry that actually has a cover URL.
                    let covers: Vec<(i32, String)> = list
                        .iter()
                        .filter_map(|a| a.cover_image_url.clone().map(|url| (a.id as i32, url)))
                        .collect();
                    // Kept alongside the trimmed AnimeRef model below so the
                    // Home dashboard's hero-card "Watch Now" button
                    // (on_watch_now) can look a real full AnimeSummary back
                    // up by id later — see HOME_TRENDING_ANIME's doc
                    // comment.
                    HOME_TRENDING_ANIME.with(|store| *store.borrow_mut() = list.clone());
                    let refs: Vec<AnimeRef> = list.into_iter().map(anime_to_ref).collect();
                    app.set_trending_anime(slint::ModelRc::from(Rc::new(slint::VecModel::from(
                        refs,
                    ))));
                    let hero_id = HOME_TRENDING_ANIME
                        .with(|items| items.borrow().first().map(|anime| anime.id));
                    let hero_watchlisted = hero_id.is_some_and(|id| {
                        DB_HANDLE.with(|db| {
                            db.borrow()
                                .as_ref()
                                .and_then(|db| ankai_core::anime::get_watchlist_entry(db, id).ok())
                                .flatten()
                                .is_some()
                        })
                    });
                    app.set_hero_watchlisted(hero_watchlisted);
                    for (id, url) in covers {
                        let app_weak = app_weak_trending.clone();
                        images::load_cover_image(
                            url,
                            handle_trending.clone(),
                            app_weak,
                            move |app, image| {
                                if !HOME_ANIME_GENERATION.with(|state| state.is_current(generation))
                                {
                                    return;
                                }
                                set_anime_cover(app.get_trending_anime(), id, image);
                            },
                        );
                    }
                }
                Err(err) => {
                    eprintln!("ankai-client: failed to load trending anime: {err}");
                    app.set_home_anime_state("error".into());
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
            if !HOME_ANIME_GENERATION.with(|state| state.is_current(generation)) {
                return;
            }
            let Some(app) = app_weak_popular.upgrade() else {
                return;
            };
            match result {
                Ok(list) => {
                    app.set_popular_anime_state("ready".into());
                    app.set_popular_anime_status("".into());
                    let covers: Vec<(i32, String)> = list
                        .iter()
                        .filter_map(|a| a.cover_image_url.clone().map(|url| (a.id as i32, url)))
                        .collect();
                    HOME_POPULAR_ANIME.with(|store| *store.borrow_mut() = list.clone());
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
                                if !HOME_ANIME_GENERATION.with(|state| state.is_current(generation))
                                {
                                    return;
                                }
                                set_anime_cover(app.get_popular_anime(), id, image);
                            },
                        );
                    }
                }
                Err(err) => {
                    eprintln!("ankai-client: failed to load popular anime: {err}");
                    app.set_popular_anime_state("error".into());
                    app.set_popular_anime_status(
                        format!("Couldn't load popular anime: {err}").into(),
                    );
                }
            }
        });
    });
}

fn set_letterboxd_cover(
    model: slint::ModelRc<LetterboxdEntryRef>,
    index: usize,
    link: &str,
    image: slint::Image,
) {
    let Some(model) = model
        .as_any()
        .downcast_ref::<slint::VecModel<LetterboxdEntryRef>>()
    else {
        return;
    };
    if let Some(mut row) = model.row_data(index) {
        if row.link.as_str() == link {
            row.poster = image;
            row.has_poster = true;
            model.set_row_data(index, row);
        }
    }
}

fn spawn_letterboxd_refresh(
    handle: tokio::runtime::Handle,
    app_weak: slint::Weak<AppWindow>,
    username: String,
) {
    let generation = LETTERBOXD_GENERATION.with(RequestGeneration::issue);
    if let Some(app) = app_weak.upgrade() {
        app.set_letterboxd_state("loading".into());
        app.set_letterboxd_status("Loading public diary…".into());
    }
    let image_handle = handle.clone();
    handle.spawn(async move {
        let result = ankai_core::letterboxd::member_feed(username.trim()).await;
        let _ = slint::invoke_from_event_loop(move || {
            if !LETTERBOXD_GENERATION.with(|state| state.is_current(generation)) {
                return;
            }
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            match result {
                Ok(entries) => {
                    let posters = entries
                        .iter()
                        .take(8)
                        .enumerate()
                        .filter_map(|(index, entry)| {
                            entry
                                .poster_url
                                .clone()
                                .map(|poster| (index, entry.link.clone(), poster))
                        })
                        .collect::<Vec<_>>();
                    let rows = entries
                        .into_iter()
                        .take(8)
                        .map(|entry| LetterboxdEntryRef {
                            title: entry.film_title.into(),
                            meta: [
                                entry.film_year.map(|year| year.to_string()),
                                entry.published_at,
                            ]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(" · ")
                            .into(),
                            review: entry
                                .review_excerpt
                                .unwrap_or_else(|| "Diary entry without a written review.".into())
                                .into(),
                            rating: entry
                                .member_rating
                                .map(|rating| format!("★ {rating:.1} / 5"))
                                .unwrap_or_else(|| "Not rated".into())
                                .into(),
                            poster: slint::Image::default(),
                            has_poster: false,
                            link: entry.link.into(),
                        })
                        .collect::<Vec<_>>();
                    app.set_letterboxd_entries(slint::ModelRc::from(Rc::new(
                        slint::VecModel::from(rows),
                    )));
                    app.set_letterboxd_state("ready".into());
                    app.set_letterboxd_status("Public member RSS · read only".into());
                    for (index, link, poster) in posters {
                        let app_weak = app.as_weak();
                        images::load_cover_image(
                            poster,
                            image_handle.clone(),
                            app_weak,
                            move |app, image| {
                                if !LETTERBOXD_GENERATION.with(|state| state.is_current(generation))
                                {
                                    return;
                                }
                                set_letterboxd_cover(
                                    app.get_letterboxd_entries(),
                                    index,
                                    &link,
                                    image,
                                );
                            },
                        );
                    }
                }
                Err(error) => {
                    app.set_letterboxd_state("error".into());
                    app.set_letterboxd_status(error.to_string().into());
                }
            }
        });
    });
}

fn set_resume_cover(model: slint::ModelRc<ResumeRef>, index: usize, image: slint::Image) {
    let Some(model) = model.as_any().downcast_ref::<slint::VecModel<ResumeRef>>() else {
        return;
    };
    if let Some(mut row) = model.row_data(index) {
        row.poster = image;
        row.has_poster = true;
        model.set_row_data(index, row);
    }
}

fn refresh_resume_entries(
    app: &AppWindow,
    db: &ankai_core::db::Db,
    handle: &tokio::runtime::Handle,
) {
    let generation = RESUME_GENERATION.with(RequestGeneration::issue);
    let entries =
        ankai_core::playback_progress::list_resumable_recent(db, 8).unwrap_or_else(|error| {
            eprintln!("ankai-client: failed to load Continue Watching: {error}");
            Vec::new()
        });
    let cards = entries
        .iter()
        .map(|entry| {
            let title = entry
                .title
                .clone()
                .unwrap_or_else(|| entry.key.media_id.clone());
            let subtitle = entry
                .key
                .episode_id
                .as_deref()
                .map(|episode| format!("Episode · {episode}"))
                .unwrap_or_else(|| "Movie".into());
            ResumeRef {
                title: title.into(),
                subtitle: subtitle.into(),
                progress_label: format!(
                    "{} / {}",
                    playback_time_label(entry.position_seconds),
                    playback_time_label(entry.duration_seconds)
                )
                .into(),
                progress: (entry.position_seconds / entry.duration_seconds).clamp(0.0, 1.0) as f32,
                poster: slint::Image::default(),
                has_poster: false,
            }
        })
        .collect::<Vec<_>>();
    RESUME_PROGRESS.with(|store| *store.borrow_mut() = entries.clone());
    app.set_continue_watching(slint::ModelRc::from(Rc::new(slint::VecModel::from(cards))));

    for (index, entry) in entries.into_iter().enumerate() {
        let Some(poster) = entry.poster_url.clone() else {
            continue;
        };
        let expected_key = entry.key;
        let expected_poster = poster.clone();
        let app_weak = app.as_weak();
        images::load_cover_image(poster, handle.clone(), app_weak, move |app, image| {
            if !RESUME_GENERATION.with(|state| state.is_current(generation)) {
                return;
            }
            let still_matches = RESUME_PROGRESS.with(|entries| {
                entries.borrow().get(index).is_some_and(|entry| {
                    entry.key == expected_key
                        && entry.poster_url.as_deref() == Some(expected_poster.as_str())
                })
            });
            if still_matches {
                set_resume_cover(app.get_continue_watching(), index, image);
            }
        });
    }
}

/// Reloads the Home dashboard's "Hot Discussions" model from
/// `core::forum_posts::list_recent_across_communities` — a plain, synchronous
/// local-DB read (no network involved, unlike the anime functions above),
/// same shape as `refresh_top8`. Capped at 12 entries: a dashboard widget,
/// not infinite scroll.
fn refresh_recent_posts(app: &AppWindow, db: &ankai_core::db::Db) {
    let recent = match ankai_core::forum_posts::list_recent_across_communities(db, 12) {
        Ok(recent) => {
            app.set_discussions_home_state("ready".into());
            app.set_discussions_home_status("".into());
            recent
        }
        Err(err) => {
            eprintln!("ankai-client: failed to load recent posts for Home: {err}");
            app.set_discussions_home_state("error".into());
            app.set_discussions_home_status(
                format!("Couldn't load local discussions: {err}").into(),
            );
            Vec::new()
        }
    };
    let refs: Vec<RecentPostRef> = recent
        .into_iter()
        .map(|p| RecentPostRef {
            community_id: p.community_id.into(),
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
    let friends = match ankai_core::friends::list_friends(db) {
        Ok(friends) => {
            app.set_friends_home_state("ready".into());
            app.set_friends_home_status("".into());
            friends
        }
        Err(err) => {
            eprintln!("ankai-client: failed to load friends for Home: {err}");
            app.set_friends_home_state("error".into());
            app.set_friends_home_status(format!("Couldn't load local friends: {err}").into());
            Vec::new()
        }
    };
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

/// The local `settings` table key a real Last.fm username is persisted
/// under (see `core::db`'s `settings` table) — same shape as
/// `"display_name"`, distinct from `"directory_username"` (that one is an
/// ANKAI directory-server identity, this one is a Last.fm account name).
const LASTFM_USERNAME_SETTING_KEY: &str = "lastfm_username";
const LETTERBOXD_USERNAME_SETTING_KEY: &str = "letterboxd_username";

/// Fetches `username`'s real Last.fm now-playing state
/// (`ankai_core::lastfm::current_now_playing`) and pushes exactly one of the
/// four real states into My Page's Now Playing widget — see
/// `ui/app.slint`'s `now-playing-state` doc comment for what each of
/// "idle"/"error"/"playing" means (this function never sets
/// "not-configured"; that's the caller's job when there's no username at
/// all, since this function always requires one). Spawned on `handle`, same
/// "don't block the UI thread on a real third-party network call" reasoning
/// as the anime and Letterboxd refresh paths.
fn spawn_lastfm_refresh(
    handle: tokio::runtime::Handle,
    app_weak: slint::Weak<AppWindow>,
    username: String,
) {
    handle.spawn(async move {
        let result = ankai_core::lastfm::current_now_playing(&username).await;
        let _ = slint::invoke_from_event_loop(move || {
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            match result {
                Ok(Some(now_playing)) => {
                    app.set_now_playing_state("playing".into());
                    app.set_now_playing_artist(now_playing.artist.into());
                    app.set_now_playing_track(now_playing.track.into());
                    app.set_now_playing_album(now_playing.album.unwrap_or_default().into());
                    app.set_now_playing_error("".into());
                }
                Ok(None) => {
                    app.set_now_playing_state("idle".into());
                    app.set_now_playing_artist("".into());
                    app.set_now_playing_track("".into());
                    app.set_now_playing_album("".into());
                    app.set_now_playing_error("".into());
                }
                Err(err) => {
                    eprintln!("ankai-client: failed to load Last.fm now-playing: {err}");
                    app.set_now_playing_state("error".into());
                    app.set_now_playing_artist("".into());
                    app.set_now_playing_track("".into());
                    app.set_now_playing_album("".into());
                    app.set_now_playing_error(err.to_string().into());
                }
            }
        });
    });
}

fn main() -> Result<(), slint::PlatformError> {
    // libmpv's render API and Slint share this OpenGL context. FemtoVG is the
    // stable OpenGL renderer in Slint 1.17 and exposes it through the rendering
    // notifier; Skia may select Metal on macOS and cannot be shared with mpv.
    if let Err(err) = slint::BackendSelector::new()
        .renderer_name("femtovg".into())
        .select()
    {
        eprintln!(
            "ankai-client: FemtoVG renderer unavailable ({err}); falling back to Slint's default backend/renderer selection."
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

    let db = Rc::new(open_local_db()?);
    DB_HANDLE.with(|handle| *handle.borrow_mut() = Some(db.clone()));

    let app = AppWindow::new()?;

    {
        let app_weak = app.as_weak();
        app.window()
            .set_rendering_notifier(move |state, graphics_api| {
                let Some(app) = app_weak.upgrade() else {
                    return;
                };
                match (state, graphics_api) {
                    (
                        slint::RenderingState::RenderingSetup,
                        slint::GraphicsAPI::NativeOpenGL { .. },
                    ) => {
                        let result =
                            playback::BoundedVideoSurface::new(graphics_api).and_then(|surface| {
                                let mut player = playback::Player::new()?;
                                surface.setup_player(&mut player)?;
                                if let Ok(url) = std::env::var("ANKAI_PLAYBACK_TEST_URL") {
                                    player.load(&url)?;
                                    app.set_player_title("Embedded playback test".into());
                                    app.set_player_active(true);
                                }
                                VIDEO_PLAYER.with(|slot| *slot.borrow_mut() = Some(player));
                                VIDEO_SURFACE.with(|slot| *slot.borrow_mut() = Some(surface));
                                Ok(())
                            });
                        if let Err(error) = result {
                            app.set_player_error(error.to_string().into());
                            app.set_stremio_status(format!("Player unavailable: {error}").into());
                        }
                    }
                    (
                        slint::RenderingState::BeforeRendering,
                        slint::GraphicsAPI::NativeOpenGL { .. },
                    ) if app.get_player_active() => {
                        let result = VIDEO_PLAYER.with(|player_slot| {
                            VIDEO_SURFACE.with(|surface_slot| {
                                let mut player_slot = player_slot.borrow_mut();
                                let mut surface_slot = surface_slot.borrow_mut();
                                let Some(player) = player_slot.as_mut() else {
                                    return Ok::<_, playback::PlayerError>(false);
                                };
                                if app.get_player_bounded_mode() {
                                    let Some(surface) = surface_slot.as_mut() else {
                                        return Err(playback::PlayerError::OpenGl(
                                            "bounded video surface is unavailable".into(),
                                        ));
                                    };
                                    match surface.ensure_size(
                                        app.get_player_video_surface_width(),
                                        app.get_player_video_surface_height(),
                                        app.window().scale_factor(),
                                    )? {
                                        playback::VideoSurfaceUpdate::Unchanged => {}
                                        playback::VideoSurfaceUpdate::Replace { image, .. } => {
                                            app.set_player_video_frame(image);
                                            app.set_player_video_frame_ready(true);
                                        }
                                        playback::VideoSurfaceUpdate::Clear => {
                                            app.set_player_video_frame(slint::Image::default());
                                            app.set_player_video_frame_ready(false);
                                        }
                                    }
                                    surface.render_player(player)?;
                                } else {
                                    let size = app.window().size();
                                    player.render(size.width as i32, size.height as i32)?;
                                }
                                Ok(player.state().has_media)
                            })
                        });
                        match result {
                            Ok(true) => app.set_player_video_ready(true),
                            Ok(false) => {}
                            Err(error) => {
                                app.set_player_loading(false);
                                app.set_player_error(error.to_string().into());
                            }
                        }
                    }
                    (
                        slint::RenderingState::AfterRendering,
                        slint::GraphicsAPI::NativeOpenGL { .. },
                    ) => {
                        VIDEO_SURFACE.with(|slot| {
                            if let Some(surface) = slot.borrow_mut().as_mut() {
                                surface.finish_frame();
                            }
                        });
                    }
                    (slint::RenderingState::RenderingTeardown, _) => {
                        app.set_player_video_frame(slint::Image::default());
                        app.set_player_video_frame_ready(false);
                        VIDEO_SURFACE.with(|slot| {
                            let mut slot = slot.borrow_mut();
                            if let Some(surface) = slot.as_mut() {
                                surface.teardown();
                            }
                            *slot = None;
                        });
                        VIDEO_PLAYER.with(|slot| *slot.borrow_mut() = None);
                    }
                    _ => {}
                }
            })
            .map_err(|error| {
                slint::PlatformError::Other(format!("failed to install video renderer: {error}"))
            })?;
    }
    let saved_display_name = db
        .get_setting("display_name")
        .unwrap_or_else(|error| {
            eprintln!("ankai-client: failed to read display_name setting: {error}");
            None
        })
        .unwrap_or_default();
    app.set_display_name_initial(initial_letter(&saved_display_name).into());
    app.set_display_name(saved_display_name.into());
    let reduced_motion = db
        .get_setting("reduced_motion")
        .unwrap_or_else(|error| {
            eprintln!("ankai-client: failed to read reduced_motion setting: {error}");
            None
        })
        .is_some_and(|value| value == "true");
    app.set_reduced_motion(reduced_motion);
    app.set_time_of_day_greeting(time_of_day_greeting().into());

    let db_for_save = db.clone();
    let app_weak_for_name = app.as_weak();
    app.on_save_display_name(move |name| {
        let result = db_for_save.set_setting("display_name", &name);
        if let Some(app) = app_weak_for_name.upgrade() {
            match result {
                Ok(_) => {
                    app.set_display_name_initial(initial_letter(&name).into());
                    app.set_shell_notice("Display name saved.".into());
                }
                Err(error) => {
                    eprintln!("ankai-client: failed to save display name: {error}");
                    app.set_shell_notice(format!("Couldn't save display name: {error}").into());
                }
            }
        }
    });

    {
        let db = db.clone();
        let app_weak = app.as_weak();
        app.on_save_reduced_motion(move |enabled| {
            if let Err(error) =
                db.set_setting("reduced_motion", if enabled { "true" } else { "false" })
            {
                eprintln!("ankai-client: failed to save reduced-motion preference: {error}");
                if let Some(app) = app_weak.upgrade() {
                    app.set_shell_notice("Couldn't save the motion preference.".into());
                }
            } else if let Some(app) = app_weak.upgrade() {
                app.set_shell_notice(
                    if enabled {
                        "Reduced motion enabled."
                    } else {
                        "Reduced motion disabled."
                    }
                    .into(),
                );
            }
        });
    }

    let communities = ankai_core::communities::list(&db).unwrap_or_else(|error| {
        eprintln!("ankai-client: failed to list communities: {error}");
        Vec::new()
    });
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
            if let Some(app) = app_weak.upgrade() {
                app.set_shell_notice("Enter a community name first.".into());
            }
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
                    app.set_shell_notice(format!("Created community: {name}").into());
                }
            }
            Err(error) => {
                eprintln!("ankai-client: failed to create community: {error}");
                if let Some(app) = app_weak.upgrade() {
                    app.set_shell_notice(format!("Couldn't create community: {error}").into());
                }
            }
        }
    });

    // Conversation switcher (see refresh_conversations/select_conversation
    // above): reading a conversation's messages is a plain local DB read,
    // so this works even when social networking is disabled for the
    // session (same "local data stays readable" precedent as
    // refresh_friends above) — only actually *sending* a reply needs a live
    // P2P node.
    let db_for_select_conversation = db.clone();
    let app_weak_for_select_conversation = app.as_weak();
    app.on_select_conversation(move |peer_id| {
        if let Some(app) = app_weak_for_select_conversation.upgrade() {
            select_conversation(&app, &db_for_select_conversation, &peer_id);
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
                if let Some(app) = app_weak_for_open_community.upgrade() {
                    app.set_shell_notice(format!("Couldn't open that community: {err}").into());
                }
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
            if let Some(app) = app_weak_for_create_post.upgrade() {
                app.set_shell_notice("Write something before posting.".into());
            }
            return;
        }
        match ankai_core::forum_posts::create_post(&db_for_create_post, &community_id, content) {
            Ok(post) => {
                community_posts_model.push(post.content.into());
                if let Some(app) = app_weak_for_create_post.upgrade() {
                    app.set_new_post_content("".into());
                    app.set_shell_notice("Post published on this device.".into());
                }
            }
            Err(err) => {
                eprintln!("ankai-client: failed to create post in community {community_id}: {err}");
                if let Some(app) = app_weak_for_create_post.upgrade() {
                    app.set_shell_notice(format!("Couldn't publish post: {err}").into());
                }
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
        if let Some(app) = app_weak_for_feature.upgrade() {
            match ankai_core::top8::add_to_top8(&db_for_feature, &community_id) {
                Ok(_) => {
                    refresh_top8(&app, &db_for_feature);
                    app.set_shell_notice("Added community to Top 8.".into());
                }
                Err(error) => {
                    eprintln!("ankai-client: failed to feature community: {error}");
                    app.set_shell_notice(format!("Couldn't update Top 8: {error}").into());
                }
            }
        }
    });

    let db_for_unfeature = db.clone();
    let app_weak_for_unfeature = app.as_weak();
    app.on_unfeature_community(move |community_id| {
        if let Some(app) = app_weak_for_unfeature.upgrade() {
            match ankai_core::top8::remove_from_top8(&db_for_unfeature, &community_id) {
                Ok(_) => {
                    refresh_top8(&app, &db_for_unfeature);
                    app.set_shell_notice("Removed community from Top 8.".into());
                }
                Err(error) => {
                    eprintln!("ankai-client: failed to unfeature community: {error}");
                    app.set_shell_notice(format!("Couldn't update Top 8: {error}").into());
                }
            }
        }
    });

    let db_for_move_up = db.clone();
    let app_weak_for_move_up = app.as_weak();
    app.on_move_featured_community_up(move |community_id| {
        if let Some(app) = app_weak_for_move_up.upgrade() {
            match ankai_core::top8::move_up(&db_for_move_up, &community_id) {
                Ok(_) => refresh_top8(&app, &db_for_move_up),
                Err(error) => {
                    eprintln!("ankai-client: failed to move featured community up: {error}");
                    app.set_shell_notice(format!("Couldn't reorder Top 8: {error}").into());
                }
            }
        }
    });

    let db_for_move_down = db.clone();
    let app_weak_for_move_down = app.as_weak();
    app.on_move_featured_community_down(move |community_id| {
        if let Some(app) = app_weak_for_move_down.upgrade() {
            match ankai_core::top8::move_down(&db_for_move_down, &community_id) {
                Ok(_) => refresh_top8(&app, &db_for_move_down),
                Err(error) => {
                    eprintln!("ankai-client: failed to move featured community down: {error}");
                    app.set_shell_notice(format!("Couldn't reorder Top 8: {error}").into());
                }
            }
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
    let hangouts = match ankai_core::hangouts::list(&db) {
        Ok(hangouts) => {
            app.set_hangouts_home_state("ready".into());
            app.set_hangouts_home_status("".into());
            hangouts
        }
        Err(error) => {
            eprintln!("ankai-client: failed to list hangouts: {error}");
            app.set_hangouts_home_state("error".into());
            app.set_hangouts_home_status(format!("Couldn't load local Hangouts: {error}").into());
            Vec::new()
        }
    };
    let hangout_names: Vec<slint::SharedString> =
        hangouts.into_iter().map(|h| h.name.into()).collect();
    let hangout_model = std::rc::Rc::new(slint::VecModel::from(hangout_names));
    app.set_hangout_names(slint::ModelRc::from(hangout_model.clone()));

    let db_for_hangouts = db.clone();
    let app_weak_for_hangouts = app.as_weak();
    app.on_create_hangout(move |name| {
        let name = name.trim();
        if name.is_empty() {
            if let Some(app) = app_weak_for_hangouts.upgrade() {
                app.set_shell_notice("Enter a Hangout name first.".into());
            }
            return;
        }
        match ankai_core::hangouts::create(&db_for_hangouts, name) {
            Ok(hangout) => {
                let hangout_name = hangout.name;
                hangout_model.push(hangout_name.clone().into());
                if let Some(app) = app_weak_for_hangouts.upgrade() {
                    app.set_new_hangout_name("".into());
                    app.set_selected_hangout_name(hangout_name.clone().into());
                    app.set_hangouts_home_state("ready".into());
                    app.set_hangouts_home_status("".into());
                    app.set_shell_notice(format!("Created Hangout: {hangout_name}").into());
                }
            }
            Err(error) => {
                eprintln!("ankai-client: failed to create hangout: {error}");
                if let Some(app) = app_weak_for_hangouts.upgrade() {
                    app.set_hangouts_home_state("error".into());
                    app.set_hangouts_home_status(error.to_string().into());
                    app.set_shell_notice(format!("Couldn't create Hangout: {error}").into());
                }
            }
        }
    });

    // A shared runtime drives every asynchronous feature, including Stremio
    // and cover art. Runtime construction is required; return a normal,
    // actionable startup error if the host cannot create it.
    let p2p_runtime = tokio::runtime::Runtime::new().map_err(|error| {
        slint::PlatformError::Other(format!(
            "couldn't start the background network runtime: {error}"
        ))
    })?;

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
    // Social initialization is optional. A damaged MLS blob, unavailable
    // socket, missing signing key, or invite error disables only peer/social
    // controls; browsing and playback keep using the runtime above.
    let social_services = match initialize_social_services(&db, &p2p_runtime) {
        Ok(services) => Some(services),
        Err(error) => {
            eprintln!("ankai-client: social networking unavailable: {error}");
            app.set_social_networking_enabled(false);
            app.set_social_networking_status(
                format!("Social networking is unavailable for this session. {error}").into(),
            );
            app.set_send_status("Messaging is disabled for this session.".into());
            app.set_directory_enabled(false);
            None
        }
    };
    if let Some(services) = social_services.as_ref() {
        app.set_account_id(services.device.account.0.clone().into());
        app.set_device_id(services.device.id.0.clone().into());
    }
    let social_node = social_services
        .as_ref()
        .map(|services| services.node.clone());

    // Now that a tokio runtime/handle exists, finish "Currently Watching"'s
    // setup (see the comment above this block) — real cover-art fetches
    // (client::images) get kicked off asynchronously on this same runtime.
    refresh_watchlist(&app, &db, p2p_runtime.handle());
    refresh_resume_entries(&app, &db, p2p_runtime.handle());

    // Local friend/message data remains readable while peer networking is
    // offline, so the disabled social view still represents persisted state.
    refresh_friends(&app, &db);
    // Real conversation list/switcher: load every conversation this device
    // has, most-recently-active first, and auto-select the most recent one
    // (if any exist) so the pane isn't blank on first render — matching
    // "assume the most relevant conversation, not that there's only one."
    refresh_conversations(&app, &db);
    if let Some(most_recent) = app.get_conversations().row_data(0) {
        select_conversation(&app, &db, &most_recent.peer_id);
    }

    if let Some(SocialServices {
        mls_provider,
        device,
        node: p2p_node,
        own_invite,
        own_invite_text,
    }) = social_services
    {
        app.set_social_networking_enabled(true);
        app.set_social_networking_status("Peer networking is ready.".into());
        app.set_own_peer_address(own_invite_text.into());
        MESSAGING_HANDLES.with(|handles| {
            *handles.borrow_mut() = Some(MessagingHandles {
                db: db.clone(),
                mls_provider: mls_provider.clone(),
            });
        });

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
        // Friends (ankai_core::friends): a real accepted-friends list plus real
        // pending *incoming* requests, sharing this same P2pNode with messaging
        // (see that module's "Wire dispatch" doc section). There is
        // deliberately no "send a friend request" UI wired here — sending would
        // need a way to pick a target device (the same look-up-by-Device-ID/
        // username machinery Messages already has), and this pass scopes that
        // out to keep the surface reviewable; accepting/declining an incoming
        // request (which core::friends fully supports) is what's wired.
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
                        app.set_shell_notice("Friend request accepted.".into());
                    }
                }
                Err(error) => {
                    eprintln!("ankai-client: failed to accept friend request: {error}");
                    if let Some(app) = app_weak_for_accept_friend.upgrade() {
                        app.set_shell_notice(
                            format!("Couldn't accept friend request: {error}").into(),
                        );
                    }
                }
            }
        });

        let db_for_decline_friend = db.clone();
        let app_weak_for_decline_friend = app.as_weak();
        app.on_decline_friend_request(move |device_id_text| {
            let device_id = ankai_core::identity::DeviceId(device_id_text.to_string());
            if let Some(app) = app_weak_for_decline_friend.upgrade() {
                match ankai_core::friends::decline_friend_request(
                    &db_for_decline_friend,
                    &device_id,
                ) {
                    Ok(()) => {
                        refresh_friends(&app, &db_for_decline_friend);
                        app.set_shell_notice("Friend request declined.".into());
                    }
                    Err(error) => {
                        eprintln!("ankai-client: failed to decline friend request: {error}");
                        app.set_shell_notice(
                            format!("Couldn't decline friend request: {error}").into(),
                        );
                    }
                }
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
                    if let Some(app) = app_weak_for_presence.upgrade() {
                        app.set_shell_notice(
                            format!("Couldn't refresh friend presence: {err}").into(),
                        );
                    }
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
        let directory_client = match directory_url.as_ref() {
            Some(url) => match ankai_core::identity::device_signer(&device, &mls_provider) {
                Ok(signer) => Some(std::sync::Arc::new(
                    ankai_directory_server::HttpDirectoryClient::new(
                        url.clone(),
                        device.id.clone(),
                        signer,
                    ),
                )),
                Err(error) => {
                    eprintln!(
                    "ankai-client: directory integration unavailable; couldn't load the device signer: {error}"
                );
                    None
                }
            },
            None => None,
        };

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
            .unwrap_or_else(|error| {
                eprintln!("ankai-client: failed to read directory_username setting: {error}");
                None
            })
            .unwrap_or_default();
        app.set_claimed_username(saved_username.into());

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
                                // The conversation list always reflects the
                                // new message's preview/timestamp. The
                                // visible message-log only reloads if this
                                // peer is (or becomes, when nothing was
                                // selected yet) the active conversation —
                                // an incoming message from someone else
                                // shouldn't yank the human's view away from
                                // whatever conversation they're reading.
                                refresh_conversations(&app, &handles.db);
                                let currently_selected = app.get_selected_conversation_peer_id();
                                if currently_selected.is_empty()
                                    || currently_selected.as_str() == peer_id
                                {
                                    select_conversation(&app, &handles.db, &peer_id);
                                }
                            }
                            // A Welcome establishing a new group — nothing
                            // user-visible yet, this device just joined. The
                            // conversation list still gains a row for it
                            // (with an honest "No messages yet" preview),
                            // so refresh it even though there's no message
                            // to show.
                            Ok(None) => {
                                refresh_conversations(&app, &handles.db);
                            }
                            Err(err) => {
                                eprintln!(
                                    "ankai-client: failed to process incoming message: {err}"
                                );
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
            // Computed up front so the success path below can create-or-
            // select-and-focus this peer's conversation in the switcher,
            // matching every other peer id this module already derives via
            // messaging::peer_id_for.
            let peer_id = ankai_core::messaging::peer_id_for(invite.addr.id);

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
                    result =
                        ankai_core::messaging::send_message(&node, addr.clone(), payload).await;
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
                            // db can't cross the tokio-thread spawn above as
                            // an Rc (not Send — see this file's
                            // MessagingHandles doc comment for the same
                            // constraint on the receive side), so it's
                            // fetched back from the UI-thread-only
                            // DB_HANDLE instead of being captured directly.
                            DB_HANDLE.with(|db| {
                                if let Some(db) = db.borrow().as_ref() {
                                    refresh_conversations(&app, db);
                                    select_conversation(&app, db, &peer_id);
                                }
                            });
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
    }

    // Stremio addon browser: manifest/catalog/meta/stream data is fetched on
    // the shared runtime and handed back to Slint on its event loop.
    {
        let runtime = p2p_runtime.handle().clone();
        let app_weak = app.as_weak();
        app.on_load_stremio_addon(move |url| {
            let url = url.trim().to_owned();
            let request_url = url.clone();
            let generation = issue_addon_load(&request_url);
            let search_generation = STREMIO_SEARCH_GENERATION.with(RequestGeneration::current);
            let app_weak = app_weak.clone();
            if let Some(app) = app_weak.upgrade() {
                app.set_stremio_status("Loading addon manifest…".into());
                app.set_stremio_loading(true);
            }
            let image_handle = runtime.clone();
            runtime.spawn(async move {
                let result = async {
                    let client = ankai_core::stremio::AddonClient::new(&url)?;
                    let manifest_url = client.manifest_url()?;
                    let manifest = client.manifest().await?;
                    // Catalog previews are no longer fetched here: Board rows
                    // (client/ui/board.slint, driven by rebuild_board_ui
                    // below) fetch each eligible catalog lazily, only once
                    // it's actually visible — see that function's doc
                    // comment. Fetching one catalog eagerly at install time
                    // would just be a redundant, immediately-discarded
                    // request for addons with more than one catalog.
                    Ok::<_, ankai_core::Error>((client, manifest_url, manifest))
                }
                .await;
                let _ = slint::invoke_from_event_loop(move || {
                    if !is_current_addon_load(&request_url, generation) {
                        return;
                    }
                    let Some(app) = app_weak.upgrade() else {
                        return;
                    };
                    let search_is_still_current =
                        STREMIO_SEARCH_GENERATION.with(|state| state.is_current(search_generation));
                    if search_is_still_current {
                        app.set_stremio_loading(false);
                    }
                    match result {
                        Ok((client, manifest_url, manifest)) => {
                            let addon_name = manifest.name.clone();
                            let persisted = DB_HANDLE.with(|db| {
                                let db = db.borrow();
                                let Some(db) = db.as_ref() else {
                                    return Err(ankai_core::Error::Db(
                                        "application database handle is unavailable".into(),
                                    ));
                                };
                                let exists = ankai_core::addons::list(db)?
                                    .iter()
                                    .any(|addon| addon.manifest_url == manifest_url);
                                if exists {
                                    ankai_core::addons::refresh_manifest(
                                        db,
                                        &manifest_url,
                                        &manifest,
                                    )
                                } else {
                                    ankai_core::addons::add(db, &manifest_url, &manifest)
                                }
                            });
                            match persisted {
                                Ok(_) => DB_HANDLE.with(|db| {
                                    if let Some(db) = db.borrow().as_ref() {
                                        refresh_addon_manager(&app, db, &image_handle);
                                    } else {
                                        app.set_stremio_status(
                                            "Loaded addon, but local registry is unavailable."
                                                .into(),
                                        );
                                    }
                                }),
                                Err(error) => app.set_stremio_status(
                                    format!("Loaded {addon_name}, but couldn't save it: {error}")
                                        .into(),
                                ),
                            }
                            STREMIO_ADDONS.with(|slot| {
                                let mut addons = slot.borrow_mut();
                                if let Some(index) = addons
                                    .iter()
                                    .position(|addon| addon.manifest_url == manifest_url)
                                {
                                    addons[index] = StremioAddon {
                                        manifest_url,
                                        client,
                                        name: addon_name.clone(),
                                        manifest,
                                    };
                                } else {
                                    addons.push(StremioAddon {
                                        manifest_url,
                                        client,
                                        name: addon_name.clone(),
                                        manifest,
                                    });
                                }
                            });
                            STREMIO_STREAMS.with(|slot| slot.borrow_mut().clear());
                            // Board catalogs are always recomputed (cheap,
                            // synchronous, no network) so switching out of
                            // search mode later reflects this addon
                            // immediately; the visible Board UI itself is
                            // only touched while no search is in progress —
                            // a search result list owns `stremio-media`
                            // until the user clears it (see
                            // BOARD_SEARCH_ACTIVE).
                            DB_HANDLE.with(|db| {
                                if let Some(db) = db.borrow().as_ref() {
                                    rebuild_board_catalogs(db);
                                }
                            });
                            if search_is_still_current && !BOARD_SEARCH_ACTIVE.with(Cell::get) {
                                app.set_stremio_stream_names(slint::ModelRc::default());
                                app.set_stremio_selected_title("".into());
                                rebuild_board_ui(&app, &image_handle, &image_handle);
                                app.set_stremio_status(format!("{addon_name} is ready.").into());
                            }
                        }
                        Err(err) if search_is_still_current => {
                            app.set_stremio_status(format!("Error: {err}").into())
                        }
                        Err(_) => {}
                    }
                });
            });
        });
    }

    // Global search (S21): queries only search-capable catalogs whose other
    // required extras are satisfied (ankai_core::board::search_eligible_catalogs
    // — see that function's doc comment), merges results across addons with
    // (type, id) dedupe (ankai_core::board::dedupe_attributed), and swaps the
    // Board surface into a flat search-result view via BOARD_SEARCH_ACTIVE.
    // An empty query clears search mode and restores normal per-catalog
    // Board browsing.
    {
        let runtime = p2p_runtime.handle().clone();
        let app_weak = app.as_weak();
        app.on_search_stremio(move |query| {
            let query = query.trim().to_owned();
            let generation = STREMIO_SEARCH_GENERATION.with(RequestGeneration::issue);
            STREMIO_DETAIL_GENERATION.with(RequestGeneration::issue);

            if query.is_empty() {
                BOARD_SEARCH_ACTIVE.with(|active| active.set(false));
                if let Some(app) = app_weak.upgrade() {
                    app.set_stremio_loading(false);
                    app.set_board_search_active(false);
                    app.set_stremio_status("Browsing installed catalogs.".into());
                    rebuild_board_ui(&app, &runtime, &runtime);
                }
                return;
            }

            let eligible: Vec<(String, String, String, String)> = BOARD_CATALOGS.with(|slot| {
                ankai_core::board::search_eligible_catalogs(&slot.borrow())
                    .into_iter()
                    .map(|catalog| {
                        (
                            catalog.key.addon_manifest_url.clone(),
                            catalog.key.media_type.clone(),
                            catalog.key.catalog_id.clone(),
                            catalog.addon_name.clone(),
                        )
                    })
                    .collect()
            });
            if eligible.is_empty() {
                if let Some(app) = app_weak.upgrade() {
                    app.set_stremio_loading(false);
                    app.set_stremio_status("None of the installed catalogs support search.".into());
                }
                return;
            }

            BOARD_SEARCH_ACTIVE.with(|active| active.set(true));
            if let Some(app) = app_weak.upgrade() {
                app.set_board_search_active(true);
                app.set_stremio_status("Searching…".into());
                app.set_stremio_loading(true);
            }
            let app_weak = app_weak.clone();
            let addons = STREMIO_ADDONS.with(|slot| slot.borrow().clone());
            let image_handle = runtime.clone();
            runtime.spawn(async move {
                let mut errors = Vec::new();
                let mut attributed = Vec::new();
                for (addon_url, media_type, catalog_id, addon_name) in &eligible {
                    let Some(client) = addons
                        .iter()
                        .find(|addon| &addon.manifest_url == addon_url)
                        .map(|addon| addon.client.clone())
                    else {
                        continue;
                    };
                    match client
                        .catalog_with_extra(media_type, catalog_id, &[("search", &query)])
                        .await
                    {
                        Ok(items) => {
                            attributed.extend(items.into_iter().map(|item| {
                                ankai_core::board::AttributedItem {
                                    item,
                                    addon_manifest_url: addon_url.clone(),
                                    addon_name: addon_name.clone(),
                                    catalog_id: catalog_id.clone(),
                                }
                            }));
                        }
                        Err(err) => errors.push(format!("{addon_name}: {err}")),
                    }
                }
                let mut deduped = ankai_core::board::dedupe_attributed(attributed);
                rank_search_results_by_relevance(&query, &mut deduped);
                let _ = slint::invoke_from_event_loop(move || {
                    if !STREMIO_SEARCH_GENERATION.with(|state| state.is_current(generation)) {
                        return;
                    }
                    let Some(app) = app_weak.upgrade() else {
                        return;
                    };
                    app.set_stremio_loading(false);
                    if deduped.is_empty() && !errors.is_empty() {
                        app.set_stremio_status(format!("Error: {}", errors.join(" | ")).into());
                    } else {
                        let count = deduped.len();
                        let addon_index_for = |url: &str| {
                            STREMIO_ADDONS.with(|slot| {
                                slot.borrow()
                                    .iter()
                                    .position(|addon| addon.manifest_url == url)
                            })
                        };
                        let (media, sources): (Vec<_>, Vec<_>) = deduped
                            .into_iter()
                            .map(|entry| {
                                (
                                    entry.item,
                                    addon_index_for(&entry.addon_manifest_url)
                                        .unwrap_or(usize::MAX),
                                )
                            })
                            .unzip();
                        show_stremio_media(&app, media, sources, &image_handle);
                        let suffix = if errors.is_empty() {
                            String::new()
                        } else {
                            format!(" Some addons failed: {}", errors.join(" | "))
                        };
                        app.set_stremio_status(
                            format!("Found {count} titles across search-capable catalogs.{suffix}")
                                .into(),
                        );
                    }
                });
            });
        });
    }

    // Board type/addon/catalog/genre filter selectors (S21). Each handler
    // updates BOARD_FILTER then rebuilds — selecting a different catalog
    // clears the genre filter (a genre choice is only meaningful for the
    // catalog it was picked against), and selecting a different type/addon
    // clears the catalog filter (the previously selected catalog may no
    // longer match).
    {
        let runtime = p2p_runtime.handle().clone();
        let app_weak = app.as_weak();
        app.on_select_board_type(move |value| {
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            BOARD_FILTER.with(|slot| {
                let mut filter = slot.borrow_mut();
                filter.selected_type = (!value.is_empty()).then(|| value.to_string());
                filter.selected_catalog_key = None;
                filter.selected_genre = None;
            });
            rebuild_board_ui(&app, &runtime, &runtime);
        });
    }
    {
        let runtime = p2p_runtime.handle().clone();
        let app_weak = app.as_weak();
        app.on_select_board_addon(move |value| {
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            BOARD_FILTER.with(|slot| {
                let mut filter = slot.borrow_mut();
                filter.selected_addon_url = (!value.is_empty()).then(|| value.to_string());
                filter.selected_catalog_key = None;
                filter.selected_genre = None;
            });
            rebuild_board_ui(&app, &runtime, &runtime);
        });
    }
    {
        let runtime = p2p_runtime.handle().clone();
        let app_weak = app.as_weak();
        app.on_select_board_catalog(move |value| {
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            BOARD_FILTER.with(|slot| {
                let mut filter = slot.borrow_mut();
                filter.selected_catalog_key = (!value.is_empty()).then(|| value.to_string());
                filter.selected_genre = None;
            });
            rebuild_board_ui(&app, &runtime, &runtime);
        });
    }
    {
        let runtime = p2p_runtime.handle().clone();
        let app_weak = app.as_weak();
        app.on_select_board_genre(move |value| {
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            BOARD_FILTER.with(|slot| {
                slot.borrow_mut().selected_genre = (!value.is_empty()).then(|| value.to_string());
            });
            // A genre change re-queries the selected catalog from scratch
            // (fresh skip=0 page) rather than appending — the item set for
            // a different genre is a different result set, not a
            // continuation of the previous one.
            if let Some(key) = BOARD_FILTER.with(|slot| slot.borrow().selected_catalog_key.clone())
            {
                BOARD_PAGE_STATE.with(|slot| {
                    slot.borrow_mut().remove(&key);
                });
            }
            rebuild_board_ui(&app, &runtime, &runtime);
        });
    }
    {
        let runtime = p2p_runtime.handle().clone();
        let app_weak = app.as_weak();
        app.on_load_more_board_row(move |row_key| {
            let row_key = row_key.to_string();
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            let can_load = BOARD_PAGE_STATE.with(|slot| {
                slot.borrow()
                    .get(&row_key)
                    .is_some_and(|state| !state.loading && state.has_more)
            });
            if !can_load {
                return;
            }
            BOARD_PAGE_STATE.with(|slot| {
                if let Some(state) = slot.borrow_mut().get_mut(&row_key) {
                    state.loading = true;
                }
            });
            rebuild_board_ui(&app, &runtime, &runtime);
            fetch_board_row(&runtime, app.as_weak(), runtime.clone(), row_key);
        });
    }
    {
        let runtime = p2p_runtime.handle().clone();
        let app_weak = app.as_weak();
        app.on_retry_board_row(move |row_key| {
            let row_key = row_key.to_string();
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            BOARD_PAGE_STATE.with(|slot| {
                let state = slot
                    .borrow_mut()
                    .entry(row_key.clone())
                    .or_insert_with(ankai_core::board::CatalogPageState::new)
                    .clone();
                let mut state = state;
                state.error = None;
                state.loading = true;
                slot.borrow_mut().insert(row_key.clone(), state);
            });
            rebuild_board_ui(&app, &runtime, &runtime);
            fetch_board_row(&runtime, app.as_weak(), runtime.clone(), row_key);
        });
    }

    // Deep-link parsing (S21): ankai://search|discover|detail|video|
    // addon-install, plus stremio:// install links and bare manifest URLs
    // (ankai_core::deeplink). This is a real, tested parser wired to a real
    // "open a link" field in the Board header, not OS-level custom URL
    // scheme registration — see that module's doc comment for the exact
    // boundary.
    {
        let runtime = p2p_runtime.handle().clone();
        let app_weak = app.as_weak();
        app.on_open_deep_link(move |raw| {
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            match ankai_core::deeplink::parse(raw.trim()) {
                Ok(ankai_core::deeplink::DeepLink::Search { query }) => {
                    app.set_stremio_search_query(query.clone().into());
                    app.invoke_search_stremio(query.into());
                }
                Ok(ankai_core::deeplink::DeepLink::Discover {
                    media_type,
                    addon_manifest_url,
                    catalog_id,
                    genre,
                }) => {
                    BOARD_FILTER.with(|slot| {
                        *slot.borrow_mut() = BoardFilter {
                            selected_type: media_type,
                            selected_addon_url: addon_manifest_url,
                            selected_catalog_key: None,
                            selected_genre: genre,
                        };
                    });
                    // A discover link may name a catalog by id, which is
                    // ambiguous across addons on its own — resolve it
                    // against the addon filter (if given) or the first
                    // catalog with a matching id otherwise.
                    if let Some(catalog_id) = catalog_id {
                        let key = BOARD_CATALOGS.with(|slot| {
                            slot.borrow()
                                .iter()
                                .find(|catalog| {
                                    catalog.key.catalog_id == catalog_id
                                        && BOARD_FILTER.with(|f| {
                                            f.borrow().selected_addon_url.as_ref().is_none_or(
                                                |url| *url == catalog.key.addon_manifest_url,
                                            )
                                        })
                                })
                                .map(board_row_key)
                        });
                        BOARD_FILTER.with(|slot| slot.borrow_mut().selected_catalog_key = key);
                    }
                    BOARD_SEARCH_ACTIVE.with(|active| active.set(false));
                    app.set_board_search_active(false);
                    app.set_selected_index(app.get_watch_index());
                    rebuild_board_ui(&app, &runtime, &runtime);
                }
                Ok(ankai_core::deeplink::DeepLink::Detail {
                    media_type,
                    id,
                    addon_manifest_url,
                }) => {
                    app.set_selected_index(app.get_watch_index());
                    open_meta_by_id(&app, &runtime, media_type, id, addon_manifest_url);
                }
                Ok(ankai_core::deeplink::DeepLink::Video {
                    media_type,
                    id,
                    video_id,
                    addon_manifest_url,
                }) => {
                    app.set_selected_index(app.get_watch_index());
                    PENDING_DEEP_LINK_VIDEO.with(|slot| *slot.borrow_mut() = Some(video_id));
                    open_meta_by_id(&app, &runtime, media_type, id, addon_manifest_url);
                }
                Ok(ankai_core::deeplink::DeepLink::AddonInstall { manifest_url }) => {
                    app.set_selected_index(app.get_watch_index());
                    app.set_stremio_addon_url(manifest_url.clone().into());
                    app.invoke_load_stremio_addon(manifest_url.into());
                }
                Err(error) => {
                    app.set_shell_notice(format!("Couldn't open that link: {error}").into());
                }
            }
        });
    }

    {
        let runtime = p2p_runtime.handle().clone();
        let app_weak = app.as_weak();
        app.on_open_stremio_media(move |index| {
            let generation = STREMIO_DETAIL_GENERATION.with(RequestGeneration::issue);
            let selected = STREMIO_MEDIA.with(|items| items.borrow().get(index as usize).cloned());
            let addons = STREMIO_ADDONS.with(|addons| addons.borrow().clone());
            let Some(selected) = selected else {
                if let Some(app) = app_weak.upgrade() {
                    app.set_shell_notice(
                        "That catalog item is no longer available. Search again.".into(),
                    );
                }
                return;
            };
            let provider = STREMIO_MEDIA_SOURCES
                .with(|sources| sources.borrow().get(index as usize).copied())
                .and_then(|source| addons.get(source))
                .map(|addon| addon.manifest_url.clone())
                .unwrap_or_else(|| "stremio".into());
            STREMIO_SELECTED_CONTEXT.with(|context| {
                *context.borrow_mut() = Some((provider.clone(), selected.id.clone()));
            });
            PENDING_PLAYBACK_KEY.with(|key| {
                *key.borrow_mut() = Some(ankai_core::playback_progress::PlaybackKey::movie(
                    provider,
                    selected.id.clone(),
                ));
            });
            PENDING_PLAYBACK_METADATA.with(|metadata| {
                *metadata.borrow_mut() = ankai_core::playback_progress::PlaybackMetadata {
                    title: Some(selected.name.clone()),
                    poster_url: selected.poster.clone(),
                    stream_url: None,
                };
            });
            if let Some(app) = app_weak.upgrade() {
                app.set_stremio_selected_title(selected.name.clone().into());
                app.set_stremio_selected_description(
                    selected.description.clone().unwrap_or_default().into(),
                );
                app.set_stremio_selected_meta_line(
                    format!(
                        "{}{}",
                        selected.media_type,
                        selected
                            .release_info
                            .as_deref()
                            .map(|release| format!("  ·  {release}"))
                            .unwrap_or_default()
                    )
                    .into(),
                );
                app.set_stremio_selected_cast("".into());
                app.set_stremio_episodes(slint::ModelRc::default());
                if let Some(card) = app.get_stremio_media().row_data(index as usize) {
                    app.set_stremio_selected_poster(card.poster);
                    app.set_stremio_selected_has_poster(card.has_poster);
                }
                app.set_stremio_status("Loading streams…".into());
                app.set_stremio_loading(true);
            }
            let app_weak = app_weak.clone();
            let image_handle = runtime.clone();
            runtime.spawn(async move {
                let mut streams = Vec::new();
                let mut errors = Vec::new();
                let mut selected_meta = None;
                for addon in &addons {
                    if selected_meta.is_none()
                        && addon.manifest.supports_resource(
                            "meta",
                            &selected.media_type,
                            &selected.id,
                        )
                    {
                        match addon.client.meta(&selected.media_type, &selected.id).await {
                            Ok(meta) => selected_meta = Some(meta),
                            Err(err) => errors.push(format!("{} metadata: {err}", addon.name)),
                        }
                    }
                    if !addon.manifest.supports_resource(
                        "stream",
                        &selected.media_type,
                        &selected.id,
                    ) {
                        continue;
                    }
                    match addon
                        .client
                        .streams(&selected.media_type, &selected.id)
                        .await
                    {
                        Ok(mut addon_streams) => {
                            for stream in &mut addon_streams {
                                if stream.name.is_none() {
                                    stream.name = Some(addon.name.clone());
                                }
                            }
                            streams.extend(addon_streams);
                        }
                        Err(err) => errors.push(format!("{}: {err}", addon.name)),
                    }
                }
                let _ = slint::invoke_from_event_loop(move || {
                    if !STREMIO_DETAIL_GENERATION.with(|state| state.is_current(generation)) {
                        return;
                    }
                    let Some(app) = app_weak.upgrade() else {
                        return;
                    };
                    app.set_stremio_loading(false);
                    if let Some(meta) = selected_meta.as_ref() {
                        show_stremio_meta(&app, meta, &image_handle);
                    } else {
                        STREMIO_EPISODES.with(|slot| slot.borrow_mut().clear());
                    }
                    if streams.is_empty() && !errors.is_empty() {
                        app.set_stremio_status(format!("Error: {}", errors.join(" | ")).into());
                    } else {
                        rank_streams_for_playability(&mut streams);
                        let autoplay = top_stream_is_playable(&streams);
                        let labels = streams
                            .iter()
                            .map(|stream| {
                                format!(
                                    "{} — {}",
                                    stremio_stream_kind(stream),
                                    stream
                                        .title
                                        .as_deref()
                                        .or(stream.name.as_deref())
                                        .unwrap_or("Unnamed stream")
                                )
                                .into()
                            })
                            .collect::<Vec<slint::SharedString>>();
                        let count = streams.len();
                        STREMIO_STREAMS.with(|slot| *slot.borrow_mut() = streams);
                        app.set_stremio_stream_names(slint::ModelRc::from(std::rc::Rc::new(
                            slint::VecModel::from(labels),
                        )));
                        app.set_stremio_status(format!("Found {count} streams.").into());
                        // Play the best-ranked stream automatically — see
                        // the matching comment in `open_meta_by_id` above.
                        // Series usually have no root-level playable
                        // streams (real episode streams come from clicking
                        // an episode below, handled separately), so this
                        // mainly fires for movies.
                        if autoplay {
                            app.invoke_activate_stremio_stream(0);
                        }
                    }
                });
            });
        });
    }

    {
        let runtime = p2p_runtime.handle().clone();
        let app_weak = app.as_weak();
        app.on_open_stremio_episode(move |index| {
            let generation = STREMIO_DETAIL_GENERATION.with(RequestGeneration::issue);
            let episode =
                STREMIO_EPISODES.with(|episodes| episodes.borrow().get(index as usize).cloned());
            let addons = STREMIO_ADDONS.with(|addons| addons.borrow().clone());
            let Some(episode) = episode else {
                if let Some(app) = app_weak.upgrade() {
                    app.set_shell_notice(
                        "That episode is no longer available. Reopen the title.".into(),
                    );
                }
                return;
            };
            STREMIO_SELECTED_CONTEXT.with(|context| {
                if let Some((provider, media_id)) = context.borrow().as_ref() {
                    PENDING_PLAYBACK_KEY.with(|key| {
                        *key.borrow_mut() =
                            Some(ankai_core::playback_progress::PlaybackKey::episode(
                                provider.clone(),
                                media_id.clone(),
                                episode.id.clone(),
                            ));
                    });
                }
            });
            PENDING_PLAYBACK_METADATA.with(|metadata| {
                let mut metadata = metadata.borrow_mut();
                let episode_title = episode.title.as_deref().unwrap_or("Untitled episode");
                metadata.title = Some(format!(
                    "{} — {episode_title}",
                    app_weak
                        .upgrade()
                        .map(|app| app.get_stremio_selected_title().to_string())
                        .filter(|title| !title.trim().is_empty())
                        .unwrap_or_else(|| "Series".to_string())
                ));
                metadata.stream_url = None;
            });
            if let Some(app) = app_weak.upgrade() {
                app.set_stremio_loading(true);
                app.set_stremio_stream_names(slint::ModelRc::default());
                app.set_stremio_status(
                    format!(
                        "Loading streams for {}…",
                        episode.title.as_deref().unwrap_or("this episode")
                    )
                    .into(),
                );
            }
            let app_weak = app_weak.clone();
            runtime.spawn(async move {
                let mut streams = episode.streams.clone();
                let mut errors = Vec::new();
                if streams.is_empty() {
                    for addon in addons.iter().filter(|addon| {
                        addon
                            .manifest
                            .supports_resource("stream", "series", &episode.id)
                    }) {
                        match addon.client.streams("series", &episode.id).await {
                            Ok(mut addon_streams) => {
                                for stream in &mut addon_streams {
                                    if stream.name.is_none() {
                                        stream.name = Some(addon.name.clone());
                                    }
                                }
                                streams.extend(addon_streams);
                            }
                            Err(error) => errors.push(format!("{}: {error}", addon.name)),
                        }
                    }
                } else {
                    for stream in &mut streams {
                        if stream.name.is_none() {
                            stream.name = Some("Inline metadata".into());
                        }
                    }
                }
                let _ = slint::invoke_from_event_loop(move || {
                    if !STREMIO_DETAIL_GENERATION.with(|state| state.is_current(generation)) {
                        return;
                    }
                    let Some(app) = app_weak.upgrade() else {
                        return;
                    };
                    app.set_stremio_loading(false);
                    rank_streams_for_playability(&mut streams);
                    let autoplay = top_stream_is_playable(&streams);
                    let labels = streams
                        .iter()
                        .map(|stream| {
                            format!(
                                "{} — {}",
                                stremio_stream_kind(stream),
                                stream
                                    .title
                                    .as_deref()
                                    .or(stream.name.as_deref())
                                    .unwrap_or("Unnamed stream")
                            )
                            .into()
                        })
                        .collect::<Vec<slint::SharedString>>();
                    let count = streams.len();
                    STREMIO_STREAMS.with(|slot| *slot.borrow_mut() = streams);
                    app.set_stremio_stream_names(slint::ModelRc::from(Rc::new(
                        slint::VecModel::from(labels),
                    )));
                    app.set_stremio_status(if count == 0 && !errors.is_empty() {
                        format!("No episode stream loaded: {}", errors.join(" | ")).into()
                    } else {
                        format!("Found {count} episode streams.").into()
                    });
                    // Play the best-ranked stream automatically — see the
                    // matching comment in `open_meta_by_id` above. This is
                    // the most common real trigger (clicking an episode).
                    if autoplay {
                        app.invoke_activate_stremio_stream(0);
                    }
                });
            });
        });
    }

    {
        let app_weak = app.as_weak();
        app.on_activate_stremio_stream(move |index| {
            let (message, playing) = STREMIO_STREAMS.with(|streams| {
                let streams = streams.borrow();
                match streams.get(index as usize).map(|stream| stream.source()) {
                    Some(Ok(ankai_core::stremio::StreamSource::Direct(url))) => {
                        VIDEO_PLAYER.with(|slot| {
                            let mut slot = slot.borrow_mut();
                            let Some(player) = slot.as_mut() else {
                                return (
                                    "Error: embedded video renderer is unavailable.".into(),
                                    false,
                                );
                            };
                            LAST_PLAYBACK_URL
                                .with(|last_url| *last_url.borrow_mut() = Some(url.to_owned()));
                            PENDING_PLAYBACK_METADATA.with(|metadata| {
                                metadata.borrow_mut().stream_url = Some(url.to_owned());
                            });
                            match player.load(url) {
                                Ok(()) => ("Playing in ANKAI with libmpv.".into(), true),
                                Err(err) => (format!("Error: {err}"), false),
                            }
                        })
                    }
                    Some(Ok(ankai_core::stremio::StreamSource::BitTorrent { .. })) => (
                        "Torrent stream selected; a torrent resolver is required before playback."
                            .into(),
                        false,
                    ),
                    Some(Err(err)) => (format!("Error: {err}"), false),
                    None => ("Error: stream is no longer available.".into(), false),
                }
            });
            if let Some(app) = app_weak.upgrade() {
                app.set_stremio_status(message.clone().into());
                if !playing {
                    // The status line above sits near the top of the detail
                    // panel, well above the "PLAY FROM" row a user just
                    // clicked in — easy to scroll past and never see. Mirror
                    // non-playable outcomes (unsupported transport, missing
                    // resolver, a real fetch error) into the shell's toast,
                    // which renders in a fixed screen position regardless of
                    // scroll, so clicking a stream that can't play yet is
                    // never silent.
                    app.set_shell_notice(message.into());
                }
                if playing {
                    ACTIVE_PLAYBACK_KEY.with(|active| {
                        *active.borrow_mut() =
                            PENDING_PLAYBACK_KEY.with(|pending| pending.borrow().clone());
                    });
                    ACTIVE_PLAYBACK_METADATA.with(|active| {
                        *active.borrow_mut() =
                            PENDING_PLAYBACK_METADATA.with(|pending| pending.borrow().clone());
                    });
                    let resume = ACTIVE_PLAYBACK_KEY.with(|key| {
                        let key = key.borrow();
                        let key = key.as_ref()?;
                        DB_HANDLE.with(|db| {
                            let db = db.borrow();
                            let db = db.as_ref()?;
                            ankai_core::playback_progress::load(db, key)
                                .ok()
                                .flatten()
                                .filter(|progress| {
                                    !progress.completed
                                        && progress.position_seconds >= 5.0
                                        && progress.duration_seconds - progress.position_seconds
                                            >= 5.0
                                })
                                .map(|progress| progress.position_seconds)
                        })
                    });
                    PENDING_RESUME_SECONDS.with(|pending| *pending.borrow_mut() = resume);
                    LAST_PROGRESS_SAVE.with(|last_save| *last_save.borrow_mut() = None);
                    app.set_player_title(app.get_stremio_selected_title());
                    app.set_player_active(true);
                    app.set_player_has_media(true);
                    app.set_player_video_ready(false);
                    app.set_player_video_frame_ready(false);
                    app.set_player_loading(true);
                    app.set_player_error("".into());
                    app.window().request_redraw();
                }
            }
        });
    }

    {
        let app_weak = app.as_weak();
        app.on_toggle_player_pause(move || {
            let state = VIDEO_PLAYER.with(|slot| {
                if let Some(player) = slot.borrow_mut().as_mut() {
                    player.toggle_pause().ok()?;
                    Some(player.state().clone())
                } else {
                    None
                }
            });
            if let Some(app) = app_weak.upgrade() {
                if let Some(state) = state.as_ref() {
                    sync_player_state(&app, state);
                }
                app.window().request_redraw();
            }
        });
    }
    {
        let db = db.clone();
        let image_handle = p2p_runtime.handle().clone();
        let app_weak = app.as_weak();
        app.on_close_player(move || {
            VIDEO_PLAYER.with(|slot| {
                if let Some(player) = slot.borrow_mut().as_mut() {
                    persist_player_progress(player.state(), true);
                    let _ = player.stop();
                }
            });
            ACTIVE_PLAYBACK_KEY.with(|key| key.borrow_mut().take());
            ACTIVE_PLAYBACK_METADATA.with(|metadata| {
                *metadata.borrow_mut() = ankai_core::playback_progress::PlaybackMetadata::default();
            });
            if let Some(app) = app_weak.upgrade() {
                app.window().set_fullscreen(false);
                app.set_player_fullscreen(false);
                app.set_player_active(false);
                app.set_player_video_frame(slint::Image::default());
                app.set_player_video_frame_ready(false);
                sync_player_state(&app, &playback::PlayerState::default());
                refresh_resume_entries(&app, &db, &image_handle);
            }
        });
    }
    {
        let app_weak = app.as_weak();
        app.on_player_frame(move || {
            if let Some(app) = app_weak.upgrade() {
                app.window().request_redraw();
            }
        });
    }
    {
        let app_weak = app.as_weak();
        app.on_player_state_tick(move || {
            let result = VIDEO_PLAYER.with(|slot| {
                let mut slot = slot.borrow_mut();
                let Some(player) = slot.as_mut() else {
                    return Ok::<_, playback::PlayerError>(None);
                };
                let events = player.poll_events()?;
                if events
                    .iter()
                    .any(|event| matches!(event, playback::PlayerEvent::FileLoaded))
                {
                    if let Some(position) =
                        PENDING_RESUME_SECONDS.with(|pending| pending.borrow_mut().take())
                    {
                        player.seek_absolute(position)?;
                    }
                }
                player.refresh_state()?;
                Ok(Some(player.state().clone()))
            });
            if let Some(app) = app_weak.upgrade() {
                match result {
                    Ok(Some(state)) => {
                        sync_player_state(&app, &state);
                        persist_player_progress(
                            &state,
                            state.phase == playback::PlaybackPhase::Ended,
                        );
                    }
                    Ok(None) => {}
                    Err(error) => app.set_player_error(error.to_string().into()),
                }
            }
        });
    }
    {
        let app_weak = app.as_weak();
        app.on_player_seek_relative(move |seconds| {
            let state = VIDEO_PLAYER.with(|slot| {
                let mut slot = slot.borrow_mut();
                let player = slot.as_mut()?;
                player.seek_relative(seconds as f64).ok()?;
                Some(player.state().clone())
            });
            if let (Some(app), Some(state)) = (app_weak.upgrade(), state.as_ref()) {
                sync_player_state(&app, state);
            }
        });
    }
    {
        let app_weak = app.as_weak();
        app.on_player_seek(move |seconds| {
            let result = VIDEO_PLAYER.with(|slot| {
                let mut slot = slot.borrow_mut();
                let Some(player) = slot.as_mut() else {
                    return Ok::<_, playback::PlayerError>(None);
                };
                player.seek_absolute(seconds as f64)?;
                Ok(Some(player.state().clone()))
            });
            if let Some(app) = app_weak.upgrade() {
                match result {
                    Ok(Some(state)) => sync_player_state(&app, &state),
                    Ok(None) => {}
                    Err(error) => app.set_player_error(error.to_string().into()),
                }
            }
        });
    }
    {
        let app_weak = app.as_weak();
        app.on_player_toggle_mute(move || {
            let result = VIDEO_PLAYER.with(|slot| {
                let mut slot = slot.borrow_mut();
                let Some(player) = slot.as_mut() else {
                    return Ok::<_, playback::PlayerError>(None);
                };
                player.toggle_mute()?;
                Ok(Some(player.state().clone()))
            });
            if let Some(app) = app_weak.upgrade() {
                match result {
                    Ok(Some(state)) => sync_player_state(&app, &state),
                    Ok(None) => {}
                    Err(error) => app.set_player_error(error.to_string().into()),
                }
            }
        });
    }
    {
        let app_weak = app.as_weak();
        app.on_player_set_volume(move |volume| {
            let result = VIDEO_PLAYER.with(|slot| {
                let mut slot = slot.borrow_mut();
                let Some(player) = slot.as_mut() else {
                    return Ok::<_, playback::PlayerError>(None);
                };
                player.set_volume(volume as f64)?;
                Ok(Some(player.state().clone()))
            });
            if let Some(app) = app_weak.upgrade() {
                match result {
                    Ok(Some(state)) => sync_player_state(&app, &state),
                    Ok(None) => {}
                    Err(error) => app.set_player_error(error.to_string().into()),
                }
            }
        });
    }
    {
        let app_weak = app.as_weak();
        app.on_player_set_speed(move |speed| {
            let result = VIDEO_PLAYER.with(|slot| {
                let mut slot = slot.borrow_mut();
                let Some(player) = slot.as_mut() else {
                    return Ok::<_, playback::PlayerError>(None);
                };
                player.set_speed(speed as f64)?;
                Ok(Some(player.state().clone()))
            });
            if let Some(app) = app_weak.upgrade() {
                match result {
                    Ok(Some(state)) => sync_player_state(&app, &state),
                    Ok(None) => {}
                    Err(error) => app.set_player_error(error.to_string().into()),
                }
            }
        });
    }
    {
        let app_weak = app.as_weak();
        app.on_player_select_audio(move |track_id| {
            let result = VIDEO_PLAYER.with(|slot| {
                let mut slot = slot.borrow_mut();
                let Some(player) = slot.as_mut() else {
                    return Ok::<_, playback::PlayerError>(None);
                };
                player.set_audio_track(Some(i64::from(track_id)))?;
                player.refresh_state()?;
                Ok(Some(player.state().clone()))
            });
            if let Some(app) = app_weak.upgrade() {
                match result {
                    Ok(Some(state)) => sync_player_state(&app, &state),
                    Ok(None) => {}
                    Err(error) => app.set_player_error(error.to_string().into()),
                }
            }
        });
    }
    {
        let app_weak = app.as_weak();
        app.on_player_select_subtitle(move |track_id| {
            let result = VIDEO_PLAYER.with(|slot| {
                let mut slot = slot.borrow_mut();
                let Some(player) = slot.as_mut() else {
                    return Ok::<_, playback::PlayerError>(None);
                };
                player.set_subtitle_track(Some(i64::from(track_id)))?;
                player.refresh_state()?;
                Ok(Some(player.state().clone()))
            });
            if let Some(app) = app_weak.upgrade() {
                match result {
                    Ok(Some(state)) => sync_player_state(&app, &state),
                    Ok(None) => {}
                    Err(error) => app.set_player_error(error.to_string().into()),
                }
            }
        });
    }
    {
        let app_weak = app.as_weak();
        app.on_player_disable_subtitles(move || {
            let result = VIDEO_PLAYER.with(|slot| {
                let mut slot = slot.borrow_mut();
                let Some(player) = slot.as_mut() else {
                    return Ok::<_, playback::PlayerError>(None);
                };
                player.set_subtitle_track(None)?;
                player.refresh_state()?;
                Ok(Some(player.state().clone()))
            });
            if let Some(app) = app_weak.upgrade() {
                match result {
                    Ok(Some(state)) => sync_player_state(&app, &state),
                    Ok(None) => {}
                    Err(error) => app.set_player_error(error.to_string().into()),
                }
            }
        });
    }
    {
        let app_weak = app.as_weak();
        app.on_player_toggle_fullscreen(move || {
            if let Some(app) = app_weak.upgrade() {
                let fullscreen = !app.get_player_fullscreen();
                app.window().set_fullscreen(fullscreen);
                app.set_player_fullscreen(fullscreen);
            }
        });
    }
    {
        let app_weak = app.as_weak();
        app.on_player_retry(move || {
            let url = LAST_PLAYBACK_URL.with(|last_url| last_url.borrow().clone());
            let result = VIDEO_PLAYER.with(|slot| {
                let mut slot = slot.borrow_mut();
                match (slot.as_mut(), url.as_deref()) {
                    (Some(player), Some(url)) => {
                        player.load(url)?;
                        Ok::<_, playback::PlayerError>(Some(player.state().clone()))
                    }
                    _ => Ok(None),
                }
            });
            if let Some(app) = app_weak.upgrade() {
                match result {
                    Ok(Some(state)) => sync_player_state(&app, &state),
                    Ok(None) => app.set_player_error("No previous stream to retry.".into()),
                    Err(error) => app.set_player_error(error.to_string().into()),
                }
            }
        });
    }

    {
        let app_weak = app.as_weak();
        app.on_global_search(move |query| {
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            let query = query.trim();
            if query.is_empty() {
                app.set_shell_notice("Type something to search.".into());
                return;
            }
            app.set_selected_index(app.get_watch_index());
            app.set_stremio_search_query(query.into());
            app.invoke_search_stremio(query.into());
        });
    }
    {
        let app_weak = app.as_weak();
        app.on_open_profile_menu(move || {
            if let Some(app) = app_weak.upgrade() {
                app.set_selected_index(app.get_profile_index());
            }
        });
    }
    {
        let app_weak = app.as_weak();
        app.on_resume_playback(move |index| {
            let entry =
                RESUME_PROGRESS.with(|entries| entries.borrow().get(index as usize).cloned());
            let Some(entry) = entry else {
                return;
            };
            let Some(url) = entry.stream_url.clone() else {
                if let Some(app) = app_weak.upgrade() {
                    app.set_selected_index(app.get_watch_index());
                    app.set_stremio_search_query(
                        entry.title.clone().unwrap_or(entry.key.media_id).into(),
                    );
                    app.set_shell_notice(
                        "This older resume point needs its stream resolved again.".into(),
                    );
                }
                return;
            };
            let loaded = VIDEO_PLAYER.with(|slot| {
                slot.borrow_mut()
                    .as_mut()
                    .ok_or_else(|| "embedded video renderer is unavailable".to_string())?
                    .load(&url)
                    .map_err(|error| error.to_string())
            });
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            if let Err(error) = loaded {
                app.set_shell_notice(format!("Couldn't resume playback: {error}").into());
                return;
            }
            let metadata = ankai_core::playback_progress::PlaybackMetadata {
                title: entry.title.clone(),
                poster_url: entry.poster_url.clone(),
                stream_url: Some(url.clone()),
            };
            ACTIVE_PLAYBACK_KEY.with(|key| *key.borrow_mut() = Some(entry.key));
            ACTIVE_PLAYBACK_METADATA.with(|active| *active.borrow_mut() = metadata);
            PENDING_RESUME_SECONDS
                .with(|position| *position.borrow_mut() = Some(entry.position_seconds));
            LAST_PLAYBACK_URL.with(|last_url| *last_url.borrow_mut() = Some(url));
            LAST_PROGRESS_SAVE.with(|last_save| *last_save.borrow_mut() = None);
            app.set_player_title(
                entry
                    .title
                    .unwrap_or_else(|| "Continue watching".into())
                    .into(),
            );
            // Unlike every other real playback trigger (activate-stremio-
            // stream's play-stream, deep-linked video opens), Home's
            // Continue Watching card starts playback while selected-index
            // is still home-index — this call site was the one place that
            // never switched to the Watch page. HomeDashboard's own render
            // condition previously didn't account for that either, so Home's
            // opaque cards kept rendering underneath the player overlay for
            // this flow specifically, reading as a broken translucent
            // overlay. Switch pages here too (matching the resume-needs-
            // resolving fallback branch above) as defense in depth alongside
            // the render-condition fix in app.slint.
            app.set_selected_index(app.get_watch_index());
            app.set_player_active(true);
            app.set_player_has_media(true);
            app.set_player_video_ready(false);
            app.set_player_video_frame_ready(false);
            app.set_player_loading(true);
            app.set_player_error("".into());
            app.window().request_redraw();
        });
    }

    {
        let db = db.clone();
        let image_handle = p2p_runtime.handle().clone();
        let app_weak = app.as_weak();
        app.on_refresh_addon_registry(move || {
            if let Some(app) = app_weak.upgrade() {
                refresh_addon_manager(&app, &db, &image_handle);
            }
        });
    }
    {
        let db = db.clone();
        let image_handle = p2p_runtime.handle().clone();
        let app_weak = app.as_weak();
        app.on_toggle_installed_addon(move |manifest_url, enabled| {
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            match ankai_core::addons::set_enabled(&db, &manifest_url, enabled) {
                Ok(_) => {
                    refresh_addon_manager(&app, &db, &image_handle);
                    reload_enabled_addons(&app, &db);
                }
                Err(error) => {
                    app.set_stremio_status(format!("Couldn't update addon: {error}").into())
                }
            }
        });
    }
    {
        let app_weak = app.as_weak();
        app.on_refresh_installed_addon(move |manifest_url| {
            if let Some(app) = app_weak.upgrade() {
                app.invoke_load_stremio_addon(manifest_url);
            }
        });
    }
    {
        let db = db.clone();
        let image_handle = p2p_runtime.handle().clone();
        let app_weak = app.as_weak();
        app.on_move_installed_addon_up(move |manifest_url| {
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            let result = ankai_core::addons::list(&db).and_then(|addons| {
                let index = addons
                    .iter()
                    .position(|addon| manifest_url == addon.manifest_url)
                    .ok_or_else(|| ankai_core::Error::Stremio("addon is not installed".into()))?;
                ankai_core::addons::move_to(&db, &manifest_url, index.saturating_sub(1))
            });
            match result {
                Ok(_) => {
                    refresh_addon_manager(&app, &db, &image_handle);
                    reload_enabled_addons(&app, &db);
                }
                Err(error) => {
                    app.set_stremio_status(format!("Couldn't reorder addon: {error}").into())
                }
            }
        });
    }
    {
        let db = db.clone();
        let image_handle = p2p_runtime.handle().clone();
        let app_weak = app.as_weak();
        app.on_move_installed_addon_down(move |manifest_url| {
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            let result = ankai_core::addons::list(&db).and_then(|addons| {
                let index = addons
                    .iter()
                    .position(|addon| manifest_url == addon.manifest_url)
                    .ok_or_else(|| ankai_core::Error::Stremio("addon is not installed".into()))?;
                let target = (index + 1).min(addons.len().saturating_sub(1));
                ankai_core::addons::move_to(&db, &manifest_url, target)
            });
            match result {
                Ok(_) => {
                    refresh_addon_manager(&app, &db, &image_handle);
                    reload_enabled_addons(&app, &db);
                }
                Err(error) => {
                    app.set_stremio_status(format!("Couldn't reorder addon: {error}").into())
                }
            }
        });
    }
    {
        let db = db.clone();
        let image_handle = p2p_runtime.handle().clone();
        let app_weak = app.as_weak();
        app.on_remove_installed_addon(move |manifest_url| {
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            if matches!(
                manifest_url.as_str(),
                CINEMETA_MANIFEST_URL | ANIME_KITSU_MANIFEST_URL
            ) {
                app.set_shell_notice("Bundled providers can be disabled, but not removed.".into());
                return;
            }
            match ankai_core::addons::remove(&db, &manifest_url) {
                Ok(_) => {
                    // A removed addon's cached catalog pages must not
                    // outlive it — otherwise a later re-install could
                    // briefly show stale pages from before the removal.
                    BOARD_MEMORY_CACHE.with(|cache| cache.clear_for_addon(&manifest_url));
                    if let Err(error) = ankai_core::catalog_cache::clear_for_addon(&db, &manifest_url) {
                        eprintln!("ankai-client: failed to clear catalog cache for removed addon: {error}");
                    }
                    refresh_addon_manager(&app, &db, &image_handle);
                    reload_enabled_addons(&app, &db);
                }
                Err(error) => {
                    app.set_stremio_status(format!("Couldn't remove addon: {error}").into())
                }
            }
        });
    }

    // Restore every enabled installed addon. Cinemeta and Anime Kitsu are
    // bundled zero-configuration catalog/metadata providers; custom addons
    // remain ordered alongside them and configured URL paths are preserved.
    let installed_addons = ankai_core::addons::list(&db).unwrap_or_else(|error| {
        app.set_stremio_status(format!("Couldn't read installed addons: {error}").into());
        Vec::new()
    });
    refresh_addon_manager(&app, &db, p2p_runtime.handle());
    let missing_builtins = [CINEMETA_MANIFEST_URL, ANIME_KITSU_MANIFEST_URL]
        .into_iter()
        .filter(|manifest_url| {
            !installed_addons
                .iter()
                .any(|addon| addon.manifest_url == *manifest_url)
        })
        .collect::<Vec<_>>();
    let has_enabled_addon = installed_addons.iter().any(|addon| addon.enabled);
    for manifest_url in &missing_builtins {
        app.invoke_load_stremio_addon((*manifest_url).into());
    }
    for addon in installed_addons.into_iter().filter(|addon| addon.enabled) {
        app.invoke_load_stremio_addon(addon.manifest_url.into());
    }
    if !missing_builtins.is_empty() {
        app.set_stremio_status("Starting bundled Cinemeta and Anime Kitsu…".into());
    } else if !has_enabled_addon {
        // The registry may contain only disabled providers. Discover remains
        // honest and empty until the user enables one in the manager.
        app.set_stremio_status("All installed addons are disabled.".into());
    }

    // Home's local sections load synchronously; live anime catalogs and
    // presence checks run on the shared runtime so third-party latency never
    // blocks the native window.
    refresh_recent_posts(&app, &db);
    let initial_friends = refresh_home_friends(&app, &db);
    spawn_anime_refresh(p2p_runtime.handle().clone(), app.as_weak());
    if let Some(node) = social_node.as_ref() {
        spawn_friend_presence_checks(
            initial_friends,
            node.clone(),
            p2p_runtime.handle().clone(),
            app.as_weak(),
        );
    }

    {
        let app_weak = app.as_weak();
        app.on_open_home_anime(move |id| {
            let anime = HOME_TRENDING_ANIME
                .with(|items| {
                    items
                        .borrow()
                        .iter()
                        .find(|anime| anime.id == id as i64)
                        .cloned()
                })
                .or_else(|| {
                    HOME_POPULAR_ANIME.with(|items| {
                        items
                            .borrow()
                            .iter()
                            .find(|anime| anime.id == id as i64)
                            .cloned()
                    })
                });
            let (Some(app), Some(anime)) = (app_weak.upgrade(), anime) else {
                return;
            };
            let title = anime
                .title_english
                .or(anime.title_romaji)
                .unwrap_or_else(|| format!("Anime {id}"));
            app.set_global_search_query(title.clone().into());
            app.invoke_global_search(title.into());
        });
    }

    // Watch Now records the title as watching and immediately enters the
    // playable-provider search; the neighboring Watchlist action below is a
    // real independent add/remove toggle.
    {
        let db_for_watch_now = db.clone();
        let app_weak_for_watch_now = app.as_weak();
        let handle_for_watch_now = p2p_runtime.handle().clone();
        app.on_watch_now(move |id| {
            let anime = HOME_TRENDING_ANIME
                .with(|store| store.borrow().iter().find(|a| a.id == id as i64).cloned());
            let Some(anime) = anime else {
                eprintln!(
                    "ankai-client: watch-now clicked for anime id {id}, not found in the last \
                     trending load"
                );
                if let Some(app) = app_weak_for_watch_now.upgrade() {
                    app.set_shell_notice(
                        "That title is no longer in the current Home results. Refresh and retry."
                            .into(),
                    );
                }
                return;
            };
            if let Err(err) = ankai_core::anime::add_to_watchlist(
                &db_for_watch_now,
                &anime,
                ankai_core::anime::WatchStatus::Watching,
            ) {
                eprintln!(
                    "ankai-client: failed to add anime {id} to watchlist from Home's Watch Now \
                     button: {err}"
                );
                if let Some(app) = app_weak_for_watch_now.upgrade() {
                    app.set_shell_notice(format!("Couldn't update watchlist: {err}").into());
                }
                return;
            }
            if let Some(app) = app_weak_for_watch_now.upgrade() {
                refresh_watchlist(&app, &db_for_watch_now, &handle_for_watch_now);
                app.set_hero_watchlisted(true);
                let title = anime
                    .title_english
                    .or(anime.title_romaji)
                    .unwrap_or_else(|| format!("Anime {id}"));
                app.set_global_search_query(title.clone().into());
                app.invoke_global_search(title.into());
            }
        });
    }
    {
        let db = db.clone();
        let image_handle = p2p_runtime.handle().clone();
        let app_weak = app.as_weak();
        app.on_toggle_home_watchlist(move |id| {
            let anime = HOME_TRENDING_ANIME.with(|items| {
                items
                    .borrow()
                    .iter()
                    .find(|anime| anime.id == id as i64)
                    .cloned()
            });
            let Some(anime) = anime else {
                if let Some(app) = app_weak.upgrade() {
                    app.set_shell_notice(
                        "That title is no longer in the current Home results. Refresh and retry."
                            .into(),
                    );
                }
                return;
            };
            let already_saved = ankai_core::anime::get_watchlist_entry(&db, anime.id)
                .ok()
                .flatten()
                .is_some();
            let result = if already_saved {
                ankai_core::anime::remove_from_watchlist(&db, anime.id)
            } else {
                ankai_core::anime::add_to_watchlist(
                    &db,
                    &anime,
                    ankai_core::anime::WatchStatus::Planned,
                )
                .map(|_| ())
            };
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            match result {
                Ok(()) => {
                    app.set_hero_watchlisted(!already_saved);
                    refresh_watchlist(&app, &db, &image_handle);
                    app.set_shell_notice(
                        if already_saved {
                            "Removed from watchlist."
                        } else {
                            "Saved to watchlist."
                        }
                        .into(),
                    );
                }
                Err(error) => {
                    app.set_shell_notice(format!("Couldn't update watchlist: {error}").into())
                }
            }
        });
    }

    {
        let db_for_refresh = db.clone();
        let social_node_for_refresh = social_node.clone();
        let p2p_handle_for_refresh = p2p_runtime.handle().clone();
        let app_weak_for_refresh = app.as_weak();
        app.on_refresh_home(move || {
            let Some(app) = app_weak_for_refresh.upgrade() else {
                return;
            };
            refresh_recent_posts(&app, &db_for_refresh);
            refresh_resume_entries(&app, &db_for_refresh, &p2p_handle_for_refresh);
            let friends = refresh_home_friends(&app, &db_for_refresh);
            spawn_anime_refresh(p2p_handle_for_refresh.clone(), app.as_weak());
            if let Some(node) = social_node_for_refresh.as_ref() {
                spawn_friend_presence_checks(
                    friends,
                    node.clone(),
                    p2p_handle_for_refresh.clone(),
                    app.as_weak(),
                );
            }
        });
    }

    let saved_letterboxd_username = db
        .get_setting(LETTERBOXD_USERNAME_SETTING_KEY)
        .unwrap_or_else(|error| {
            eprintln!("ankai-client: failed to read letterboxd_username setting: {error}");
            None
        })
        .unwrap_or_default();
    app.set_letterboxd_username(saved_letterboxd_username.clone().into());
    if saved_letterboxd_username.trim().is_empty() {
        app.set_letterboxd_state("not-configured".into());
    } else {
        spawn_letterboxd_refresh(
            p2p_runtime.handle().clone(),
            app.as_weak(),
            saved_letterboxd_username,
        );
    }
    {
        let db = db.clone();
        let runtime = p2p_runtime.handle().clone();
        let app_weak = app.as_weak();
        app.on_load_letterboxd(move |username| {
            let username = username.trim().to_owned();
            let Some(app) = app_weak.upgrade() else {
                return;
            };
            if let Err(error) = db.set_setting(LETTERBOXD_USERNAME_SETTING_KEY, &username) {
                eprintln!("ankai-client: failed to save Letterboxd username: {error}");
                app.set_letterboxd_state("error".into());
                app.set_letterboxd_status(
                    format!("Couldn't save the Letterboxd username: {error}").into(),
                );
                return;
            }
            app.set_letterboxd_username(username.clone().into());
            if username.is_empty() {
                // Prevent a feed for the previous username from repopulating
                // the panel after the user explicitly cleared it.
                LETTERBOXD_GENERATION.with(RequestGeneration::issue);
                app.set_letterboxd_entries(slint::ModelRc::default());
                app.set_letterboxd_state("not-configured".into());
                app.set_letterboxd_status("".into());
                app.set_shell_notice("Letterboxd diary disconnected.".into());
            } else {
                app.set_shell_notice("Letterboxd username saved; refreshing diary…".into());
                spawn_letterboxd_refresh(runtime.clone(), app.as_weak(), username);
            }
        });
    }
    {
        let app_weak = app.as_weak();
        app.on_open_letterboxd_link(move |url| {
            if let Err(error) = open_external_url(&url) {
                if let Some(app) = app_weak.upgrade() {
                    app.set_shell_notice(
                        format!("Couldn't open this Letterboxd entry: {error}").into(),
                    );
                }
            }
        });
    }

    // Now Playing (core::lastfm). A Last.fm username is a local-only
    // setting (see ui/app.slint's now-playing-state doc comment for the
    // real four-state machine this drives). "not-configured" is the honest
    // default whenever nothing's saved — set explicitly here rather than
    // relying on AppWindow's property default alone, so it's correct on
    // every load path (startup, and after clearing the field in Settings).
    let saved_lastfm_username = db
        .get_setting(LASTFM_USERNAME_SETTING_KEY)
        .unwrap_or_else(|error| {
            eprintln!("ankai-client: failed to read lastfm_username setting: {error}");
            None
        })
        .unwrap_or_default();
    app.set_lastfm_username(saved_lastfm_username.clone().into());
    if saved_lastfm_username.trim().is_empty() {
        app.set_now_playing_state("not-configured".into());
    } else {
        spawn_lastfm_refresh(
            p2p_runtime.handle().clone(),
            app.as_weak(),
            saved_lastfm_username,
        );
    }

    {
        let db_for_lastfm_save = db.clone();
        let app_weak_for_lastfm_save = app.as_weak();
        let handle_for_lastfm_save = p2p_runtime.handle().clone();
        app.on_save_lastfm_username(move |username| {
            let username = username.trim().to_string();
            let Some(app) = app_weak_for_lastfm_save.upgrade() else {
                return;
            };
            if let Err(error) =
                db_for_lastfm_save.set_setting(LASTFM_USERNAME_SETTING_KEY, &username)
            {
                eprintln!("ankai-client: failed to save Last.fm username: {error}");
                app.set_now_playing_state("error".into());
                app.set_now_playing_error(format!("Couldn't save username: {error}").into());
                app.set_shell_notice("Couldn't save the Last.fm username.".into());
                return;
            }
            app.set_lastfm_username(username.clone().into());
            if username.is_empty() {
                app.set_now_playing_state("not-configured".into());
                app.set_now_playing_artist("".into());
                app.set_now_playing_track("".into());
                app.set_now_playing_album("".into());
                app.set_now_playing_error("".into());
                app.set_shell_notice("Last.fm disconnected.".into());
            } else {
                app.set_shell_notice("Last.fm username saved; checking now playing…".into());
                spawn_lastfm_refresh(handle_for_lastfm_save.clone(), app.as_weak(), username);
            }
        });
    }

    // Real periodic re-check, not just a one-shot startup fetch: Last.fm's
    // nowplaying state changes as songs change, so this has to actually
    // keep polling to stay honest. Reads the *current* saved username fresh
    // on every tick (a plain, synchronous local-DB read — Db isn't Send,
    // but slint::Timer callbacks run on the UI thread, so this is exactly
    // as safe as any other direct `db` access in this file) rather than
    // capturing it once at setup time, so a username saved/cleared in
    // Settings takes effect on the very next tick without a restart. Kept
    // alive for the app's whole lifetime by binding it here, in the same
    // scope as `app.run()` below — a slint::Timer stops firing once
    // dropped, so this must not go out of scope before the event loop
    // starts.
    let db_for_lastfm_timer = db.clone();
    let app_weak_for_lastfm_timer = app.as_weak();
    let handle_for_lastfm_timer = p2p_runtime.handle().clone();
    let lastfm_timer = slint::Timer::default();
    lastfm_timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_secs(30),
        move || {
            let Some(app) = app_weak_for_lastfm_timer.upgrade() else {
                return;
            };
            let username = db_for_lastfm_timer
                .get_setting(LASTFM_USERNAME_SETTING_KEY)
                .unwrap_or_else(|error| {
                    eprintln!(
                        "ankai-client: failed to read lastfm_username setting during refresh: {error}"
                    );
                    None
                })
                .unwrap_or_default();
            if username.trim().is_empty() {
                // Stays "not-configured" — already set by the save/startup
                // path above, nothing to poll.
                return;
            }
            spawn_lastfm_refresh(handle_for_lastfm_timer.clone(), app.as_weak(), username);
        },
    );

    // Home is the default landing pane (see ui/app.slint's selected-index
    // doc comment) — set explicitly here too, rather than relying solely on
    // that property's initial value, so this stays correct even if
    // nav-items/home-index are reordered again later.
    app.set_selected_index(app.get_home_index());

    app.run()
}

#[cfg(test)]
mod request_generation_tests {
    use super::RequestGeneration;

    #[test]
    fn only_the_latest_issued_token_is_current() {
        let generation = RequestGeneration::new();
        let older = generation.issue();
        let newer = generation.issue();

        assert!(!generation.is_current(older));
        assert!(generation.is_current(newer));
    }

    #[test]
    fn generation_wraps_without_panicking_or_emitting_zero() {
        let generation = RequestGeneration(std::cell::Cell::new(u64::MAX));

        assert_eq!(generation.issue(), 1);
        assert!(generation.is_current(1));
    }
}

#[cfg(test)]
mod stream_ranking_tests {
    use super::{rank_streams_for_playability, stream_quality_tier, top_stream_is_playable};
    use ankai_core::stremio::Stream;

    fn direct(name: &str) -> Stream {
        Stream {
            url: Some("https://cdn.example.com/stream.mp4".into()),
            name: Some(name.into()),
            ..Stream::default()
        }
    }

    fn torrent(name: &str) -> Stream {
        Stream {
            info_hash: Some("a".repeat(40)),
            name: Some(name.into()),
            ..Stream::default()
        }
    }

    #[test]
    fn quality_tier_recognizes_common_resolution_tokens() {
        assert_eq!(stream_quality_tier(&direct("Torrentio\n2160p HEVC")), 0);
        assert_eq!(stream_quality_tier(&direct("Torrentio\n4K WEB-DL")), 0);
        assert_eq!(stream_quality_tier(&direct("Torrentio\n1080p")), 1);
        assert_eq!(stream_quality_tier(&direct("Torrentio\n720p")), 2);
        assert_eq!(stream_quality_tier(&direct("Torrentio\n480p")), 3);
        assert_eq!(stream_quality_tier(&direct("Torrentio\n360p")), 4);
        assert_eq!(stream_quality_tier(&direct("Unlabeled source")), 5);
    }

    #[test]
    fn direct_streams_always_rank_before_unplayable_ones_regardless_of_quality() {
        let mut streams = vec![torrent("Torrentio\n2160p HEVC"), direct("Provider\n480p")];
        rank_streams_for_playability(&mut streams);
        assert_eq!(streams[0].name.as_deref(), Some("Provider\n480p"));
    }

    #[test]
    fn among_direct_streams_highest_resolution_is_ranked_first() {
        let mut streams = vec![
            direct("Provider\n480p"),
            direct("Provider\n1080p"),
            direct("Provider\n720p"),
            direct("Provider\n2160p"),
        ];
        rank_streams_for_playability(&mut streams);
        let order: Vec<_> = streams.iter().map(|s| s.name.clone().unwrap()).collect();
        assert_eq!(
            order,
            vec![
                "Provider\n2160p",
                "Provider\n1080p",
                "Provider\n720p",
                "Provider\n480p",
            ]
        );
    }

    #[test]
    fn top_stream_is_playable_true_only_when_best_ranked_pick_is_direct() {
        let mut playable = vec![direct("Provider\n1080p")];
        rank_streams_for_playability(&mut playable);
        assert!(top_stream_is_playable(&playable));

        let mut unplayable = vec![torrent("Torrentio\n2160p")];
        rank_streams_for_playability(&mut unplayable);
        assert!(!top_stream_is_playable(&unplayable));

        assert!(!top_stream_is_playable(&[]));
    }
}
