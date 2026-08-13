//! Local-first persistence (SQLite), encrypted at rest via SQLCipher.
//!
//! See `docs/adr/0004-e2ee-stack.md`'s "Local encrypted message database"
//! section: the primary choice is SQLCipher via `rusqlite`'s
//! `bundled-sqlcipher` feature (SQLCipher's BSD-style-licensed C source is
//! vendored through a build script; the `rusqlite` Rust crate itself is
//! MIT — see `deny.toml`'s note on why this doesn't need its own
//! allow-list entry).
//!
//! This module is Phase 1 scaffolding: a minimal typed connection API plus
//! a `schema_version` bookkeeping table and a tiny migrations runner. No
//! feature tables yet — those land with actual features later. Deriving or
//! storing the passphrase itself (OS secure storage, per ADR-0004's "Key
//! management" section) is out of scope here; callers supply it.

use std::path::Path;

use rusqlite::Connection;

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
const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    description: "create schema_version table",
    sql: "CREATE TABLE IF NOT EXISTS schema_version (
        version     INTEGER NOT NULL PRIMARY KEY,
        applied_at  TEXT NOT NULL DEFAULT (datetime('now')),
        description TEXT NOT NULL
    );",
}];

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
    fn in_memory_open_applies_migration_one() {
        let db = Db::open_in_memory("correct horse battery staple")
            .expect("opening an in-memory encrypted db should succeed");
        assert_eq!(db.schema_version().unwrap(), 1);
    }

    #[test]
    fn schema_version_bookkeeping_is_idempotent_across_reopen() {
        let path = temp_db_path("idempotent");

        {
            let db = Db::open(&path, "hunter2").expect("first open should succeed");
            assert_eq!(db.schema_version().unwrap(), 1);
        } // connection dropped, file persists on disk

        {
            // Reopening an already-migrated database must not error and
            // must not re-apply (or double-record) migration 1.
            let db = Db::open(&path, "hunter2").expect("second open should succeed");
            assert_eq!(db.schema_version().unwrap(), 1);

            let row_count: i64 = db
                .connection()
                .query_row("SELECT count(*) FROM schema_version", [], |row| row.get(0))
                .unwrap();
            assert_eq!(row_count, 1, "migration 1 must be recorded exactly once");
        }

        cleanup(&path);
    }

    #[test]
    fn wrong_key_fails_to_open_existing_encrypted_db() {
        let path = temp_db_path("wrong-key");

        {
            let db = Db::open(&path, "the-real-passphrase").expect("initial open should succeed");
            assert_eq!(db.schema_version().unwrap(), 1);
        }

        let result = Db::open(&path, "not-the-real-passphrase");
        assert!(
            result.is_err(),
            "opening an encrypted db with the wrong key must fail, not silently succeed"
        );

        cleanup(&path);
    }
}
