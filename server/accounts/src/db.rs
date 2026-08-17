//! SQLite-backed storage for the accounts server, per
//! `docs/adr/0009-account-system.md`'s "Hosting" section.
//!
//! Unencrypted, same reasoning as `server/directory/src/db.rs`: nothing
//! stored here is confidential. Device registrations are signed *public*
//! key material (an account root key publicly vouching "this device
//! belongs to me," meant to be readable by anyone verifying that device —
//! see `ankai_core::account::DeviceRegistration`'s doc comment). Recovery
//! blobs are already client-side ciphertext before they ever reach this
//! server, per `docs/adr/0004-e2ee-stack.md` — this server only ever holds
//! opaque bytes it cannot decrypt, so encrypting the column they sit in
//! would be theater, not defense, exactly like `server/directory`'s
//! reasoning for its own SQLite file.
//!
//! `Storage`'s methods are synchronous (`rusqlite` has no async story);
//! callers (see `crate::app`) run them inside `tokio::task::spawn_blocking`,
//! matching `server/directory`'s established pattern.

use std::sync::Mutex;

use rusqlite::Connection;

/// Recovery blobs are opaque client-side ciphertext of unknown-to-this-
/// server internal structure (per `docs/adr/0004-e2ee-stack.md`), so this
/// server can only bound their size, not validate their contents. 1 MiB is
/// a generous reference-implementation ceiling for "an MLS state snapshot
/// plus an account root key," not a value derived from measuring a real
/// backup — revisit once a real backup format exists.
pub const MAX_RECOVERY_BLOB_BYTES: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("storage error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("storage lock poisoned")]
    LockPoisoned,
}

/// A device registration row as stored — the same fields as
/// `ankai_core::account::DeviceRegistration`, hex-encoded for SQLite
/// storage, plus `revoked_at_unix`. Kept as a distinct type from the core
/// crate's `DeviceRegistration` (which has no notion of revocation — that's
/// a server-side bookkeeping concept, not part of what the account root key
/// actually signs) rather than smuggling an extra optional field onto it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeviceRegistrationRow {
    pub device_id: String,
    pub device_public_key_hex: String,
    pub signed_at_unix: i64,
    pub signature_hex: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RegisterDeviceError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("device id is already registered under a different account or key")]
    DeviceIdConflict,
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
            CREATE TABLE IF NOT EXISTS accounts (
                account_id TEXT PRIMARY KEY,
                account_public_key_hex TEXT NOT NULL,
                first_seen_unix INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS device_registrations (
                device_id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL,
                device_public_key_hex TEXT NOT NULL,
                signed_at_unix INTEGER NOT NULL,
                signature_hex TEXT NOT NULL,
                revoked_at_unix INTEGER
            );
            CREATE INDEX IF NOT EXISTS device_registrations_account_id
                ON device_registrations(account_id);
            CREATE TABLE IF NOT EXISTS recovery_blobs (
                account_id TEXT PRIMARY KEY,
                blob BLOB NOT NULL,
                updated_at_unix INTEGER NOT NULL
            );
            ",
        )?;
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, StorageError> {
        self.conn.lock().map_err(|_| StorageError::LockPoisoned)
    }

    /// Registers (or idempotently refreshes) `device_id` as belonging to
    /// `account_id`/`account_public_key_hex`. The caller (see
    /// `crate::app::add_device`) has already verified the
    /// `DeviceRegistration`'s own embedded signature and self-certification
    /// before calling this — this method's own job is purely storage
    /// bookkeeping and the one thing signature verification alone can't
    /// catch: whether `device_id` is already claimed by a *different*
    /// account or key.
    #[allow(clippy::too_many_arguments)]
    pub fn register_device(
        &self,
        account_id: &str,
        account_public_key_hex: &str,
        device_id: &str,
        device_public_key_hex: &str,
        signed_at_unix: i64,
        signature_hex: &str,
        now: i64,
    ) -> Result<(), RegisterDeviceError> {
        let conn = self.lock()?;

        // Check the device-id conflict *before* touching the `accounts`
        // table at all — a rejected impersonation attempt must not leave
        // behind a side effect (e.g. implicitly creating the attacker's own
        // empty account row) just because it got as far as being checked.
        let existing_device: Option<(String, String)> = conn
            .query_row(
                "SELECT account_id, device_public_key_hex FROM device_registrations WHERE device_id = ?1",
                [device_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        let is_idempotent_refresh = match &existing_device {
            Some((existing_account, existing_key)) => {
                if existing_account == account_id && existing_key == device_public_key_hex {
                    true
                } else {
                    // Same device id, but a different account or a
                    // different device key claiming it — the exact
                    // "impersonation attempt" this method exists to catch.
                    return Err(RegisterDeviceError::DeviceIdConflict);
                }
            }
            None => false,
        };

        let existing_account_key: Option<String> = conn
            .query_row(
                "SELECT account_public_key_hex FROM accounts WHERE account_id = ?1",
                [account_id],
                |row| row.get(0),
            )
            .ok();
        match existing_account_key {
            // Self-certification already guarantees this always matches in
            // correct operation (the id *is* a hash of the key) — this is
            // a defense-in-depth check, not the primary trust boundary.
            Some(existing) if existing != account_public_key_hex => {
                return Err(RegisterDeviceError::DeviceIdConflict);
            }
            Some(_) => {}
            None => {
                conn.execute(
                    "INSERT INTO accounts (account_id, account_public_key_hex, first_seen_unix) VALUES (?1, ?2, ?3)",
                    rusqlite::params![account_id, account_public_key_hex, now],
                )
                .map_err(StorageError::from)?;
            }
        }

        if is_idempotent_refresh {
            conn.execute(
                "UPDATE device_registrations SET signed_at_unix = ?2, signature_hex = ?3, revoked_at_unix = NULL WHERE device_id = ?1",
                rusqlite::params![device_id, signed_at_unix, signature_hex],
            )
            .map_err(StorageError::from)?;
        } else {
            conn.execute(
                "INSERT INTO device_registrations
                    (device_id, account_id, device_public_key_hex, signed_at_unix, signature_hex, revoked_at_unix)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
                rusqlite::params![
                    device_id,
                    account_id,
                    device_public_key_hex,
                    signed_at_unix,
                    signature_hex
                ],
            )
            .map_err(StorageError::from)?;
        }

        Ok(())
    }

    /// The account's currently-known public key (if this account id has
    /// ever had a device registered), and every currently non-revoked
    /// device registration for it, oldest first.
    pub fn list_devices(
        &self,
        account_id: &str,
    ) -> Result<Option<(String, Vec<DeviceRegistrationRow>)>, StorageError> {
        let conn = self.lock()?;
        let account_key: Option<String> = conn
            .query_row(
                "SELECT account_public_key_hex FROM accounts WHERE account_id = ?1",
                [account_id],
                |row| row.get(0),
            )
            .ok();
        let Some(account_key) = account_key else {
            return Ok(None);
        };

        let mut stmt = conn.prepare(
            "SELECT device_id, device_public_key_hex, signed_at_unix, signature_hex
             FROM device_registrations
             WHERE account_id = ?1 AND revoked_at_unix IS NULL
             ORDER BY signed_at_unix ASC",
        )?;
        let rows: Vec<DeviceRegistrationRow> = stmt
            .query_map([account_id], |row| {
                Ok(DeviceRegistrationRow {
                    device_id: row.get(0)?,
                    device_public_key_hex: row.get(1)?,
                    signed_at_unix: row.get(2)?,
                    signature_hex: row.get(3)?,
                })
            })?
            .collect::<Result<_, _>>()?;

        Ok(Some((account_key, rows)))
    }

    /// Marks `device_id` (which must belong to `account_id`) revoked as of
    /// `now`. `Ok(false)` if no such non-revoked registration exists for
    /// this account (the caller maps that to a 404) — never silently
    /// revokes a device belonging to a different account.
    pub fn revoke_device(
        &self,
        account_id: &str,
        device_id: &str,
        now: i64,
    ) -> Result<bool, StorageError> {
        let conn = self.lock()?;
        let updated = conn.execute(
            "UPDATE device_registrations SET revoked_at_unix = ?3
             WHERE device_id = ?1 AND account_id = ?2 AND revoked_at_unix IS NULL",
            rusqlite::params![device_id, account_id, now],
        )?;
        Ok(updated > 0)
    }

    /// Stores (replacing any previous) opaque recovery-blob bytes for
    /// `account_id`. Callers must enforce [`MAX_RECOVERY_BLOB_BYTES`]
    /// before calling this (see `crate::app`) — this method itself does not
    /// re-check size, since axum's extractor already bounds the body it
    /// receives.
    pub fn put_recovery_blob(
        &self,
        account_id: &str,
        blob: &[u8],
        now: i64,
    ) -> Result<(), StorageError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO recovery_blobs (account_id, blob, updated_at_unix) VALUES (?1, ?2, ?3)
             ON CONFLICT(account_id) DO UPDATE SET blob = excluded.blob, updated_at_unix = excluded.updated_at_unix",
            rusqlite::params![account_id, blob, now],
        )?;
        Ok(())
    }

    /// The current recovery-blob bytes for `account_id`, or `None` if
    /// nothing has ever been stored.
    pub fn get_recovery_blob(&self, account_id: &str) -> Result<Option<Vec<u8>>, StorageError> {
        let conn = self.lock()?;
        let blob: Option<Vec<u8>> = conn
            .query_row(
                "SELECT blob FROM recovery_blobs WHERE account_id = ?1",
                [account_id],
                |row| row.get(0),
            )
            .ok();
        Ok(blob)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registering_a_device_and_listing_it_round_trips() {
        let storage = Storage::open(":memory:").unwrap();
        storage
            .register_device("acct1", "acctkey", "dev1", "devkey", 100, "sig", 100)
            .unwrap();

        let (account_key, devices) = storage.list_devices("acct1").unwrap().unwrap();
        assert_eq!(account_key, "acctkey");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].device_id, "dev1");
    }

    #[test]
    fn listing_an_unknown_account_is_none_not_an_error() {
        let storage = Storage::open(":memory:").unwrap();
        assert_eq!(storage.list_devices("nobody").unwrap(), None);
    }

    #[test]
    fn re_registering_the_same_device_and_key_is_idempotent() {
        let storage = Storage::open(":memory:").unwrap();
        storage
            .register_device("acct1", "acctkey", "dev1", "devkey", 100, "sig-a", 100)
            .unwrap();
        storage
            .register_device("acct1", "acctkey", "dev1", "devkey", 200, "sig-b", 200)
            .unwrap();

        let (_, devices) = storage.list_devices("acct1").unwrap().unwrap();
        assert_eq!(devices.len(), 1, "refreshing must not duplicate the row");
        assert_eq!(devices[0].signed_at_unix, 200);
    }

    #[test]
    fn a_different_account_cannot_claim_an_already_registered_device_id() {
        let storage = Storage::open(":memory:").unwrap();
        storage
            .register_device("acct1", "acctkey1", "dev1", "devkey1", 100, "sig", 100)
            .unwrap();

        let result =
            storage.register_device("acct2", "acctkey2", "dev1", "devkey2", 200, "sig2", 200);
        assert!(matches!(result, Err(RegisterDeviceError::DeviceIdConflict)));

        // The original registration is untouched.
        let (account_key, devices) = storage.list_devices("acct1").unwrap().unwrap();
        assert_eq!(account_key, "acctkey1");
        assert_eq!(devices[0].device_public_key_hex, "devkey1");
    }

    #[test]
    fn a_different_device_key_under_the_same_device_id_and_account_is_also_a_conflict() {
        let storage = Storage::open(":memory:").unwrap();
        storage
            .register_device("acct1", "acctkey1", "dev1", "devkey1", 100, "sig", 100)
            .unwrap();

        let result = storage.register_device(
            "acct1",
            "acctkey1",
            "dev1",
            "devkey-DIFFERENT",
            200,
            "sig2",
            200,
        );
        assert!(matches!(result, Err(RegisterDeviceError::DeviceIdConflict)));
    }

    #[test]
    fn revoking_removes_a_device_from_the_active_list() {
        let storage = Storage::open(":memory:").unwrap();
        storage
            .register_device("acct1", "acctkey", "dev1", "devkey", 100, "sig", 100)
            .unwrap();

        let revoked = storage.revoke_device("acct1", "dev1", 200).unwrap();
        assert!(revoked);

        let (_, devices) = storage.list_devices("acct1").unwrap().unwrap();
        assert!(devices.is_empty());
    }

    #[test]
    fn revoking_a_device_under_the_wrong_account_fails() {
        let storage = Storage::open(":memory:").unwrap();
        storage
            .register_device("acct1", "acctkey", "dev1", "devkey", 100, "sig", 100)
            .unwrap();

        let revoked = storage.revoke_device("acct2", "dev1", 200).unwrap();
        assert!(
            !revoked,
            "revoking dev1 under a different account must not succeed"
        );

        // Still listed as active under its real account.
        let (_, devices) = storage.list_devices("acct1").unwrap().unwrap();
        assert_eq!(devices.len(), 1);
    }

    #[test]
    fn revoking_an_unknown_device_reports_not_found() {
        let storage = Storage::open(":memory:").unwrap();
        assert!(!storage.revoke_device("acct1", "nobody", 100).unwrap());
    }

    #[test]
    fn recovery_blob_round_trips_and_replaces() {
        let storage = Storage::open(":memory:").unwrap();
        assert_eq!(storage.get_recovery_blob("acct1").unwrap(), None);

        storage.put_recovery_blob("acct1", b"blob-a", 100).unwrap();
        assert_eq!(
            storage.get_recovery_blob("acct1").unwrap(),
            Some(b"blob-a".to_vec())
        );

        storage.put_recovery_blob("acct1", b"blob-b", 200).unwrap();
        assert_eq!(
            storage.get_recovery_blob("acct1").unwrap(),
            Some(b"blob-b".to_vec())
        );
    }
}
