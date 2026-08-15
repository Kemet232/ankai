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
//!
//! Also holds the `usernames` table (device-scoped short names — see
//! `crate::username`'s module doc comment for the full scope/validation
//! story); `claim_username`/`lookup_username` are this table's read/write
//! surface, mirroring `bind_device_key`'s first-come-first-served shape.

use std::sync::Mutex;

use rusqlite::Connection;

use crate::auth::AuthError;
use crate::username::{self, UsernameError};

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
            CREATE TABLE IF NOT EXISTS usernames (
                username TEXT PRIMARY KEY,
                device_id TEXT NOT NULL,
                claimed_at_unix INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS usernames_device_id ON usernames(device_id);
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

    /// Claims `username_val` for `device_id`, per `crate::username`'s
    /// module doc comment: validates the format first (real rejection, not
    /// silent normalization); if the username is unclaimed, binds it to
    /// `device_id`; if it's already claimed by `device_id` itself, this is
    /// an idempotent "touch" (updates the claimed-at timestamp, same
    /// semantics as `bind_device_key`'s "same key again: fine"); if it's
    /// claimed by a *different* device, rejects with
    /// [`UsernameError::AlreadyClaimed`] — first claim wins, matching the
    /// TOFU spirit of `bind_device_key`.
    ///
    /// A device can only hold one username at a time: claiming a new one
    /// deletes any username this same `device_id` previously held, freeing
    /// it for someone else to claim. This is what makes "the same device
    /// can update its own claimed username" (per the module doc comment)
    /// actually change the username rather than just adding a second one.
    pub fn claim_username(
        &self,
        device_id: &str,
        username_val: &str,
        now: i64,
    ) -> Result<(), ClaimUsernameError> {
        username::validate(username_val)?;

        let conn = self.lock()?;
        let existing_owner: Option<String> = conn
            .query_row(
                "SELECT device_id FROM usernames WHERE username = ?1",
                [username_val],
                |row| row.get(0),
            )
            .ok();

        match existing_owner {
            Some(owner) if owner == device_id => {
                conn.execute(
                    "UPDATE usernames SET claimed_at_unix = ?2 WHERE username = ?1",
                    rusqlite::params![username_val, now],
                )
                .map_err(StorageError::from)?;
                Ok(())
            }
            Some(_) => Err(UsernameError::AlreadyClaimed.into()),
            None => {
                conn.execute("DELETE FROM usernames WHERE device_id = ?1", [device_id])
                    .map_err(StorageError::from)?;
                conn.execute(
                    "INSERT INTO usernames (username, device_id, claimed_at_unix) VALUES (?1, ?2, ?3)",
                    rusqlite::params![username_val, device_id, now],
                )
                .map_err(StorageError::from)?;
                Ok(())
            }
        }
    }

    /// The `DeviceId` currently claiming `username_val`, or `None` if
    /// nobody has (either it was never claimed, or the format wouldn't
    /// even validate — either way, "not found," not an error, matching
    /// `get_endpoint_addr`'s convention).
    pub fn lookup_username(&self, username_val: &str) -> Result<Option<String>, StorageError> {
        let conn = self.lock()?;
        let device_id: Option<String> = conn
            .query_row(
                "SELECT device_id FROM usernames WHERE username = ?1",
                [username_val],
                |row| row.get(0),
            )
            .ok();
        Ok(device_id)
    }
}

/// Errors from [`Storage::claim_username`]: either the username failed
/// [`crate::username::validate`], it's already claimed by a different
/// device, or a real storage error occurred. Kept as a distinct type from
/// [`StorageError`] (which has no opinion on username semantics) and
/// [`UsernameError`] (which has no opinion on SQLite) rather than folding
/// either into the other.
#[derive(Debug, thiserror::Error)]
pub enum ClaimUsernameError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Username(#[from] UsernameError),
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
    fn username_claim_round_trips_and_is_first_come_first_served() {
        let storage = Storage::open(":memory:").unwrap();
        storage.claim_username("d1", "alice_1", 100).unwrap();
        assert_eq!(
            storage.lookup_username("alice_1").unwrap(),
            Some("d1".to_string())
        );

        // A different device can't steal an already-claimed username.
        let err = storage.claim_username("d2", "alice_1", 200).unwrap_err();
        assert!(matches!(
            err,
            ClaimUsernameError::Username(UsernameError::AlreadyClaimed)
        ));
        // Still owned by d1, unaffected by the failed steal attempt.
        assert_eq!(
            storage.lookup_username("alice_1").unwrap(),
            Some("d1".to_string())
        );

        // The same device can re-claim (idempotent touch).
        storage.claim_username("d1", "alice_1", 300).unwrap();
        assert_eq!(
            storage.lookup_username("alice_1").unwrap(),
            Some("d1".to_string())
        );
    }

    #[test]
    fn same_device_can_change_its_username_freeing_the_old_one() {
        let storage = Storage::open(":memory:").unwrap();
        storage.claim_username("d1", "old_name", 100).unwrap();
        storage.claim_username("d1", "new_name", 200).unwrap();

        assert_eq!(storage.lookup_username("old_name").unwrap(), None);
        assert_eq!(
            storage.lookup_username("new_name").unwrap(),
            Some("d1".to_string())
        );

        // The freed name is now claimable by someone else.
        storage.claim_username("d2", "old_name", 300).unwrap();
        assert_eq!(
            storage.lookup_username("old_name").unwrap(),
            Some("d2".to_string())
        );
    }

    #[test]
    fn invalid_usernames_are_rejected_not_silently_accepted() {
        let storage = Storage::open(":memory:").unwrap();

        for bad in ["Alice", "ab", &"a".repeat(21), "has space", "has-dash"] {
            let err = storage.claim_username("d1", bad, 100).unwrap_err();
            assert!(
                matches!(err, ClaimUsernameError::Username(UsernameError::Invalid(_))),
                "expected {bad:?} to be rejected as invalid"
            );
        }
        // None of the rejected attempts should have been stored.
        assert_eq!(storage.lookup_username("alice").unwrap(), None);
    }

    #[test]
    fn looking_up_a_username_nobody_claimed_is_empty_not_an_error() {
        let storage = Storage::open(":memory:").unwrap();
        assert_eq!(storage.lookup_username("nobody_here").unwrap(), None);
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
