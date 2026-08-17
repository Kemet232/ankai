//! Catalog-aware Board/Discover model (S21 of
//! `docs/roadmaps/stremio-competitive-parity.md`).
//!
//! S20 gave ANKAI a typed, capability-routed Stremio protocol client. Until
//! now the client layered exactly one behavior on top of it: merge every
//! addon's default (or search) catalog into one flat shelf, tagged only with
//! which addon index it came from. That collapses real protocol structure —
//! an addon can expose many distinct catalogs per content type, each with
//! its own name, genre list, and required/optional extras — into "whatever
//! the first catalog returned."
//!
//! This module replaces that shelf with an explicit catalog-aware model:
//!
//! - [`board_catalogs`] enumerates every catalog on every enabled addon, in
//!   persisted addon-priority order, and classifies each one as a passive
//!   Home/Board row (no required extra — safe to load with no user input)
//!   or an extra-gated row (for example a search-only catalog, or one that
//!   requires picking a genre first).
//! - [`available_types`], [`available_addons`], and [`Catalog::genre_options`]
//!   (via [`genre_options`]) are the manifest-derived data the type/catalog/
//!   genre/addon selectors are built from — nothing here is hand-maintained.
//! - [`CatalogPageState`] is a small per-catalog state machine for `skip`
//!   pagination: it tracks the next `skip` value, in-catalog dedupe by
//!   `(type, id)`, and an honest `has_more` flag (Stremio's protocol has no
//!   total-count field, so "the last page returned zero items" is the only
//!   real end-of-catalog signal this crate can observe).
//! - [`dedupe_attributed`] applies the same `(type, id)` dedupe across a
//!   *merged* list (global search results spanning several catalogs/addons),
//!   preserving whichever source was inserted first — callers insert in
//!   addon-priority order, so the highest-priority addon's attribution wins.
//! - [`fetch_catalog_page`] is the actual network+cache orchestration: try
//!   live, cache a success, and fall back to the bounded memory/disk cache
//!   (see [`crate::catalog_cache`]) — with an honest last-fetched timestamp
//!   and the live error — when the network call fails. See that function's
//!   doc comment for the exact stale-while-revalidate policy this crate
//!   implements, which is deliberately simpler than "always serve cache
//!   instantly, patch in the background": every board/search interaction
//!   still tries live first, and the cache exists to make failure and
//!   offline use honest and usable rather than empty.

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::addons::InstalledAddon;
use crate::catalog_cache::{self, CachedCatalogPage, MemoryCatalogCache};
use crate::db::Db;
use crate::stremio::{AddonClient, Catalog, CatalogExtra, Manifest, MetaPreview};

/// Identifies exactly one catalog on exactly one installed addon.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BoardCatalogKey {
    pub addon_manifest_url: String,
    pub media_type: String,
    pub catalog_id: String,
}

/// One catalog, enriched with the addon/manifest context a Board/Discover UI
/// needs to render and query it.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardCatalogRef {
    pub key: BoardCatalogKey,
    pub addon_name: String,
    pub addon_priority: u32,
    pub catalog_name: String,
    pub media_type: String,
    pub genres: Vec<String>,
    pub extra: Vec<CatalogExtra>,
    /// True when this catalog can be shown as a passive Home/Board row with
    /// no user input — i.e. it declares no `isRequired` extra.
    pub is_passive: bool,
}

impl BoardCatalogRef {
    /// True when this catalog declares a `search` extra and every *other*
    /// declared extra is optional — the only shape a global cross-catalog
    /// search can safely satisfy, since a search query only ever supplies
    /// one extra value.
    pub fn supports_global_search(&self) -> bool {
        self.extra.iter().any(|extra| extra.name == "search")
            && self
                .extra
                .iter()
                .all(|extra| !extra.is_required || extra.name == "search")
    }
}

/// Enumerates every catalog on every enabled, catalog-capable addon, in
/// persisted addon-priority order (then manifest catalog-declaration order).
///
/// `manifests` pairs each live-loaded addon's canonical manifest URL with
/// its fetched [`Manifest`] — the same key [`InstalledAddon::manifest_url`]
/// uses, so an addon that is installed but not yet (re)loaded this session
/// simply contributes no rows rather than guessing at stale data.
pub fn board_catalogs(
    addons: &[InstalledAddon],
    manifests: &[(String, Manifest)],
) -> Vec<BoardCatalogRef> {
    let mut ordered: Vec<&InstalledAddon> = addons.iter().filter(|a| a.enabled).collect();
    ordered.sort_by_key(|a| a.priority);

    let mut out = Vec::new();
    for addon in ordered {
        let Some((_, manifest)) = manifests.iter().find(|(url, _)| *url == addon.manifest_url)
        else {
            continue;
        };
        for catalog in &manifest.catalogs {
            out.push(BoardCatalogRef {
                key: BoardCatalogKey {
                    addon_manifest_url: addon.manifest_url.clone(),
                    media_type: catalog.media_type.clone(),
                    catalog_id: catalog.id.clone(),
                },
                addon_name: addon.name.clone(),
                addon_priority: addon.priority,
                catalog_name: catalog.name.clone().unwrap_or_else(|| catalog.id.clone()),
                media_type: catalog.media_type.clone(),
                genres: genre_options(catalog),
                extra: catalog.extra.clone(),
                is_passive: !catalog.extra.iter().any(|extra| extra.is_required),
            });
        }
    }
    out
}

/// The genre options a catalog declares, preferring the modern `extra`
/// options list (the `genre` extra's `options` array) and falling back to
/// the deprecated top-level `catalog.genres` field some addons still send.
pub fn genre_options(catalog: &Catalog) -> Vec<String> {
    if let Some(extra) = catalog.extra.iter().find(|extra| extra.name == "genre") {
        if !extra.options.is_empty() {
            return extra.options.clone();
        }
    }
    catalog.genres.clone()
}

/// Content types present across `catalogs`, in first-seen order — the data
/// source for a Board type selector.
pub fn available_types(catalogs: &[BoardCatalogRef]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for catalog in catalogs {
        if seen.insert(catalog.media_type.clone()) {
            out.push(catalog.media_type.clone());
        }
    }
    out
}

/// `(manifest_url, addon_name)` pairs present across `catalogs`, in
/// addon-priority order — the data source for a Board addon selector.
pub fn available_addons(catalogs: &[BoardCatalogRef]) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for catalog in catalogs {
        if seen.insert(catalog.key.addon_manifest_url.clone()) {
            out.push((
                catalog.key.addon_manifest_url.clone(),
                catalog.addon_name.clone(),
            ));
        }
    }
    out
}

/// Catalogs eligible for a global cross-addon search request: they declare
/// `search` and no other required extra (see
/// [`BoardCatalogRef::supports_global_search`]).
pub fn search_eligible_catalogs(catalogs: &[BoardCatalogRef]) -> Vec<&BoardCatalogRef> {
    catalogs
        .iter()
        .filter(|catalog| catalog.supports_global_search())
        .collect()
}

/// Per-catalog `skip`-pagination state.
#[derive(Debug, Clone, Default)]
pub struct CatalogPageState {
    /// Accumulated items across every page fetched so far, deduped within
    /// this one catalog by `(type, id)`.
    pub items: Vec<MetaPreview>,
    /// The `skip` value the next page request should use.
    pub next_skip: usize,
    pub loading: bool,
    pub error: Option<String>,
    /// False once a fetched page came back empty — Stremio's protocol has no
    /// total-count field, so this is the only reliable end-of-catalog signal.
    pub has_more: bool,
    /// True once at least one page has been applied (successfully or not) —
    /// lets a UI distinguish "never loaded" from "loaded and empty".
    pub initialized: bool,
    /// Set when the most recently applied page came from the offline/stale
    /// cache rather than a live fetch (see [`fetch_catalog_page`]).
    pub cache_notice: Option<CacheNotice>,
}

/// Disclosure shown alongside a page served from the stale-while-revalidate
/// cache instead of a live response.
#[derive(Debug, Clone, PartialEq)]
pub struct CacheNotice {
    pub fetched_at_unix: u64,
    pub live_error: String,
}

impl CatalogPageState {
    pub fn new() -> Self {
        Self {
            has_more: true,
            ..Self::default()
        }
    }

    /// Applies a freshly fetched (live or cache-fallback) page.
    pub fn apply_page(&mut self, page: Vec<MetaPreview>, cache_notice: Option<CacheNotice>) {
        self.loading = false;
        self.error = None;
        self.has_more = !page.is_empty();
        self.next_skip += page.len();
        self.initialized = true;
        self.cache_notice = cache_notice;
        let mut seen: HashSet<(String, String)> = self
            .items
            .iter()
            .map(|item| (item.media_type.clone(), item.id.clone()))
            .collect();
        for item in page {
            let key = (item.media_type.clone(), item.id.clone());
            if seen.insert(key) {
                self.items.push(item);
            }
        }
    }

    /// Records a page fetch that failed with no cache fallback available.
    pub fn apply_error(&mut self, message: String) {
        self.loading = false;
        self.error = Some(message);
    }
}

/// One item merged into a cross-catalog list (global search), attributed to
/// the addon/catalog it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributedItem {
    pub item: MetaPreview,
    pub addon_manifest_url: String,
    pub addon_name: String,
    pub catalog_id: String,
}

/// Deduplicates a merged list by `(type, id)`, keeping the first occurrence
/// of each — callers insert in addon-priority order, so the first occurrence
/// is always the highest-priority addon's attribution.
pub fn dedupe_attributed(items: Vec<AttributedItem>) -> Vec<AttributedItem> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|entry| seen.insert((entry.item.media_type.clone(), entry.item.id.clone())))
        .collect()
}

/// The outcome of one catalog-page fetch attempt.
#[derive(Debug, Clone, PartialEq)]
pub enum CatalogFetchOutcome {
    /// A live response was fetched and (best-effort) cached.
    Live(Vec<MetaPreview>),
    /// The live fetch failed; a memory or disk cache entry was served
    /// instead, along with when it was fetched and why the live call failed.
    CachedFallback {
        items: Vec<MetaPreview>,
        fetched_at_unix: u64,
        live_error: String,
    },
    /// The live fetch failed and no cache entry exists for this exact
    /// catalog/extras key.
    Failed(String),
}

/// Fetches one catalog page, applying ANKAI's stale-while-revalidate policy:
///
/// 1. Always attempt a live request first — a cache is a fallback, not a
///    substitute for fresh data.
/// 2. On success, write the page into the bounded in-memory cache and the
///    bounded on-disk cache (best-effort: a cache write failure does not
///    fail the fetch) and return [`CatalogFetchOutcome::Live`].
/// 3. On failure, check the in-memory cache, then the on-disk cache, for the
///    exact same addon/type/catalog/extras key. If found, return
///    [`CatalogFetchOutcome::CachedFallback`] with the cached items, when
///    they were fetched, and the live error — an honest "offline/last known
///    good" disclosure rather than a silent stale success. If nothing is
///    cached, return [`CatalogFetchOutcome::Failed`].
///
/// This intentionally does not implement "serve stale instantly, then patch
/// in the background": every call still pays for one live attempt. That
/// tradeoff favors freshness for a Board/Discover surface where the user is
/// actively browsing, at the cost of a cache hit never being faster than a
/// live success. Revisit if this proves too slow on a flaky connection.
#[allow(clippy::too_many_arguments)]
pub async fn fetch_catalog_page(
    memory: &MemoryCatalogCache,
    db: &Db,
    client: &AddonClient,
    addon_url: &str,
    media_type: &str,
    catalog_id: &str,
    extra: &[(&str, &str)],
) -> CatalogFetchOutcome {
    let key = catalog_cache::cache_key(addon_url, media_type, catalog_id, extra);
    match client
        .catalog_page_with_extra(media_type, catalog_id, extra)
        .await
    {
        Ok(page) => {
            let cached = CachedCatalogPage {
                items: page.metas.clone(),
                fetched_at_unix: unix_now(),
                cache_max_age_secs: page.cache_max_age,
            };
            memory.put(key.clone(), cached.clone());
            let _ = catalog_cache::put(db, &key, addon_url, media_type, catalog_id, &cached);
            CatalogFetchOutcome::Live(page.metas)
        }
        Err(err) => {
            if let Some(cached) = memory.get(&key) {
                return CatalogFetchOutcome::CachedFallback {
                    items: cached.items,
                    fetched_at_unix: cached.fetched_at_unix,
                    live_error: err.to_string(),
                };
            }
            match catalog_cache::get(db, &key) {
                Ok(Some(lookup)) => CatalogFetchOutcome::CachedFallback {
                    items: lookup.page.items,
                    fetched_at_unix: lookup.page.fetched_at_unix,
                    live_error: err.to_string(),
                },
                _ => CatalogFetchOutcome::Failed(err.to_string()),
            }
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::addons::ResourceRoles;
    use crate::stremio::{Catalog, ManifestBehaviorHints};

    fn addon(url: &str, id: &str, name: &str, priority: u32) -> InstalledAddon {
        InstalledAddon {
            manifest_url: url.into(),
            addon_id: id.into(),
            name: name.into(),
            version: "1.0.0".into(),
            description: None,
            logo: None,
            background: None,
            types: vec!["movie".into()],
            roles: ResourceRoles {
                catalog: true,
                ..Default::default()
            },
            enabled: true,
            priority,
            health: Default::default(),
        }
    }

    fn manifest(id: &str, catalogs: Vec<Catalog>) -> Manifest {
        Manifest {
            id: id.into(),
            name: id.into(),
            version: "1.0.0".into(),
            description: None,
            logo: None,
            background: None,
            resources: vec![],
            types: vec!["movie".into()],
            id_prefixes: vec![],
            catalogs,
            addon_catalogs: vec![],
            contact_email: None,
            behavior_hints: ManifestBehaviorHints::default(),
            config: vec![],
        }
    }

    fn catalog(media_type: &str, id: &str, name: &str, extra: Vec<CatalogExtra>) -> Catalog {
        Catalog {
            media_type: media_type.into(),
            id: id.into(),
            name: Some(name.into()),
            genres: vec![],
            extra,
        }
    }

    fn required_extra(name: &str, options: Vec<&str>) -> CatalogExtra {
        CatalogExtra {
            name: name.into(),
            options: options.into_iter().map(String::from).collect(),
            is_required: true,
            options_limit: None,
        }
    }

    fn optional_extra(name: &str) -> CatalogExtra {
        CatalogExtra {
            name: name.into(),
            options: vec![],
            is_required: false,
            options_limit: None,
        }
    }

    #[test]
    fn board_catalogs_are_ordered_by_addon_priority_then_declaration() {
        let addons = vec![
            addon("https://b.example/manifest.json", "b", "B", 1),
            addon("https://a.example/manifest.json", "a", "A", 0),
        ];
        let manifests = vec![
            (
                "https://b.example/manifest.json".to_string(),
                manifest("b", vec![catalog("movie", "top", "Top", vec![])]),
            ),
            (
                "https://a.example/manifest.json".to_string(),
                manifest(
                    "a",
                    vec![
                        catalog("movie", "popular", "Popular", vec![]),
                        catalog("series", "new", "New", vec![]),
                    ],
                ),
            ),
        ];
        let board = board_catalogs(&addons, &manifests);
        assert_eq!(
            board
                .iter()
                .map(|c| (c.addon_name.as_str(), c.key.catalog_id.as_str()))
                .collect::<Vec<_>>(),
            vec![("A", "popular"), ("A", "new"), ("B", "top")]
        );
    }

    #[test]
    fn disabled_addons_and_unmatched_manifests_are_excluded() {
        let mut disabled = addon("https://c.example/manifest.json", "c", "C", 0);
        disabled.enabled = false;
        let addons = vec![
            disabled,
            addon("https://d.example/manifest.json", "d", "D", 1),
        ];
        let manifests = vec![(
            "https://c.example/manifest.json".to_string(),
            manifest("c", vec![catalog("movie", "top", "Top", vec![])]),
        )];
        let board = board_catalogs(&addons, &manifests);
        assert!(board.is_empty());
    }

    #[test]
    fn passive_rows_exclude_catalogs_with_required_extras() {
        let addons = vec![addon("https://a.example/manifest.json", "a", "A", 0)];
        let manifests = vec![(
            "https://a.example/manifest.json".to_string(),
            manifest(
                "a",
                vec![
                    catalog("movie", "top", "Top", vec![optional_extra("skip")]),
                    catalog(
                        "movie",
                        "search",
                        "Search",
                        vec![required_extra("search", vec![])],
                    ),
                ],
            ),
        )];
        let board = board_catalogs(&addons, &manifests);
        assert!(board[0].is_passive);
        assert!(!board[1].is_passive);
    }

    #[test]
    fn global_search_eligibility_requires_search_to_be_the_only_required_extra() {
        let search_only = BoardCatalogRef {
            key: BoardCatalogKey {
                addon_manifest_url: "u".into(),
                media_type: "movie".into(),
                catalog_id: "s".into(),
            },
            addon_name: "A".into(),
            addon_priority: 0,
            catalog_name: "Search".into(),
            media_type: "movie".into(),
            extra: vec![required_extra("search", vec![])],
            genres: vec![],
            is_passive: false,
        };
        assert!(search_only.supports_global_search());

        let mut needs_extra = search_only.clone();
        needs_extra.extra.push(required_extra("region", vec!["us"]));
        assert!(!needs_extra.supports_global_search());

        let mut no_search = search_only.clone();
        no_search.extra = vec![optional_extra("genre")];
        assert!(!no_search.supports_global_search());
    }

    #[test]
    fn genre_options_prefer_extra_declaration_over_legacy_field() {
        let mut c = catalog(
            "movie",
            "top",
            "Top",
            vec![CatalogExtra {
                name: "genre".into(),
                options: vec!["Action".into(), "Comedy".into()],
                is_required: false,
                options_limit: None,
            }],
        );
        assert_eq!(genre_options(&c), vec!["Action", "Comedy"]);

        c.extra.clear();
        c.genres = vec!["Legacy".into()];
        assert_eq!(genre_options(&c), vec!["Legacy"]);
    }

    #[test]
    fn available_types_and_addons_are_deduped_and_ordered() {
        let addons = vec![
            addon("https://a.example/manifest.json", "a", "A", 0),
            addon("https://b.example/manifest.json", "b", "B", 1),
        ];
        let manifests = vec![
            (
                "https://a.example/manifest.json".to_string(),
                manifest(
                    "a",
                    vec![
                        catalog("movie", "top", "Top", vec![]),
                        catalog("series", "top", "Top", vec![]),
                    ],
                ),
            ),
            (
                "https://b.example/manifest.json".to_string(),
                manifest("b", vec![catalog("movie", "hot", "Hot", vec![])]),
            ),
        ];
        let board = board_catalogs(&addons, &manifests);
        assert_eq!(available_types(&board), vec!["movie", "series"]);
        assert_eq!(
            available_addons(&board),
            vec![
                (
                    "https://a.example/manifest.json".to_string(),
                    "A".to_string()
                ),
                (
                    "https://b.example/manifest.json".to_string(),
                    "B".to_string()
                ),
            ]
        );
    }

    fn preview(media_type: &str, id: &str, name: &str) -> MetaPreview {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "type": media_type,
            "name": name,
        }))
        .unwrap()
    }

    #[test]
    fn catalog_page_state_dedupes_within_a_catalog_and_tracks_skip() {
        let mut state = CatalogPageState::new();
        assert!(state.has_more);
        state.apply_page(
            vec![preview("movie", "1", "One"), preview("movie", "2", "Two")],
            None,
        );
        assert_eq!(state.next_skip, 2);
        assert!(state.has_more);
        assert_eq!(state.items.len(), 2);

        // A page that re-sends an already-seen id (real addons sometimes do
        // this near a page boundary) must not duplicate the item.
        state.apply_page(
            vec![preview("movie", "2", "Two"), preview("movie", "3", "Three")],
            None,
        );
        assert_eq!(state.next_skip, 4);
        assert_eq!(state.items.len(), 3);

        state.apply_page(vec![], None);
        assert!(!state.has_more);
        assert!(state.initialized);
    }

    #[test]
    fn dedupe_attributed_keeps_first_occurrence_across_catalogs() {
        let items = vec![
            AttributedItem {
                item: preview("movie", "shared", "Shared"),
                addon_manifest_url: "https://high.example".into(),
                addon_name: "High priority".into(),
                catalog_id: "top".into(),
            },
            AttributedItem {
                item: preview("movie", "shared", "Shared (dup)"),
                addon_manifest_url: "https://low.example".into(),
                addon_name: "Low priority".into(),
                catalog_id: "hot".into(),
            },
            AttributedItem {
                item: preview("series", "shared", "Different type, same id"),
                addon_manifest_url: "https://low.example".into(),
                addon_name: "Low priority".into(),
                catalog_id: "hot".into(),
            },
        ];
        let deduped = dedupe_attributed(items);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].addon_name, "High priority");
        assert_eq!(deduped[1].item.media_type, "series");
    }
}
