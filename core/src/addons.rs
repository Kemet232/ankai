//! Persisted Stremio addon registry.
//!
//! The Stremio protocol client in [`crate::stremio`] deliberately keeps
//! catalog data live-only. This module persists only the user's installed
//! addon configuration and the last manifest-derived summary needed to draw
//! an addon manager: URL, identity, display metadata, declared resource
//! roles, enabled state, priority, and last-known health. Catalogs, titles,
//! metadata, streams, and image bytes remain live data and are never stored
//! here.
//!
//! Storage uses `Db`'s existing encrypted `settings` table. One versioned
//! JSON document is a better fit than a new relational table because this is
//! a single, small, ordered preference list owned by one local device. The
//! configured URL is canonicalized through [`crate::stremio::AddonClient`]
//! so `https://host/addon` and `https://host/addon/manifest.json` cannot be
//! installed twice. Crucially, configuration path segments are preserved;
//! two differently configured URLs are allowed even when their manifests
//! declare the same addon id.

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::Error;
use crate::stremio::{AddonClient, Manifest, Resource};

const ADDONS_SETTING_KEY: &str = "stremio_addons_v1";
const REGISTRY_FORMAT_VERSION: u8 = 1;

/// The protocol roles an addon's manifest declares.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ResourceRoles {
    pub catalog: bool,
    pub meta: bool,
    pub stream: bool,
    pub subtitles: bool,
}

impl ResourceRoles {
    /// Derives the roles from both string and descriptor resource forms.
    pub fn from_manifest(manifest: &Manifest) -> Self {
        let mut roles = Self {
            // A catalog declaration is useful evidence even when an
            // imperfect third-party manifest omits `"catalog"` from its
            // resource array.
            catalog: !manifest.catalogs.is_empty(),
            ..Self::default()
        };

        for resource in &manifest.resources {
            let name = match resource {
                Resource::Name(name) | Resource::Descriptor { name, .. } => name.as_str(),
            };
            match name {
                "catalog" => roles.catalog = true,
                "meta" => roles.meta = true,
                "stream" => roles.stream = true,
                "subtitles" => roles.subtitles = true,
                _ => {}
            }
        }
        roles
    }
}

/// Last-known outcome of contacting an installed addon.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddonHealthState {
    /// Installed state exists, but no network result has been recorded yet.
    #[default]
    Unknown,
    Healthy,
    Failing,
}

/// Health information suitable for a status badge and diagnostic detail.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AddonHealth {
    pub state: AddonHealthState,
    /// Unix time in seconds. `None` means the addon has not been checked.
    pub checked_at_unix: Option<u64>,
    /// Human-readable latest error. Healthy entries always clear this.
    pub last_error: Option<String>,
}

impl AddonHealth {
    pub fn healthy() -> Self {
        Self {
            state: AddonHealthState::Healthy,
            checked_at_unix: unix_now(),
            last_error: None,
        }
    }

    pub fn failing(error: impl Into<String>) -> Self {
        Self {
            state: AddonHealthState::Failing,
            checked_at_unix: unix_now(),
            last_error: Some(error.into()),
        }
    }
}

/// One locally installed Stremio addon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledAddon {
    /// Canonical, fully configured URL ending in `manifest.json`.
    pub manifest_url: String,
    pub addon_id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub roles: ResourceRoles,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    /// Zero-based precedence; lower values are queried first.
    #[serde(default)]
    pub priority: u32,
    #[serde(default)]
    pub health: AddonHealth,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredRegistry {
    version: u8,
    #[serde(default)]
    addons: Vec<InstalledAddon>,
}

/// Lists installed addons in query priority order.
pub fn list(db: &Db) -> Result<Vec<InstalledAddon>, Error> {
    read_registry(db)
}

/// Installs an addon from a manifest the caller has successfully fetched.
///
/// The successful fetch records healthy state. Duplicate canonical URLs are
/// rejected, while equal addon ids at different configured URLs are allowed.
pub fn add(db: &Db, manifest_url: &str, manifest: &Manifest) -> Result<InstalledAddon, Error> {
    validate_manifest(manifest)?;
    let manifest_url = canonical_manifest_url(manifest_url)?;
    let mut addons = read_registry(db)?;
    if addons
        .iter()
        .any(|addon| addon.manifest_url == manifest_url)
    {
        return Err(Error::Stremio(format!(
            "addon is already installed: {manifest_url}"
        )));
    }

    let addon = InstalledAddon {
        manifest_url,
        addon_id: manifest.id.clone(),
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        description: manifest.description.clone(),
        logo: manifest.logo.clone(),
        background: manifest.background.clone(),
        types: manifest.types.clone(),
        roles: ResourceRoles::from_manifest(manifest),
        enabled: true,
        priority: addons.len() as u32,
        health: AddonHealth::healthy(),
    };
    addons.push(addon.clone());
    write_registry(db, &mut addons)?;
    Ok(addon)
}

/// Refreshes manifest-derived display and capability fields without changing
/// enabled state or priority, and records a successful health check.
pub fn refresh_manifest(
    db: &Db,
    manifest_url: &str,
    manifest: &Manifest,
) -> Result<InstalledAddon, Error> {
    validate_manifest(manifest)?;
    mutate_one(db, manifest_url, |addon| {
        addon.addon_id = manifest.id.clone();
        addon.name = manifest.name.clone();
        addon.version = manifest.version.clone();
        addon.description = manifest.description.clone();
        addon.logo = manifest.logo.clone();
        addon.background = manifest.background.clone();
        addon.types = manifest.types.clone();
        addon.roles = ResourceRoles::from_manifest(manifest);
        addon.health = AddonHealth::healthy();
    })
}

/// Enables or disables an installed addon without losing its configuration.
pub fn set_enabled(db: &Db, manifest_url: &str, enabled: bool) -> Result<InstalledAddon, Error> {
    mutate_one(db, manifest_url, |addon| addon.enabled = enabled)
}

/// Records a failed health check while preserving the last valid manifest
/// summary, so an unavailable addon remains identifiable and recoverable.
pub fn mark_failing(
    db: &Db,
    manifest_url: &str,
    error: impl Into<String>,
) -> Result<InstalledAddon, Error> {
    let health = AddonHealth::failing(error);
    mutate_one(db, manifest_url, move |addon| addon.health = health)
}

/// Moves an addon to a zero-based priority index and returns the full new
/// ordering. Moving to its current index is a no-op.
pub fn move_to(
    db: &Db,
    manifest_url: &str,
    new_index: usize,
) -> Result<Vec<InstalledAddon>, Error> {
    let canonical_url = canonical_manifest_url(manifest_url)?;
    let mut addons = read_registry(db)?;
    if new_index >= addons.len() {
        return Err(Error::Stremio(format!(
            "addon priority index {new_index} is outside a list of {} addons",
            addons.len()
        )));
    }
    let current_index = addons
        .iter()
        .position(|addon| addon.manifest_url == canonical_url)
        .ok_or_else(|| addon_not_found(&canonical_url))?;
    if current_index != new_index {
        let addon = addons.remove(current_index);
        addons.insert(new_index, addon);
        write_registry(db, &mut addons)?;
    }
    Ok(addons)
}

/// Removes an addon. Returns `true` when an entry was removed and `false`
/// when the URL was not installed.
pub fn remove(db: &Db, manifest_url: &str) -> Result<bool, Error> {
    let canonical_url = canonical_manifest_url(manifest_url)?;
    let mut addons = read_registry(db)?;
    let old_len = addons.len();
    addons.retain(|addon| addon.manifest_url != canonical_url);
    let removed = old_len != addons.len();
    if removed {
        write_registry(db, &mut addons)?;
    }
    Ok(removed)
}

fn mutate_one(
    db: &Db,
    manifest_url: &str,
    mutate: impl FnOnce(&mut InstalledAddon),
) -> Result<InstalledAddon, Error> {
    let canonical_url = canonical_manifest_url(manifest_url)?;
    let mut addons = read_registry(db)?;
    let index = addons
        .iter()
        .position(|addon| addon.manifest_url == canonical_url)
        .ok_or_else(|| addon_not_found(&canonical_url))?;
    mutate(&mut addons[index]);
    let updated = addons[index].clone();
    write_registry(db, &mut addons)?;
    Ok(updated)
}

fn canonical_manifest_url(url: &str) -> Result<String, Error> {
    AddonClient::new(url.trim())?.manifest_url()
}

fn validate_manifest(manifest: &Manifest) -> Result<(), Error> {
    for (field, value) in [
        ("id", manifest.id.as_str()),
        ("name", manifest.name.as_str()),
        ("version", manifest.version.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(Error::Stremio(format!(
                "cannot install addon whose manifest has an empty {field}"
            )));
        }
    }
    Ok(())
}

fn read_registry(db: &Db) -> Result<Vec<InstalledAddon>, Error> {
    let Some(raw) = db.get_setting(ADDONS_SETTING_KEY)? else {
        return Ok(Vec::new());
    };
    let mut stored: StoredRegistry = serde_json::from_str(&raw)
        .map_err(|error| Error::Stremio(format!("installed addon registry is corrupt: {error}")))?;
    if stored.version != REGISTRY_FORMAT_VERSION {
        return Err(Error::Stremio(format!(
            "unsupported installed addon registry version {}",
            stored.version
        )));
    }
    validate_and_normalize(&mut stored.addons)?;
    Ok(stored.addons)
}

fn write_registry(db: &Db, addons: &mut [InstalledAddon]) -> Result<(), Error> {
    validate_and_normalize(addons)?;
    let stored = StoredRegistry {
        version: REGISTRY_FORMAT_VERSION,
        addons: addons.to_vec(),
    };
    let raw = serde_json::to_string(&stored)
        .map_err(|error| Error::Stremio(format!("failed to encode addon registry: {error}")))?;
    db.set_setting(ADDONS_SETTING_KEY, &raw)
}

fn validate_and_normalize(addons: &mut [InstalledAddon]) -> Result<(), Error> {
    let mut urls = HashSet::with_capacity(addons.len());
    for (index, addon) in addons.iter_mut().enumerate() {
        let canonical = canonical_manifest_url(&addon.manifest_url)?;
        if !urls.insert(canonical.clone()) {
            return Err(Error::Stremio(format!(
                "installed addon registry contains duplicate URL: {canonical}"
            )));
        }
        addon.manifest_url = canonical;
        addon.priority = index as u32;
        if addon.health.state == AddonHealthState::Healthy {
            addon.health.last_error = None;
        }
    }
    Ok(())
}

fn addon_not_found(url: &str) -> Error {
    Error::Stremio(format!("addon is not installed: {url}"))
}

fn enabled_by_default() -> bool {
    true
}

fn unix_now() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Db {
        Db::open_in_memory("correct horse battery staple").unwrap()
    }

    fn manifest(id: &str, name: &str) -> Manifest {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "name": name,
            "version": "1.2.3",
            "description": "A test addon",
            "logo": "https://images.example/addon.png",
            "resources": [
                "catalog",
                { "name": "meta", "types": ["movie", "series"] },
                "stream",
                "subtitles"
            ],
            "types": ["movie", "series"],
            "catalogs": [{"type": "series", "id": "anime", "name": "Anime"}]
        }))
        .unwrap()
    }

    #[test]
    fn add_persists_manifest_summary_roles_and_order() {
        let db = setup();
        let first = add(
            &db,
            "https://example.com/configured",
            &manifest("test.one", "First"),
        )
        .unwrap();
        let second = add(
            &db,
            "https://other.example/manifest.json",
            &manifest("test.two", "Second"),
        )
        .unwrap();

        assert_eq!(
            first.manifest_url,
            "https://example.com/configured/manifest.json"
        );
        assert_eq!(first.priority, 0);
        assert_eq!(second.priority, 1);
        assert!(first.enabled);
        assert_eq!(
            first.roles,
            ResourceRoles {
                catalog: true,
                meta: true,
                stream: true,
                subtitles: true,
            }
        );
        assert_eq!(
            first.logo.as_deref(),
            Some("https://images.example/addon.png")
        );
        assert_eq!(first.health.state, AddonHealthState::Healthy);
        assert!(first.health.checked_at_unix.is_some());
        assert_eq!(list(&db).unwrap(), vec![first, second]);
    }

    #[test]
    fn configured_urls_are_distinct_but_equivalent_urls_are_duplicates() {
        let db = setup();
        let shared = manifest("same.id", "Configured Addon");
        add(&db, "https://example.com/config-a", &shared).unwrap();
        add(&db, "https://example.com/config-b/manifest.json", &shared).unwrap();
        assert_eq!(list(&db).unwrap().len(), 2);

        let error = add(&db, "https://example.com/config-a/manifest.json", &shared).unwrap_err();
        assert!(error.to_string().contains("already installed"));
        assert_eq!(list(&db).unwrap().len(), 2);
    }

    #[test]
    fn disable_reorder_health_and_remove_round_trip() {
        let db = setup();
        add(&db, "https://one.example", &manifest("one", "One")).unwrap();
        add(&db, "https://two.example", &manifest("two", "Two")).unwrap();
        add(&db, "https://three.example", &manifest("three", "Three")).unwrap();

        let disabled = set_enabled(&db, "https://two.example", false).unwrap();
        assert!(!disabled.enabled);
        let failed = mark_failing(&db, "https://two.example", "HTTP 503").unwrap();
        assert_eq!(failed.health.state, AddonHealthState::Failing);
        assert_eq!(failed.health.last_error.as_deref(), Some("HTTP 503"));

        let reordered = move_to(&db, "https://three.example", 0).unwrap();
        assert_eq!(
            reordered
                .iter()
                .map(|addon| addon.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Three", "One", "Two"]
        );
        assert_eq!(
            reordered
                .iter()
                .map(|addon| addon.priority)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );

        assert!(remove(&db, "https://one.example/manifest.json").unwrap());
        assert!(!remove(&db, "https://not-installed.example").unwrap());
        let remaining = list(&db).unwrap();
        assert_eq!(
            remaining
                .iter()
                .map(|addon| addon.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Three", "Two"]
        );
        assert_eq!(
            remaining
                .iter()
                .map(|addon| addon.priority)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert!(!remaining[1].enabled);
        assert_eq!(remaining[1].health.state, AddonHealthState::Failing);
    }

    #[test]
    fn refresh_updates_manifest_fields_but_keeps_user_state() {
        let db = setup();
        add(&db, "https://example.com", &manifest("old.id", "Old Name")).unwrap();
        set_enabled(&db, "https://example.com", false).unwrap();
        mark_failing(&db, "https://example.com", "timeout").unwrap();

        let mut updated = manifest("new.id", "New Name");
        updated.version = "2.0.0".into();
        updated.resources = vec![Resource::Name("stream".into())];
        updated.catalogs.clear();
        let refreshed = refresh_manifest(&db, "https://example.com", &updated).unwrap();

        assert_eq!(refreshed.addon_id, "new.id");
        assert_eq!(refreshed.name, "New Name");
        assert_eq!(refreshed.version, "2.0.0");
        assert!(!refreshed.enabled);
        assert_eq!(refreshed.priority, 0);
        assert_eq!(
            refreshed.roles,
            ResourceRoles {
                stream: true,
                ..ResourceRoles::default()
            }
        );
        assert_eq!(refreshed.health.state, AddonHealthState::Healthy);
        assert_eq!(refreshed.health.last_error, None);
    }

    #[test]
    fn invalid_urls_and_corrupt_registry_are_not_silently_accepted() {
        let db = setup();
        assert!(add(&db, "file:///tmp/addon", &manifest("bad", "Bad")).is_err());

        db.set_setting(ADDONS_SETTING_KEY, "{not-json").unwrap();
        let error = list(&db).unwrap_err();
        assert!(error.to_string().contains("registry is corrupt"));
    }
}
