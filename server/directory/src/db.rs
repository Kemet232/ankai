//! SQLite-backed storage for the directory server, per
//! `docs/adr/0008-identity-discovery-service.md`'s "Storage" and
//! "Staleness / expiry / consumption" sections.
//!
//! Unencrypted, deliberately — unlike `ankai_core::db`'s SQLCipher-backed
//! client-side DB, nothing this stores is confidential (see the ADR).
//!
//! `Storage`'s methods are synchronous (`rusqlite` has no async story);
//! callers (see `crate::app`) are expected to run them inside
//! `tokio::task::spawn_blocking` rather than call them directly from an
//! async context, per the ADR's "why not sqlx" reasoning.

use std::sync::Mutex;

use rusqlite::Connection;

use crate::auth::AuthError;

/// `KeyPackage`s older than this are excluded from lookups (and get
/// deleted the next time storage happens to touch that device's rows),
/// even if never consumed — bounds storage growth from devices that
/// published and were never contacted. See the ADR.
pub const KEY_PACKAGE_MAX_AGE_SECS: i64 = 30 * 24 * 60 * 60;

/// `EndpointAddr`s older than this are treated as stale and not returned by
/// lookups. See the ADR.
pub const ENDPOINT_ADDR_TTL_SECS: i64 = 10 * 60;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("storage error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("storage lock poisoned")]
    LockPoisoned,
}

pub struct Storage {
    conn: Mutex<Connection>,
}

impl Storage {
    /// Opens (creating if needed) a SQLite database at `path` and ensures
    /// its schema exists. Use `":memory:"` for tests/ephemeral use.
    pub fn open(path: &str) -> Result<Self, StorageError> {
        let conn = Connection::open(path)?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn init_schema(conn: &Connection) -> Result<(), StorageError> {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS device_keys (
                device_id TEXT PRIMARY KEY,
                public_key_hex TEXT NOT NULL,
                first_seen_unix INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS key_packages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                device_id TEXT NOT NULL,
                key_package_json TEXT NOT NULL,
                published_at_unix INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS key_packages_device_id ON key_packages(device_id);
            CREATE TABLE IF NOT EXISTS endpoint_addrs (
                device_id TEXT PRIMARY KEY,
                addr_json TEXT NOT NULL,
                published_at_unix INTEGER NOT NULL
            );
            ",
        )?;
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, StorageError> {
        self.conn.lock().map_err(|_| StorageError::LockPoisoned)
    }

    /// Enforces TOFU pubkey pinning (see the ADR's "Auth" section): the
    /// first call for a given `device_id` binds `public_key_hex` to it;
    /// every later call must present the same key or is rejected.
    pub fn bind_device_key(
        &self,
        device_id: &str,
        public_key_hex: &str,
        now: i64,
    ) -> Result<(), AuthError> {
        let conn = self.lock().map_err(|_| AuthError::DeviceKeyMismatch)?;
        let existing: Option<String> = conn
            .query_row(
                "SELECT public_key_hex FROM device_keys WHERE device_id = ?1",
                [device_id],
                |row| row.get(0),
            )
            .ok();

        match existing {
            Some(bound) if bound == public_key_hex => Ok(()),
            Some(_) => Err(AuthError::DeviceKeyMismatch),
            None => {
                conn.execute(
                    "INSERT INTO device_keys (device_id, public_key_hex, first_seen_unix) VALUES (?1, ?2, ?3)",
                    rusqlite::params![device_id, public_key_hex, now],
                )
                .map_err(|_| AuthError::DeviceKeyMismatch)?;
                Ok(())
            }
        }
    }

    pub fn insert_key_package(
        &self,
        device_id: &str,
        key_package_json: &str,
        now: i64,
    ) -> Result<(), StorageError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO key_packages (device_id, key_package_json, published_at_unix) VALUES (?1, ?2, ?3)",
            rusqlite::params![device_id, key_package_json, now],
        )?;
        Ok(())
    }

    /// Returns every currently-published, non-expired `KeyPackage` (as
    /// stored JSON strings, oldest first) for `device_id`, and **deletes
    /// those exact rows** — lookup consumes what it returns, per the ADR.
    /// Also opportunistically deletes any rows that are expired
    /// (older than `max_age_secs`) but weren't returned, so expired,
    /// never-consumed rows don't linger forever once *something* touches
    /// that device's rows.
    pub fn take_key_packages(
        &self,
        device_id: &str,
        now: i64,
        max_age_secs: i64,
    ) -> Result<Vec<String>, StorageError> {
        let conn = self.lock()?;
        let cutoff = now - max_age_secs;

        conn.execute(
            "DELETE FROM key_packages WHERE device_id = ?1 AND published_at_unix <= ?2",
            rusqlite::params![device_id, cutoff],
        )?;

        let mut stmt = conn.prepare(
            "SELECT id, key_package_json FROM key_packages WHERE device_id = ?1 ORDER BY id ASC",
        )?;
        let rows: Vec<(i64, String)> = stmt
            .query_map([device_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        drop(stmt);

        for (id, _) in &rows {
            conn.execute("DELETE FROM key_packages WHERE id = ?1", [id])?;
        }

        Ok(rows.into_iter().map(|(_, json)| json).collect())
    }

    pub fn upsert_endpoint_addr(
        &self,
        device_id: &str,
        addr_json: &str,
        now: i64,
    ) -> Result<(), StorageError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO endpoint_addrs (device_id, addr_json, published_at_unix) VALUES (?1, ?2, ?3)
             ON CONFLICT(device_id) DO UPDATE SET addr_json = excluded.addr_json, published_at_unix = excluded.published_at_unix",
            rusqlite::params![device_id, addr_json, now],
        )?;
        Ok(())
    }

    /// The current `EndpointAddr` JSON for `device_id`, or `None` if it's
    /// never published one or its last publish is older than `ttl_secs`.
    pub fn get_endpoint_addr(
        &self,
        device_id: &str,
        now: i64,
        ttl_secs: i64,
    ) -> Result<Option<String>, StorageError> {
        let conn = self.lock()?;
        let row: Option<(String, i64)> = conn
            .query_row(
                "SELECT addr_json, published_at_unix FROM endpoint_addrs WHERE device_id = ?1",
                [device_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();

        Ok(match row {
            Some((addr_json, published_at)) if now - published_at <= ttl_secs => Some(addr_json),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_key_binding_is_tofu_pinned() {
        let storage = Storage::open(":memory:").unwrap();
        storage.bind_device_key("d1", "aabb", 100).unwrap();
        // Same key again: fine (idempotent).
        storage.bind_device_key("d1", "aabb", 200).unwrap();
        // Different key for the same device id: rejected.
        let err = storage.bind_device_key("d1", "ccdd", 300).unwrap_err();
        assert_eq!(err, AuthError::DeviceKeyMismatch);
    }

    #[test]
    fn key_package_lookup_consumes_what_it_returns() {
        let storage = Storage::open(":memory:").unwrap();
        storage.insert_key_package("d1", "\"kp1\"", 100).unwrap();
        storage.insert_key_package("d1", "\"kp2\"", 101).unwrap();

        let first = storage
            .take_key_packages("d1", 200, KEY_PACKAGE_MAX_AGE_SECS)
            .unwrap();
        assert_eq!(first, vec!["\"kp1\"".to_string(), "\"kp2\"".to_string()]);

        let second = storage
            .take_key_packages("d1", 200, KEY_PACKAGE_MAX_AGE_SECS)
            .unwrap();
        assert!(
            second.is_empty(),
            "a second lookup should find nothing left to consume"
        );
    }

    #[test]
    fn expired_key_packages_are_excluded_and_purged() {
        let storage = Storage::open(":memory:").unwrap();
        storage.insert_key_package("d1", "\"old\"", 0).unwrap();

        let now = KEY_PACKAGE_MAX_AGE_SECS + 1;
        let found = storage
            .take_key_packages("d1", now, KEY_PACKAGE_MAX_AGE_SECS)
            .unwrap();
        assert!(
            found.is_empty(),
            "expired key package should not be returned"
        );
    }

    #[test]
    fn endpoint_addr_replaces_and_expires_by_ttl() {
        let storage = Storage::open(":memory:").unwrap();
        storage
            .upsert_endpoint_addr("d1", "\"addr-a\"", 100)
            .unwrap();
        assert_eq!(
            storage
                .get_endpoint_addr("d1", 100, ENDPOINT_ADDR_TTL_SECS)
                .unwrap(),
            Some("\"addr-a\"".to_string())
        );

        // Republishing replaces rather than accumulating.
        storage
            .upsert_endpoint_addr("d1", "\"addr-b\"", 150)
            .unwrap();
        assert_eq!(
            storage
                .get_endpoint_addr("d1", 150, ENDPOINT_ADDR_TTL_SECS)
                .unwrap(),
            Some("\"addr-b\"".to_string())
        );

        // Past the TTL, it's treated as absent.
        let stale_now = 150 + ENDPOINT_ADDR_TTL_SECS + 1;
        assert_eq!(
            storage
                .get_endpoint_addr("d1", stale_now, ENDPOINT_ADDR_TTL_SECS)
                .unwrap(),
            None
        );
    }

    #[test]
    fn looking_up_a_device_nobody_published_for_is_empty_not_an_error() {
        let storage = Storage::open(":memory:").unwrap();
        assert!(storage
            .take_key_packages("nobody", 0, KEY_PACKAGE_MAX_AGE_SECS)
            .unwrap()
            .is_empty());
        assert_eq!(
            storage
                .get_endpoint_addr("nobody", 0, ENDPOINT_ADDR_TTL_SECS)
                .unwrap(),
            None
        );
    }
}
