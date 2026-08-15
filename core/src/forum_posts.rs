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
//! `communities` and `hangouts` already lean on.
//!
//! **Cross-community recent-activity feed.** `list_recent_across_communities`
//! (below) is a read-only aggregation view over the same `posts`/
//! `communities` tables, added for a "Hot Discussions" widget on a future
//! Home dashboard screen: the most recent posts across *every* community
//! this device knows about, each tagged with its parent community's id/name
//! so a mixed feed can still say "this one's from Zelda Fans." It is not a
//! new domain concept — no new table, no new writes, nothing a post or
//! community doesn't already have — and it carries the exact same
//! local-only discipline as the rest of this module and `communities.rs`:
//! it only ever sees this device's own local communities/posts, never
//! anything from a network peer (there is no P2P/community sync yet, see
//! both modules' doc comments). Kept as its own module
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

/// One entry in the cross-community recent-activity feed: a post's content
/// plus enough about its parent community that a caller mixing posts from
/// several communities together can still label each one correctly. See
/// the module doc comment's "Cross-community recent-activity feed" section
/// for scope.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecentPost {
    pub post_id: String,
    pub community_id: String,
    pub community_name: String,
    pub content: String,
    /// When the post was created, as SQLite's `datetime('now')` text
    /// (UTC, `YYYY-MM-DD HH:MM:SS`) — the same column `posts.created_at`
    /// already stores and orders by, just exposed here since a feed needs
    /// to show "how recent" (unlike `Post`, which has no reason to expose
    /// it today).
    pub created_at: String,
}

/// The most recent posts across *every* community this device knows about,
/// most-recent-first, each tagged with its parent community's id/name.
///
/// This is a straightforward `posts` JOIN `communities` on community id,
/// ordered by `posts.created_at DESC`, capped at `limit` — a read-only
/// aggregation view for a dashboard "recent activity" widget, not a new
/// domain concept (see the module doc comment). Deliberately no pagination/
/// cursor support: `limit` is a hard cap on a single most-recent-first
/// fetch, matching what a dashboard widget needs, not infinite scroll.
///
/// A post whose `community_id` doesn't match any row in `communities` (see
/// `create_post`'s doc comment on the unenforced "trust the caller"
/// contract) is silently excluded, since there is no community name to
/// label it with — an inner join, not a left join.
pub fn list_recent_across_communities(db: &Db, limit: usize) -> Result<Vec<RecentPost>, Error> {
    let conn = db.connection();
    let mut stmt = conn
        .prepare(
            "SELECT posts.id, posts.community_id, communities.name, posts.content, posts.created_at
             FROM posts
             JOIN communities ON communities.id = posts.community_id
             ORDER BY posts.created_at DESC
             LIMIT ?1",
        )
        .map_err(|e| Error::Db(format!("failed to prepare recent-posts query: {e}")))?;

    let rows = stmt
        .query_map(rusqlite::params![limit as i64], |row| {
            Ok(RecentPost {
                post_id: row.get(0)?,
                community_id: row.get(1)?,
                community_name: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|e| Error::Db(format!("failed to query recent posts: {e}")))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::Db(format!("failed to read recent-post row: {e}")))
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

    /// Backdates a post's `created_at` to an explicit, distinct value so
    /// ordering assertions don't depend on real wall-clock gaps between
    /// fast, in-test inserts (`created_at` is SQLite `datetime('now')`
    /// text, second-granularity) — a verifiable creation order, not a
    /// timing-dependent one.
    fn set_created_at(db: &Db, post_id: &str, created_at: &str) {
        db.connection()
            .execute(
                "UPDATE posts SET created_at = ?1 WHERE id = ?2",
                rusqlite::params![created_at, post_id],
            )
            .unwrap();
    }

    #[test]
    fn recent_posts_mix_across_communities_most_recent_first_and_respect_limit() {
        let db = Db::open_in_memory("correct horse battery staple").unwrap();
        let zelda = communities::create(&db, "Zelda Fans").unwrap();
        let retro = communities::create(&db, "Retro Netplay").unwrap();
        let indie = communities::create(&db, "Indie Devs").unwrap();

        // Nothing posted anywhere yet.
        assert_eq!(list_recent_across_communities(&db, 10).unwrap(), Vec::new());

        let p1 = create_post(&db, &zelda.id, "Best dungeon in the series?").unwrap();
        set_created_at(&db, &p1.id, "2026-08-15 10:00:00");

        let p2 = create_post(&db, &retro.id, "Anyone up for a match tonight?").unwrap();
        set_created_at(&db, &p2.id, "2026-08-15 10:05:00");

        let p3 = create_post(&db, &indie.id, "Shipped a demo, feedback welcome!").unwrap();
        set_created_at(&db, &p3.id, "2026-08-15 10:10:00");

        let p4 = create_post(&db, &zelda.id, "Replaying Wind Waker HD tonight.").unwrap();
        set_created_at(&db, &p4.id, "2026-08-15 10:15:00");

        let p5 = create_post(&db, &retro.id, "I'm in, what time?").unwrap();
        set_created_at(&db, &p5.id, "2026-08-15 10:20:00");

        // Full feed: most-recent-first, each correctly labeled with its
        // real parent community, and posts DO mix across communities
        // (unlike list_posts, which is scoped to one community).
        let all = list_recent_across_communities(&db, 10).unwrap();
        assert_eq!(
            all,
            vec![
                RecentPost {
                    post_id: p5.id.clone(),
                    community_id: retro.id.clone(),
                    community_name: "Retro Netplay".to_string(),
                    content: "I'm in, what time?".to_string(),
                    created_at: "2026-08-15 10:20:00".to_string(),
                },
                RecentPost {
                    post_id: p4.id.clone(),
                    community_id: zelda.id.clone(),
                    community_name: "Zelda Fans".to_string(),
                    content: "Replaying Wind Waker HD tonight.".to_string(),
                    created_at: "2026-08-15 10:15:00".to_string(),
                },
                RecentPost {
                    post_id: p3.id.clone(),
                    community_id: indie.id.clone(),
                    community_name: "Indie Devs".to_string(),
                    content: "Shipped a demo, feedback welcome!".to_string(),
                    created_at: "2026-08-15 10:10:00".to_string(),
                },
                RecentPost {
                    post_id: p2.id.clone(),
                    community_id: retro.id.clone(),
                    community_name: "Retro Netplay".to_string(),
                    content: "Anyone up for a match tonight?".to_string(),
                    created_at: "2026-08-15 10:05:00".to_string(),
                },
                RecentPost {
                    post_id: p1.id.clone(),
                    community_id: zelda.id.clone(),
                    community_name: "Zelda Fans".to_string(),
                    content: "Best dungeon in the series?".to_string(),
                    created_at: "2026-08-15 10:00:00".to_string(),
                },
            ]
        );

        // limit caps the result to the N most recent, still most-recent-first.
        let capped = list_recent_across_communities(&db, 2).unwrap();
        assert_eq!(
            capped.iter().map(|p| p.post_id.clone()).collect::<Vec<_>>(),
            vec![p5.id.clone(), p4.id.clone()]
        );

        // limit 0 means no results, not "unlimited".
        assert_eq!(list_recent_across_communities(&db, 0).unwrap(), Vec::new());
    }
}
