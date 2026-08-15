//! Local-first persistence (SQLite), encrypted at rest via SQLCipher.
//!
//! See `docs/adr/0004-e2ee-stack.md`'s "Local encrypted message database"
//! section: the primary choice is SQLCipher via `rusqlite`'s
//! `bundled-sqlcipher` feature (SQLCipher's BSD-style-licensed C source is
//! vendored through a build script; the `rusqlite` Rust crate itself is
//! MIT — see `deny.toml`'s note on why this doesn't need its own
//! allow-list entry).
//!
//! This module is Phase 1 scaffolding: a minimal typed connection API, a
//! `schema_version` bookkeeping table, a tiny migrations runner, and a
//! generic `settings` key-value table for simple client preferences. Real
//! per-feature tables (messages, profiles, communities, ...) still land
//! later. Deriving or storing the passphrase itself is out of scope here —
//! see `crate::keychain` for the OS-secure-storage-backed key management
//! ADR-0004's "Key management" section calls for; callers of this module
//! just supply whatever passphrase string they got from there.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use crate::error::Error;

/// A single ordered migration, applied at most once, in `version` order.
struct Migration {
    version: i64,
    description: &'static str,
    sql: &'static str,
}

/// The ordered list of all migrations that have ever shipped. Append new
/// migrations to the end with strictly increasing `version`s — never edit
/// or remove an already-shipped entry, since existing databases record
/// which versions they've applied.
///
/// Phase 1: just the bookkeeping table itself. Real feature tables land in
/// later migrations appended here.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        description: "create schema_version table",
        sql: "CREATE TABLE IF NOT EXISTS schema_version (
        version     INTEGER NOT NULL PRIMARY KEY,
        applied_at  TEXT NOT NULL DEFAULT (datetime('now')),
        description TEXT NOT NULL
    );",
    },
    Migration {
        version: 2,
        description: "create settings table",
        sql: "CREATE TABLE IF NOT EXISTS settings (
        key   TEXT NOT NULL PRIMARY KEY,
        value TEXT NOT NULL
    );",
    },
    Migration {
        version: 3,
        description: "create mls_storage table",
        // Single-row table (id is always 0): see crate::mls_provider's doc
        // comment for why OpenMLS's group/ratchet state is persisted here
        // as one opaque blob rather than one row per field.
        sql: "CREATE TABLE IF NOT EXISTS mls_storage (
        id   INTEGER NOT NULL PRIMARY KEY CHECK (id = 0),
        data BLOB NOT NULL
    );",
    },
    Migration {
        version: 4,
        description: "create communities table",
        // See crate::communities's doc comment for what a "community" is
        // (and isn't) in this Phase 1 scaffolding.
        sql: "CREATE TABLE IF NOT EXISTS communities (
        id         TEXT NOT NULL PRIMARY KEY,
        name       TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );",
    },
    Migration {
        version: 5,
        description: "create hangouts table",
        // See crate::hangouts's doc comment for what a "Hangout" is (and
        // isn't) in this Phase 1 scaffolding — local-only hosting metadata,
        // no participants/playback state/media.
        sql: "CREATE TABLE IF NOT EXISTS hangouts (
        id         TEXT NOT NULL PRIMARY KEY,
        name       TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );",
    },
    Migration {
        version: 6,
        description: "create conversations and messages tables",
        // See crate::messaging's doc comment. `conversations` maps a peer
        // (this device's stable hex-encoded EndpointId for them, see
        // `messaging::peer_id_for`) to the real OpenMLS `GroupId` of the
        // 2-member MLS group backing that 1:1 conversation — the group's
        // actual state lives in the `mls_storage` blob (migration 3), this
        // table just remembers which group id goes with which peer.
        // `messages` stores already-decrypted plaintext (the ciphertext
        // itself is never at rest anywhere): "encrypted at rest" is
        // satisfied by the whole SQLCipher-encrypted file, same as
        // `settings`/`communities` already rely on, not a second encryption
        // layer on top.
        sql: "CREATE TABLE IF NOT EXISTS conversations (
        peer_id    TEXT NOT NULL PRIMARY KEY,
        group_id   BLOB NOT NULL,
        created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE TABLE IF NOT EXISTS messages (
        id         TEXT NOT NULL PRIMARY KEY,
        peer_id    TEXT NOT NULL,
        direction  TEXT NOT NULL CHECK (direction IN ('sent', 'received')),
        content    TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );",
    },
    Migration {
        version: 7,
        description: "create posts table",
        // See crate::forum_posts's doc comment for what a "post" is (and
        // isn't) in this Phase 1 scaffolding — flat, append-only discussion
        // text attached to a community (see migration 4), no replies/
        // editing/deleting/authorship/moderation. No FOREIGN KEY to
        // `communities`: this schema doesn't enable SQLite's foreign_keys
        // pragma anywhere else either (see `communities`/`hangouts`/
        // `messaging`'s tables), so it would be unenforced decoration, not
        // real integrity.
        sql: "CREATE TABLE IF NOT EXISTS posts (
        id           TEXT NOT NULL PRIMARY KEY,
        community_id TEXT NOT NULL,
        content      TEXT NOT NULL,
        created_at   TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE INDEX IF NOT EXISTS idx_posts_community_id ON posts (community_id);",
    },
    Migration {
        version: 8,
        description: "create watchlist table",
        // See crate::anime's doc comment for the full shape of this table,
        // including why title/cover/episode-count are cached here rather
        // than always re-fetched from AniList. `anilist_id` (AniList's own
        // numeric media id) is the primary key, not a locally-generated
        // random id like `communities`/`hangouts`/`posts` use — it's
        // already a stable, globally-meaningful key, and using it directly
        // makes "is this anime already on the watchlist" a plain lookup
        // instead of a search.
        sql: "CREATE TABLE IF NOT EXISTS watchlist (
        anilist_id       INTEGER NOT NULL PRIMARY KEY,
        title_romaji     TEXT,
        title_english    TEXT,
        cover_image_url  TEXT,
        episode_count    INTEGER,
        watched_episodes INTEGER NOT NULL DEFAULT 0,
        status           TEXT NOT NULL CHECK (status IN ('watching', 'completed', 'planned', 'dropped')),
        added_at         TEXT NOT NULL DEFAULT (datetime('now')),
        updated_at       TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE INDEX IF NOT EXISTS idx_watchlist_status ON watchlist (status);",
    },
];

/// A handle to ANKAI's local encrypted SQLite database.
pub struct Db {
    conn: Connection,
}

impl Db {
    /// Opens (creating if absent) an encrypted SQLite database at `path`,
    /// keyed by `passphrase`, and applies any pending migrations.
    ///
    /// Returns `Error::Db` if the file can't be opened, `passphrase` is
    /// wrong for an existing encrypted file, or a migration fails.
    pub fn open(path: impl AsRef<Path>, passphrase: &str) -> Result<Self, Error> {
        let conn = Connection::open(path.as_ref())
            .map_err(|e| Error::Db(format!("failed to open database: {e}")))?;
        Self::from_connection(conn, passphrase)
    }

    /// Opens a private, in-memory encrypted database. Useful for tests and
    /// ephemeral scratch use; nothing persists after the handle is dropped.
    pub fn open_in_memory(passphrase: &str) -> Result<Self, Error> {
        let conn = Connection::open_in_memory()
            .map_err(|e| Error::Db(format!("failed to open in-memory database: {e}")))?;
        Self::from_connection(conn, passphrase)
    }

    fn from_connection(conn: Connection, passphrase: &str) -> Result<Self, Error> {
        // The SQLCipher key must be set before any other statement touches
        // the file — SQLCipher intercepts the first read to derive the
        // page cipher from it.
        conn.pragma_update(None, "key", passphrase)
            .map_err(|e| Error::Db(format!("failed to set encryption key: {e}")))?;

        // SQLCipher only actually validates the key once something forces
        // it to read the database header, so force that check eagerly here
        // rather than deferring a confusing failure to the caller's first
        // unrelated query. A wrong key surfaces as a "file is not a
        // database" error from SQLite on this read.
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|e| Error::Db(format!("failed to unlock database (wrong key?): {e}")))?;

        let mut db = Self { conn };
        db.run_migrations()?;
        Ok(db)
    }

    /// Applies every migration in `MIGRATIONS` with a version greater than
    /// the database's currently recorded version, each in its own
    /// transaction. Safe to call on an already-up-to-date database — it's
    /// a no-op in that case (idempotent).
    fn run_migrations(&mut self) -> Result<(), Error> {
        let current = current_schema_version(&self.conn)?;

        for migration in MIGRATIONS {
            if migration.version <= current {
                continue;
            }

            let tx = self
                .conn
                .transaction()
                .map_err(|e| Error::Db(format!("failed to start migration transaction: {e}")))?;

            tx.execute_batch(migration.sql)
                .map_err(|e| Error::Db(format!("migration {} failed: {e}", migration.version)))?;

            tx.execute(
                "INSERT INTO schema_version (version, description) VALUES (?1, ?2)",
                rusqlite::params![migration.version, migration.description],
            )
            .map_err(|e| {
                Error::Db(format!(
                    "failed to record migration {}: {e}",
                    migration.version
                ))
            })?;

            tx.commit().map_err(|e| {
                Error::Db(format!(
                    "failed to commit migration {}: {e}",
                    migration.version
                ))
            })?;
        }

        Ok(())
    }

    /// The highest applied migration version, per the `schema_version`
    /// bookkeeping table. `0` means no migrations have ever been applied
    /// (not possible after a successful `open`/`open_in_memory`, since
    /// migration 1 always runs, but useful for tests/diagnostics).
    pub fn schema_version(&self) -> Result<i64, Error> {
        current_schema_version(&self.conn)
    }

    /// Reads a single value from the `settings` key-value table, or `None`
    /// if `key` has never been set.
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, Error> {
        self.conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                rusqlite::params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| Error::Db(format!("failed to read setting {key:?}: {e}")))
    }

    /// Sets a single value in the `settings` key-value table, overwriting
    /// any existing value for `key`.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), Error> {
        self.conn
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![key, value],
            )
            .map_err(|e| Error::Db(format!("failed to write setting {key:?}: {e}")))?;
        Ok(())
    }

    /// Reads OpenMLS's persisted storage blob (see `crate::mls_provider`),
    /// or `None` if nothing has been persisted yet.
    pub fn get_mls_storage_blob(&self) -> Result<Option<Vec<u8>>, Error> {
        self.conn
            .query_row("SELECT data FROM mls_storage WHERE id = 0", [], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|e| Error::Db(format!("failed to read MLS storage blob: {e}")))
    }

    /// Overwrites OpenMLS's persisted storage blob.
    pub fn set_mls_storage_blob(&self, data: &[u8]) -> Result<(), Error> {
        self.conn
            .execute(
                "INSERT INTO mls_storage (id, data) VALUES (0, ?1)
                 ON CONFLICT(id) DO UPDATE SET data = excluded.data",
                rusqlite::params![data],
            )
            .map_err(|e| Error::Db(format!("failed to write MLS storage blob: {e}")))?;
        Ok(())
    }

    /// Direct access to the underlying connection, for callers/modules
    /// (e.g. a future OpenMLS storage-trait backend, per ADR-0004) that
    /// need to run their own statements against this same encrypted file.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

/// Reads the current schema version, treating a missing `schema_version`
/// table (a brand-new database, before migration 1 has run) as version 0.
fn current_schema_version(conn: &Connection) -> Result<i64, Error> {
    let table_exists: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| Error::Db(format!("failed to check schema_version table: {e}")))?;

    if table_exists == 0 {
        return Ok(0);
    }

    conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |row| row.get(0),
    )
    .map_err(|e| Error::Db(format!("failed to read schema_version: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A unique path under the system temp dir, not created until a
    /// connection is opened against it.
    fn temp_db_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before UNIX epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "ankai-core-test-{label}-{}-{nanos}.sqlite",
            std::process::id()
        ))
    }

    /// Best-effort cleanup of a SQLite file and its WAL/journal siblings.
    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let _ = fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[test]
    fn in_memory_open_applies_all_migrations() {
        let db = Db::open_in_memory("correct horse battery staple")
            .expect("opening an in-memory encrypted db should succeed");
        assert_eq!(db.schema_version().unwrap(), 8);
    }

    #[test]
    fn schema_version_bookkeeping_is_idempotent_across_reopen() {
        let path = temp_db_path("idempotent");

        {
            let db = Db::open(&path, "hunter2").expect("first open should succeed");
            assert_eq!(db.schema_version().unwrap(), 8);
        } // connection dropped, file persists on disk

        {
            // Reopening an already-migrated database must not error and
            // must not re-apply (or double-record) any migration.
            let db = Db::open(&path, "hunter2").expect("second open should succeed");
            assert_eq!(db.schema_version().unwrap(), 8);

            let row_count: i64 = db
                .connection()
                .query_row("SELECT count(*) FROM schema_version", [], |row| row.get(0))
                .unwrap();
            assert_eq!(row_count, 8, "each migration must be recorded exactly once");
        }

        cleanup(&path);
    }

    #[test]
    fn setting_round_trips_and_missing_key_is_none() {
        let db = Db::open_in_memory("correct horse battery staple").unwrap();

        assert_eq!(db.get_setting("display_name").unwrap(), None);

        db.set_setting("display_name", "Ahmed").unwrap();
        assert_eq!(
            db.get_setting("display_name").unwrap(),
            Some("Ahmed".to_string())
        );

        // Setting an existing key again overwrites, not duplicates.
        db.set_setting("display_name", "Ahmed El Mahdi").unwrap();
        assert_eq!(
            db.get_setting("display_name").unwrap(),
            Some("Ahmed El Mahdi".to_string())
        );

        let row_count: i64 = db
            .connection()
            .query_row("SELECT count(*) FROM settings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(row_count, 1);
    }

    #[test]
    fn wrong_key_fails_to_open_existing_encrypted_db() {
        let path = temp_db_path("wrong-key");

        {
            let db = Db::open(&path, "the-real-passphrase").expect("initial open should succeed");
            assert_eq!(db.schema_version().unwrap(), 8);
        }

        let result = Db::open(&path, "not-the-real-passphrase");
        assert!(
            result.is_err(),
            "opening an encrypted db with the wrong key must fail, not silently succeed"
        );

        cleanup(&path);
    }
}
