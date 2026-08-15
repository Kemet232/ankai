//! Local discussion posts within a community — the "forum" half of
//! "communities/forums" that `crate::communities` alone doesn't provide.
//!
//! **Phase 1 scope.** A post here is just a flat, timestamped block of text
//! attached to a community this device already knows about (see
//! `crate::communities`), created by the user on this device. Deliberately
//! **not** modeled yet, matching the same discipline `communities.rs` and
//! `hangouts.rs` apply to their own features:
//!
//! - **No replies/threading.** Posts are a single flat, chronological list
//!   per community — no parent/child relationships.
//! - **No editing or deleting.** Posts are append-only once created.
//! - **No author attribution beyond "this device."** There is no
//!   multi-device/multi-user model yet (same as `communities`/`hangouts`),
//!   so a post doesn't record "who" wrote it beyond the fact that it was
//!   written locally — there's only one writer possible right now.
//! - **No moderation.** No reporting, hiding, or removal tooling.
//! - **No real sync.** A community's posts only exist in this device's own
//!   local encrypted DB until real P2P/community sync is built (separate,
//!   unstarted future work — same caveat `communities.rs` already carries
//!   for the community itself).
//!
//! Schema lives in `db`'s migration list (see migration 7); this module owns
//! the actual queries against `Db::connection()`, the same extension point
//! `communities` and `hangouts` already lean on. Kept as its own module
//! rather than folded into `communities.rs` to keep "what a community is"
//! (identity/naming) separate from "what's inside a community"
//! (discussion content) — the same separation the product spec draws
//! between "communities" and "forums."

use rusqlite::OptionalExtension;

use crate::db::Db;
use crate::error::Error;
use crate::util::random_id;

/// A single discussion post inside a community. See the module doc comment
/// for what's deliberately not modeled yet (replies, editing, authorship,
/// moderation).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Post {
    pub id: String,
    pub community_id: String,
    pub content: String,
}

/// Creates and persists a new post with a fresh random id, attached to
/// `community_id`. Does not verify `community_id` refers to an existing
/// community — same "trust the caller" contract `communities::get` already
/// has no enforcement mechanism for, since there's no foreign-key
/// enforcement in this schema (see `db.rs`'s other tables for precedent).
pub fn create_post(db: &Db, community_id: &str, content: &str) -> Result<Post, Error> {
    let id = random_id();
    db.connection()
        .execute(
            "INSERT INTO posts (id, community_id, content) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, community_id, content],
        )
        .map_err(|e| Error::Db(format!("failed to create post in {community_id:?}: {e}")))?;
    Ok(Post {
        id,
        community_id: community_id.to_string(),
        content: content.to_string(),
    })
}

/// Lists every post attached to `community_id`, oldest first (chronological,
/// like a real discussion thread). Posts belonging to other communities are
/// never included.
pub fn list_posts(db: &Db, community_id: &str) -> Result<Vec<Post>, Error> {
    let conn = db.connection();
    let mut stmt = conn
        .prepare(
            "SELECT id, community_id, content FROM posts
             WHERE community_id = ?1
             ORDER BY created_at ASC",
        )
        .map_err(|e| Error::Db(format!("failed to prepare post list query: {e}")))?;

    let rows = stmt
        .query_map(rusqlite::params![community_id], |row| {
            Ok(Post {
                id: row.get(0)?,
                community_id: row.get(1)?,
                content: row.get(2)?,
            })
        })
        .map_err(|e| Error::Db(format!("failed to query posts: {e}")))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::Db(format!("failed to read post row: {e}")))
}

/// Fetches a single post by id, or `None` if it doesn't exist.
pub fn get_post(db: &Db, id: &str) -> Result<Option<Post>, Error> {
    db.connection()
        .query_row(
            "SELECT id, community_id, content FROM posts WHERE id = ?1",
            rusqlite::params![id],
            |row| {
                Ok(Post {
                    id: row.get(0)?,
                    community_id: row.get(1)?,
                    content: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(|e| Error::Db(format!("failed to read post {id:?}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::communities;

    #[test]
    fn create_persists_and_list_returns_in_chronological_order() {
        let db = Db::open_in_memory("correct horse battery staple").unwrap();
        let community = communities::create(&db, "Zelda Fans").unwrap();

        assert_eq!(list_posts(&db, &community.id).unwrap(), Vec::new());

        let first = create_post(&db, &community.id, "Anyone hyped for the new trailer?").unwrap();
        let second = create_post(&db, &community.id, "Yes! Day one.").unwrap();
        let third = create_post(&db, &community.id, "Same, taking the day off.").unwrap();

        assert_ne!(first.id, second.id, "each post gets a distinct id");
        assert_ne!(second.id, third.id);

        let all = list_posts(&db, &community.id).unwrap();
        assert_eq!(all, vec![first.clone(), second, third]);

        assert_eq!(get_post(&db, &first.id).unwrap(), Some(first));
        assert_eq!(get_post(&db, "does-not-exist").unwrap(), None);
    }

    #[test]
    fn posts_do_not_leak_across_communities() {
        let db = Db::open_in_memory("correct horse battery staple").unwrap();
        let zelda = communities::create(&db, "Zelda Fans").unwrap();
        let retro = communities::create(&db, "Retro Netplay").unwrap();

        let zelda_post = create_post(&db, &zelda.id, "Best dungeon in the series?").unwrap();
        let retro_post_1 = create_post(&db, &retro.id, "Anyone up for a match tonight?").unwrap();
        let retro_post_2 = create_post(&db, &retro.id, "I'm in, what time?").unwrap();

        assert_eq!(list_posts(&db, &zelda.id).unwrap(), vec![zelda_post]);
        assert_eq!(
            list_posts(&db, &retro.id).unwrap(),
            vec![retro_post_1, retro_post_2]
        );
    }
}
