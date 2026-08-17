//! Bounded memory + disk stale-while-revalidate cache for Stremio catalog
//! pages (S21 of `docs/roadmaps/stremio-competitive-parity.md`).
//!
//! [`crate::board::fetch_catalog_page`] is the only intended caller: it
//! tries a live request first and falls back to whatever this module has
//! cached only when the live call fails. This module itself does not decide
//! freshness policy beyond "does a row exist" — see that function's doc
//! comment for the full policy.
//!
//! Two tiers exist because they serve different failure modes: the
//! in-memory tier ([`MemoryCatalogCache`]) survives only for the process's
//! lifetime and is checked first (cheap, no I/O); the disk tier persists in
//! the same SQLCipher-encrypted database every other feature table uses, so
//! a genuinely offline restart still has something to show instead of an
//! empty Board. Both are bounded and evict the least-recently-touched entry
//! first once full — this is metadata only (titles, posters, descriptions),
//! never stream URLs or credentials, but it is still real addon response
//! data and stays inside the encrypted database rather than a plaintext
//! cache file.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::Error;
use crate::stremio::MetaPreview;

/// Maximum rows kept in the on-disk cache. Evicted oldest-accessed-first.
const MAX_DISK_ENTRIES: usize = 300;
/// Maximum entries kept in the in-memory cache per process.
const DEFAULT_MEMORY_CAPACITY: usize = 64;

/// One cached catalog page plus when it was fetched and the addon's own
/// declared freshness window (if any).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedCatalogPage {
    pub items: Vec<MetaPreview>,
    pub fetched_at_unix: u64,
    /// The addon's `cacheMaxAge` hint (seconds), if it sent one. Currently
    /// recorded for future use (e.g. skipping a live request while still
    /// within this window); [`crate::board::fetch_catalog_page`] does not
    /// yet act on it — see that function's doc comment for why every fetch
    /// still tries live first.
    pub cache_max_age_secs: Option<u64>,
}

/// Builds the stable cache key one exact catalog request maps to: the
/// addon's canonical manifest URL, content type, catalog id, and a
/// sorted/normalized encoding of the extras (so `skip=20&genre=Action` and
/// `genre=Action&skip=20` share one entry).
pub fn cache_key(
    addon_url: &str,
    media_type: &str,
    catalog_id: &str,
    extra: &[(&str, &str)],
) -> String {
    let mut sorted: Vec<(&str, &str)> = extra.to_vec();
    sorted.sort_unstable();
    let extra_part = sorted
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    format!("{addon_url}\u{1}{media_type}\u{1}{catalog_id}\u{1}{extra_part}")
}

/// A bounded, process-lifetime in-memory cache tier.
pub struct MemoryCatalogCache {
    capacity: usize,
    entries: Mutex<(HashMap<String, CachedCatalogPage>, VecDeque<String>)>,
}

impl MemoryCatalogCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: Mutex::new((HashMap::new(), VecDeque::new())),
        }
    }

    pub fn get(&self, key: &str) -> Option<CachedCatalogPage> {
        let guard = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.0.get(key).cloned()
    }

    pub fn put(&self, key: String, page: CachedCatalogPage) {
        let mut guard = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (map, order) = &mut *guard;
        if !map.contains_key(&key) {
            order.push_back(key.clone());
            while map.len() >= self.capacity {
                if let Some(oldest) = order.pop_front() {
                    map.remove(&oldest);
                } else {
                    break;
                }
            }
        }
        map.insert(key, page);
    }

    /// Drops every entry belonging to one addon — used when that addon is
    /// removed from the registry, so a stale cache never outlives it.
    pub fn clear_for_addon(&self, addon_url: &str) {
        let mut guard = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (map, order) = &mut *guard;
        let prefix = format!("{addon_url}\u{1}");
        order.retain(|key| !key.starts_with(&prefix));
        map.retain(|key, _| !key.starts_with(&prefix));
    }
}

impl Default for MemoryCatalogCache {
    fn default() -> Self {
        Self::new(DEFAULT_MEMORY_CAPACITY)
    }
}

/// A disk-cache lookup result.
pub struct CacheLookup {
    pub page: CachedCatalogPage,
}

/// Reads one cached page from disk, if present, and refreshes its
/// last-accessed timestamp (best-effort — a failure to record the touch
/// does not fail the read).
pub fn get(db: &Db, key: &str) -> Result<Option<CacheLookup>, Error> {
    // SQLite (via rusqlite) only implements `FromSql`/`ToSql` for signed
    // integer types, so timestamps and the cache-age hint round-trip through
    // `i64` at the storage boundary and back to `u64` in the public API —
    // real Unix timestamps and second counts never approach `i64::MAX`.
    let row: Option<(String, Option<i64>, i64)> = db
        .connection()
        .query_row(
            "SELECT items_json, cache_max_age_secs, fetched_at FROM catalog_cache WHERE cache_key = ?1",
            [key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|e| Error::Db(format!("failed to read catalog cache: {e}")))?;
    let Some((items_json, cache_max_age_secs, fetched_at_unix)) = row else {
        return Ok(None);
    };
    let items: Vec<MetaPreview> = serde_json::from_str(&items_json)
        .map_err(|e| Error::Db(format!("catalog cache row is corrupt: {e}")))?;
    let _ = db.connection().execute(
        "UPDATE catalog_cache SET accessed_at = ?1 WHERE cache_key = ?2",
        rusqlite::params![unix_now() as i64, key],
    );
    Ok(Some(CacheLookup {
        page: CachedCatalogPage {
            items,
            fetched_at_unix: fetched_at_unix.max(0) as u64,
            cache_max_age_secs: cache_max_age_secs.map(|value| value.max(0) as u64),
        },
    }))
}

/// Writes (inserting or replacing) one cached page to disk, then evicts the
/// least-recently-accessed rows beyond [`MAX_DISK_ENTRIES`].
pub fn put(
    db: &Db,
    key: &str,
    addon_url: &str,
    media_type: &str,
    catalog_id: &str,
    page: &CachedCatalogPage,
) -> Result<(), Error> {
    let items_json = serde_json::to_string(&page.items)
        .map_err(|e| Error::Db(format!("failed to encode catalog cache page: {e}")))?;
    let now = unix_now();
    db.connection()
        .execute(
            "INSERT INTO catalog_cache
                (cache_key, addon_url, media_type, catalog_id, items_json, fetched_at, cache_max_age_secs, accessed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(cache_key) DO UPDATE SET
                items_json = excluded.items_json,
                fetched_at = excluded.fetched_at,
                cache_max_age_secs = excluded.cache_max_age_secs,
                accessed_at = excluded.accessed_at",
            rusqlite::params![
                key,
                addon_url,
                media_type,
                catalog_id,
                items_json,
                page.fetched_at_unix as i64,
                page.cache_max_age_secs.map(|value| value as i64),
                now as i64,
            ],
        )
        .map_err(|e| Error::Db(format!("failed to write catalog cache: {e}")))?;
    evict_excess(db)
}

/// Removes every cached row for one addon — call this when the addon is
/// removed from the registry.
pub fn clear_for_addon(db: &Db, addon_url: &str) -> Result<(), Error> {
    db.connection()
        .execute(
            "DELETE FROM catalog_cache WHERE addon_url = ?1",
            [addon_url],
        )
        .map_err(|e| Error::Db(format!("failed to clear catalog cache for addon: {e}")))?;
    Ok(())
}

fn evict_excess(db: &Db) -> Result<(), Error> {
    db.connection()
        .execute(
            "DELETE FROM catalog_cache WHERE cache_key IN (
                SELECT cache_key FROM catalog_cache
                ORDER BY accessed_at ASC
                LIMIT MAX(0, (SELECT COUNT(*) FROM catalog_cache) - ?1)
            )",
            [MAX_DISK_ENTRIES as i64],
        )
        .map_err(|e| Error::Db(format!("failed to evict catalog cache: {e}")))?;
    Ok(())
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

    fn setup() -> Db {
        Db::open_in_memory("correct horse battery staple").unwrap()
    }

    fn preview(id: &str) -> MetaPreview {
        serde_json::from_value(serde_json::json!({"id": id, "type": "movie", "name": id})).unwrap()
    }

    #[test]
    fn cache_key_normalizes_extra_order() {
        let a = cache_key(
            "https://addon.example",
            "movie",
            "top",
            &[("skip", "20"), ("genre", "Action")],
        );
        let b = cache_key(
            "https://addon.example",
            "movie",
            "top",
            &[("genre", "Action"), ("skip", "20")],
        );
        assert_eq!(a, b);
        let different = cache_key("https://addon.example", "movie", "top", &[("skip", "40")]);
        assert_ne!(a, different);
    }

    #[test]
    fn disk_cache_round_trips_and_survives_a_miss() {
        let db = setup();
        assert!(get(&db, "missing").unwrap().is_none());

        let key = cache_key("https://addon.example", "movie", "top", &[]);
        let page = CachedCatalogPage {
            items: vec![preview("1"), preview("2")],
            fetched_at_unix: 1_000,
            cache_max_age_secs: Some(300),
        };
        put(&db, &key, "https://addon.example", "movie", "top", &page).unwrap();

        let looked_up = get(&db, &key).unwrap().expect("cache entry should exist");
        assert_eq!(looked_up.page.items.len(), 2);
        assert_eq!(looked_up.page.fetched_at_unix, 1_000);
        assert_eq!(looked_up.page.cache_max_age_secs, Some(300));
    }

    #[test]
    fn disk_cache_overwrites_on_conflict() {
        let db = setup();
        let key = cache_key("https://addon.example", "movie", "top", &[]);
        let first = CachedCatalogPage {
            items: vec![preview("1")],
            fetched_at_unix: 1_000,
            cache_max_age_secs: None,
        };
        put(&db, &key, "https://addon.example", "movie", "top", &first).unwrap();
        let second = CachedCatalogPage {
            items: vec![preview("1"), preview("2")],
            fetched_at_unix: 2_000,
            cache_max_age_secs: None,
        };
        put(&db, &key, "https://addon.example", "movie", "top", &second).unwrap();

        let looked_up = get(&db, &key).unwrap().unwrap();
        assert_eq!(looked_up.page.items.len(), 2);
        assert_eq!(looked_up.page.fetched_at_unix, 2_000);
    }

    #[test]
    fn disk_cache_evicts_least_recently_accessed_beyond_capacity() {
        let db = setup();
        for i in 0..(MAX_DISK_ENTRIES + 5) {
            let key = cache_key("https://addon.example", "movie", &format!("cat{i}"), &[]);
            let page = CachedCatalogPage {
                items: vec![preview("1")],
                fetched_at_unix: i as u64,
                cache_max_age_secs: None,
            };
            put(
                &db,
                &key,
                "https://addon.example",
                "movie",
                &format!("cat{i}"),
                &page,
            )
            .unwrap();
        }
        let count: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM catalog_cache", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count as usize, MAX_DISK_ENTRIES);
        // The earliest-inserted (and never re-accessed) entry should be gone.
        let first_key = cache_key("https://addon.example", "movie", "cat0", &[]);
        assert!(get(&db, &first_key).unwrap().is_none());
    }

    #[test]
    fn clear_for_addon_removes_only_that_addons_rows() {
        let db = setup();
        let key_a = cache_key("https://a.example", "movie", "top", &[]);
        let key_b = cache_key("https://b.example", "movie", "top", &[]);
        let page = CachedCatalogPage {
            items: vec![preview("1")],
            fetched_at_unix: 1,
            cache_max_age_secs: None,
        };
        put(&db, &key_a, "https://a.example", "movie", "top", &page).unwrap();
        put(&db, &key_b, "https://b.example", "movie", "top", &page).unwrap();

        clear_for_addon(&db, "https://a.example").unwrap();
        assert!(get(&db, &key_a).unwrap().is_none());
        assert!(get(&db, &key_b).unwrap().is_some());
    }

    #[test]
    fn memory_cache_evicts_oldest_beyond_capacity() {
        let cache = MemoryCatalogCache::new(2);
        let page = |n: u64| CachedCatalogPage {
            items: vec![preview("1")],
            fetched_at_unix: n,
            cache_max_age_secs: None,
        };
        cache.put("a".into(), page(1));
        cache.put("b".into(), page(2));
        cache.put("c".into(), page(3));
        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_some());
        assert!(cache.get("c").is_some());
    }

    #[test]
    fn memory_cache_clear_for_addon() {
        let cache = MemoryCatalogCache::new(8);
        let key_a = cache_key("https://a.example", "movie", "top", &[]);
        let key_b = cache_key("https://b.example", "movie", "top", &[]);
        let page = CachedCatalogPage {
            items: vec![preview("1")],
            fetched_at_unix: 1,
            cache_max_age_secs: None,
        };
        cache.put(key_a.clone(), page.clone());
        cache.put(key_b.clone(), page);
        cache.clear_for_addon("https://a.example");
        assert!(cache.get(&key_a).is_none());
        assert!(cache.get(&key_b).is_some());
    }
}
