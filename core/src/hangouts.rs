//! Local Hangout (watch-party) session records.
//!
//! **Phase 1 scope.** The product concept ("Hangouts") from the original
//! spec is a real-time watch party: multiple peers in a session, synced
//! playback state, actual video played back through libmpv per
//! ADR-0006. None of that exists yet: ADR-0006 is Accepted but unimplemented
//! anywhere in this codebase, and there is no P2P gossip/pub-sub layer
//! (`iroh-gossip`) built on top of `core::p2p` — only its raw point-to-point
//! transport. Building a "watch party" UI that pretends to sync playback
//! without any of that behind it would be exactly the kind of fake data this
//! project deliberately avoids (see `communities.rs` and `messaging.rs`'s
//! doc comments for the same discipline applied to their own features).
//!
//! What this module *does* provide, honestly: a Hangout is a named session
//! this device knows about locally, created (hosted) by the user — the same
//! shape as `crate::communities::Community`. "Owner/host = the local device"
//! is not a separate stored field: exactly like communities, there is no
//! membership or multi-device model yet, so whichever device created a
//! Hangout row is definitionally its only host. Nothing here models
//! participants, invitations, playback state, or media of any kind. See
//! `PROGRESS.md` for why this scope was chosen over a fuller implementation.
//!
//! Schema lives in `db`'s migration list (see migration 5); this module owns
//! the actual queries against `Db::connection()`, same extension point
//! `communities` and `mls_provider` already lean on.

use rusqlite::OptionalExtension;

use crate::db::Db;
use crate::error::Error;
use crate::util::random_id;

/// A watch-party session this device hosts locally. See the module doc
/// comment for what's deliberately not modeled yet (participants, playback
/// state, media).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Hangout {
    pub id: String,
    pub name: String,
}

/// Creates and persists a new Hangout with a fresh random id.
pub fn create(db: &Db, name: &str) -> Result<Hangout, Error> {
    let id = random_id();
    db.connection()
        .execute(
            "INSERT INTO hangouts (id, name) VALUES (?1, ?2)",
            rusqlite::params![id, name],
        )
        .map_err(|e| Error::Db(format!("failed to create hangout {name:?}: {e}")))?;
    Ok(Hangout {
        id,
        name: name.to_string(),
    })
}

/// Lists every Hangout this device hosts, oldest first.
pub fn list(db: &Db) -> Result<Vec<Hangout>, Error> {
    let conn = db.connection();
    let mut stmt = conn
        .prepare("SELECT id, name FROM hangouts ORDER BY created_at ASC")
        .map_err(|e| Error::Db(format!("failed to prepare hangout list query: {e}")))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(Hangout {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })
        .map_err(|e| Error::Db(format!("failed to query hangouts: {e}")))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::Db(format!("failed to read hangout row: {e}")))
}

/// Fetches a single Hangout by id, or `None` if it doesn't exist.
pub fn get(db: &Db, id: &str) -> Result<Option<Hangout>, Error> {
    db.connection()
        .query_row(
            "SELECT id, name FROM hangouts WHERE id = ?1",
            rusqlite::params![id],
            |row| {
                Ok(Hangout {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(|e| Error::Db(format!("failed to read hangout {id:?}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_persists_and_list_returns_in_creation_order() {
        let db = Db::open_in_memory("correct horse battery staple").unwrap();
        assert_eq!(list(&db).unwrap(), Vec::new());

        let first = create(&db, "Friday Anime Night").unwrap();
        let second = create(&db, "Speedrun Watchalong").unwrap();

        assert_ne!(first.id, second.id, "each hangout gets a distinct id");

        let all = list(&db).unwrap();
        assert_eq!(all, vec![first.clone(), second]);

        assert_eq!(get(&db, &first.id).unwrap(), Some(first));
        assert_eq!(get(&db, "does-not-exist").unwrap(), None);
    }
}
