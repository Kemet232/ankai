//! Local community records.
//!
//! **Phase 1 scope.** A community here is just a name this device knows
//! about locally, created by the user — nothing about membership, other
//! members, forums/channels, or moderation is modeled yet. There is no
//! networking or discovery involved: creating a community right now is
//! exactly as local-only as everything else in `core::db`. This exists so
//! the nav skeleton's Communities pane has one genuine, persisted piece of
//! state to create and list, instead of remaining pure placeholder text —
//! not a claim that communities are otherwise feature-complete.
//!
//! Schema lives in `db`'s migration list (see migration 4); this module
//! owns the actual queries against `Db::connection()`, the same extension
//! point `mls_provider` and (implicitly) `identity` already lean on.

use rusqlite::OptionalExtension;

use crate::db::Db;
use crate::error::Error;
use crate::util::random_id;

/// A community this device knows about. See the module doc comment for
/// what's deliberately not modeled yet (membership, channels, ...).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Community {
    pub id: String,
    pub name: String,
}

/// Creates and persists a new community with a fresh random id.
pub fn create(db: &Db, name: &str) -> Result<Community, Error> {
    let id = random_id();
    db.connection()
        .execute(
            "INSERT INTO communities (id, name) VALUES (?1, ?2)",
            rusqlite::params![id, name],
        )
        .map_err(|e| Error::Db(format!("failed to create community {name:?}: {e}")))?;
    Ok(Community {
        id,
        name: name.to_string(),
    })
}

/// Lists every community this device knows about, oldest first.
pub fn list(db: &Db) -> Result<Vec<Community>, Error> {
    let conn = db.connection();
    let mut stmt = conn
        .prepare("SELECT id, name FROM communities ORDER BY created_at ASC")
        .map_err(|e| Error::Db(format!("failed to prepare community list query: {e}")))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(Community {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })
        .map_err(|e| Error::Db(format!("failed to query communities: {e}")))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::Db(format!("failed to read community row: {e}")))
}

/// Fetches a single community by id, or `None` if it doesn't exist.
pub fn get(db: &Db, id: &str) -> Result<Option<Community>, Error> {
    db.connection()
        .query_row(
            "SELECT id, name FROM communities WHERE id = ?1",
            rusqlite::params![id],
            |row| {
                Ok(Community {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(|e| Error::Db(format!("failed to read community {id:?}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_persists_and_list_returns_in_creation_order() {
        let db = Db::open_in_memory("correct horse battery staple").unwrap();
        assert_eq!(list(&db).unwrap(), Vec::new());

        let first = create(&db, "Zelda Fans").unwrap();
        let second = create(&db, "Retro Netplay").unwrap();

        assert_ne!(first.id, second.id, "each community gets a distinct id");

        let all = list(&db).unwrap();
        assert_eq!(all, vec![first.clone(), second]);

        assert_eq!(get(&db, &first.id).unwrap(), Some(first));
        assert_eq!(get(&db, "does-not-exist").unwrap(), None);
    }
}
