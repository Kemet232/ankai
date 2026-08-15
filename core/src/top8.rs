//! Local "Top 8" featured communities for this device's profile.
//!
//! **Scope — read this first.** The original MySpace "Top 8" idea was about
//! featuring your top 8 *friends*. ANKAI has no real contacts/friends/
//! relationship model built yet — there is no directory of "people I know,"
//! only one-off pasted peer invites in `core::messaging`. Faking a friends
//! list here just to build "Top 8 friends" would be exactly the kind of
//! made-up data this project deliberately avoids (see `communities.rs` and
//! `hangouts.rs`'s doc comments for the same discipline applied to their own
//! features). What ANKAI *does* have real, local, persisted data for right
//! now is `core::communities`. So this module builds "Top 8 communities":
//! the user picks up to 8 of their own communities, in an order they choose,
//! to feature on their Profile pane. **This should be revisited once a real
//! contact/friend concept exists** — most likely by adding a parallel "Top 8
//! people" feature alongside this one (the two aren't mutually exclusive),
//! not by treating this module as a stand-in for that larger feature.
//!
//! **Storage.** No new table/migration. This reuses the existing `settings`
//! key-value table (`db::Db::get_setting`/`set_setting`, migration 2 — the
//! same table `display_name` already lives in). A dedicated `top8` table
//! would only ever hold a single row per device (there's one profile per
//! local device, same "one value" shape as `display_name`), just with an
//! ordered-list value instead of a plain string — that's exactly the
//! key/value shape `settings` already provides, so a new table would be
//! duplicate machinery for no real benefit. The ordered list of up to 8
//! community ids is stored under the key [`TOP8_SETTING_KEY`] as a JSON
//! array of strings (`serde_json`, already a workspace dependency). This
//! also sidesteps claiming a new `MIGRATIONS` version number — see
//! `db.rs`'s migration list and `PROGRESS.md`'s session 10 notes on why
//! migration-number collisions across concurrent branches are a real,
//! expected (if cheap to fix) cost in this repo right now.
//!
//! **What this is not**: no networking, no sharing this list with anyone
//! else, and no stronger notion of "these are my communities" than
//! `communities.rs` already has (whichever device created a community is
//! definitionally its only "owner" — there's no membership model yet). If a
//! listed community id no longer resolves via `communities::get` (there's no
//! delete yet, so this can't currently happen, but nothing here assumes it
//! never will), [`get_top8`] silently drops it rather than erroring, instead
//! of surfacing a broken/dangling entry in the UI.

use crate::communities::{self, Community};
use crate::db::Db;
use crate::error::Error;

/// The `settings` table key this module's ordered community-id list is
/// stored under (see the module doc comment for why `settings` and not a
/// dedicated table).
const TOP8_SETTING_KEY: &str = "top8_community_ids";

/// The maximum number of communities a Top 8 can hold. Named, not just
/// inlined as `8`, so every enforcement site and test refers to the same
/// constant.
pub const MAX_TOP8: usize = 8;

/// Reads the raw ordered list of community ids from `settings`, or an empty
/// list if nothing has been set yet. Internal: callers should go through
/// [`get_top8`] (resolved to real `Community` records) or the mutation
/// functions below, which all read/write through this same helper so they
/// stay consistent with each other.
fn read_ids(db: &Db) -> Result<Vec<String>, Error> {
    match db.get_setting(TOP8_SETTING_KEY)? {
        None => Ok(Vec::new()),
        Some(raw) => serde_json::from_str(&raw)
            .map_err(|e| Error::Db(format!("corrupt top8_community_ids setting: {e}"))),
    }
}

/// Writes the raw ordered list of community ids to `settings`, replacing
/// whatever was there before.
fn write_ids(db: &Db, ids: &[String]) -> Result<(), Error> {
    let raw = serde_json::to_string(ids)
        .map_err(|e| Error::Db(format!("failed to encode top8_community_ids setting: {e}")))?;
    db.set_setting(TOP8_SETTING_KEY, &raw)
}

/// The current Top 8, in order, resolved to full `Community` records. Empty
/// if nothing has been featured yet. Any id that no longer resolves to a
/// real community is silently skipped — see the module doc comment.
pub fn get_top8(db: &Db) -> Result<Vec<Community>, Error> {
    resolve(db, &read_ids(db)?)
}

fn resolve(db: &Db, ids: &[String]) -> Result<Vec<Community>, Error> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(community) = communities::get(db, id)? {
            out.push(community);
        }
    }
    Ok(out)
}

/// Replaces the entire Top 8 with `community_ids`, in the given order.
///
/// Rejects (without writing anything) if:
/// - more than [`MAX_TOP8`] ids are given,
/// - the same id appears more than once,
/// - any id doesn't correspond to a community this device actually has
///   (catches stale/bogus ids from the UI early, rather than silently
///   storing a list that will just get filtered down later by
///   [`get_top8`]).
///
/// Returns the resolved list on success, same as [`get_top8`] would then
/// return.
pub fn set_top8(db: &Db, community_ids: &[String]) -> Result<Vec<Community>, Error> {
    if community_ids.len() > MAX_TOP8 {
        return Err(Error::Db(format!(
            "top8 supports at most {MAX_TOP8} communities, got {}",
            community_ids.len()
        )));
    }

    let mut seen = std::collections::HashSet::new();
    for id in community_ids {
        if !seen.insert(id.as_str()) {
            return Err(Error::Db(format!("duplicate community id in top8: {id:?}")));
        }
    }

    let resolved = resolve(db, community_ids)?;
    if resolved.len() != community_ids.len() {
        return Err(Error::Db(
            "top8 given at least one community id this device doesn't have".to_string(),
        ));
    }

    write_ids(db, community_ids)?;
    Ok(resolved)
}

/// Appends `community_id` to the end of the Top 8. A no-op (returns the
/// current list unchanged) if it's already featured. Errors if the Top 8 is
/// already full ([`MAX_TOP8`]) or if `community_id` doesn't correspond to a
/// real community.
pub fn add_to_top8(db: &Db, community_id: &str) -> Result<Vec<Community>, Error> {
    if communities::get(db, community_id)?.is_none() {
        return Err(Error::Db(format!(
            "cannot feature unknown community id: {community_id:?}"
        )));
    }

    let mut ids = read_ids(db)?;
    if ids.iter().any(|id| id == community_id) {
        return resolve(db, &ids);
    }
    if ids.len() >= MAX_TOP8 {
        return Err(Error::Db(format!(
            "top8 is already full ({MAX_TOP8} communities) — remove one before adding another"
        )));
    }

    ids.push(community_id.to_string());
    write_ids(db, &ids)?;
    resolve(db, &ids)
}

/// Removes `community_id` from the Top 8, if present. A no-op if it wasn't
/// featured.
pub fn remove_from_top8(db: &Db, community_id: &str) -> Result<Vec<Community>, Error> {
    let ids = read_ids(db)?;
    let filtered: Vec<String> = ids.into_iter().filter(|id| id != community_id).collect();
    write_ids(db, &filtered)?;
    resolve(db, &filtered)
}

/// Swaps `community_id` one position earlier in the Top 8. A no-op if it's
/// already first. Errors if `community_id` isn't currently featured.
pub fn move_up(db: &Db, community_id: &str) -> Result<Vec<Community>, Error> {
    shift(db, community_id, -1)
}

/// Swaps `community_id` one position later in the Top 8. A no-op if it's
/// already last. Errors if `community_id` isn't currently featured.
pub fn move_down(db: &Db, community_id: &str) -> Result<Vec<Community>, Error> {
    shift(db, community_id, 1)
}

fn shift(db: &Db, community_id: &str, delta: isize) -> Result<Vec<Community>, Error> {
    let mut ids = read_ids(db)?;
    let index = ids
        .iter()
        .position(|id| id == community_id)
        .ok_or_else(|| Error::Db(format!("community not currently in top8: {community_id:?}")))?;

    let new_index = index as isize + delta;
    if new_index < 0 || new_index as usize >= ids.len() {
        // Already at the boundary in that direction: no-op.
        return resolve(db, &ids);
    }

    ids.swap(index, new_index as usize);
    write_ids(db, &ids)?;
    resolve(db, &ids)
}

/// Small helper for `client`: the raw ordered list of featured community
/// ids, without resolving to full records. Useful for UI code that just
/// needs to know "is this community currently featured" without paying for
/// a full resolve of every entry.
pub fn top8_ids(db: &Db) -> Result<Vec<String>, Error> {
    read_ids(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Db {
        Db::open_in_memory("correct horse battery staple").unwrap()
    }

    #[test]
    fn get_top8_is_empty_when_nothing_set() {
        let db = setup();
        assert_eq!(get_top8(&db).unwrap(), Vec::new());
    }

    #[test]
    fn set_top8_persists_and_round_trips_in_order() {
        let db = setup();
        let a = communities::create(&db, "Zelda Fans").unwrap();
        let b = communities::create(&db, "Retro Netplay").unwrap();
        let c = communities::create(&db, "Speedrunning").unwrap();

        let ids = vec![c.id.clone(), a.id.clone(), b.id.clone()];
        let resolved = set_top8(&db, &ids).unwrap();
        assert_eq!(resolved, vec![c.clone(), a.clone(), b.clone()]);

        // Round-trips through a fresh read, not just the return value of set_top8.
        assert_eq!(get_top8(&db).unwrap(), vec![c, a, b]);
    }

    #[test]
    fn set_top8_rejects_more_than_eight() {
        let db = setup();
        let mut ids = Vec::new();
        for i in 0..(MAX_TOP8 + 1) {
            ids.push(
                communities::create(&db, &format!("Community {i}"))
                    .unwrap()
                    .id,
            );
        }
        assert_eq!(ids.len(), MAX_TOP8 + 1);

        let err = set_top8(&db, &ids).unwrap_err();
        assert!(err.to_string().contains("at most 8"));

        // Nothing was written on rejection.
        assert_eq!(get_top8(&db).unwrap(), Vec::new());
    }

    #[test]
    fn set_top8_rejects_duplicates_and_unknown_ids() {
        let db = setup();
        let a = communities::create(&db, "Zelda Fans").unwrap();

        assert!(set_top8(&db, &[a.id.clone(), a.id.clone()]).is_err());
        assert!(set_top8(&db, &["does-not-exist".to_string()]).is_err());
        assert_eq!(get_top8(&db).unwrap(), Vec::new());
    }

    #[test]
    fn add_to_top8_appends_is_idempotent_and_enforces_the_limit() {
        let db = setup();
        let a = communities::create(&db, "Zelda Fans").unwrap();
        let b = communities::create(&db, "Retro Netplay").unwrap();

        assert_eq!(add_to_top8(&db, &a.id).unwrap(), vec![a.clone()]);
        // Adding the same one again is a no-op, not a duplicate.
        assert_eq!(add_to_top8(&db, &a.id).unwrap(), vec![a.clone()]);
        assert_eq!(add_to_top8(&db, &b.id).unwrap(), vec![a.clone(), b.clone()]);

        // Fill up to the limit with fresh communities, then confirm a 9th is rejected.
        for i in 0..(MAX_TOP8 - 2) {
            let extra = communities::create(&db, &format!("Filler {i}")).unwrap();
            add_to_top8(&db, &extra.id).unwrap();
        }
        assert_eq!(get_top8(&db).unwrap().len(), MAX_TOP8);

        let overflow = communities::create(&db, "One Too Many").unwrap();
        let err = add_to_top8(&db, &overflow.id).unwrap_err();
        assert!(err.to_string().contains("already full"));
        assert_eq!(get_top8(&db).unwrap().len(), MAX_TOP8);
    }

    #[test]
    fn remove_from_top8_removes_and_is_a_no_op_if_absent() {
        let db = setup();
        let a = communities::create(&db, "Zelda Fans").unwrap();
        let b = communities::create(&db, "Retro Netplay").unwrap();
        set_top8(&db, &[a.id.clone(), b.id.clone()]).unwrap();

        assert_eq!(remove_from_top8(&db, &a.id).unwrap(), vec![b.clone()]);
        // Removing again (already gone) is a no-op, not an error.
        assert_eq!(remove_from_top8(&db, &a.id).unwrap(), vec![b.clone()]);
        assert_eq!(get_top8(&db).unwrap(), vec![b]);
    }

    #[test]
    fn move_up_and_down_reorder_and_respect_boundaries() {
        let db = setup();
        let a = communities::create(&db, "A").unwrap();
        let b = communities::create(&db, "B").unwrap();
        let c = communities::create(&db, "C").unwrap();
        set_top8(&db, &[a.id.clone(), b.id.clone(), c.id.clone()]).unwrap();

        // Move the middle one up: B, A, C.
        let after = move_up(&db, &b.id).unwrap();
        assert_eq!(after, vec![b.clone(), a.clone(), c.clone()]);

        // Move it back down: A, B, C.
        let after = move_down(&db, &b.id).unwrap();
        assert_eq!(after, vec![a.clone(), b.clone(), c.clone()]);

        // Already first: moving up further is a no-op.
        let after = move_up(&db, &a.id).unwrap();
        assert_eq!(after, vec![a.clone(), b.clone(), c.clone()]);

        // Already last: moving down further is a no-op.
        let after = move_down(&db, &c.id).unwrap();
        assert_eq!(after, vec![a, b, c]);
    }

    #[test]
    fn move_rejects_a_community_not_currently_featured() {
        let db = setup();
        let a = communities::create(&db, "A").unwrap();
        assert!(move_up(&db, &a.id).is_err());
        assert!(move_down(&db, &a.id).is_err());
    }

    #[test]
    fn get_top8_silently_skips_ids_that_no_longer_resolve() {
        let db = setup();
        let a = communities::create(&db, "A").unwrap();
        // Directly poke a dangling id into the setting to simulate a
        // community that vanished out from under the list (not currently
        // reachable through this module's own API, since there's no
        // delete yet — but get_top8 shouldn't ever surface a broken entry
        // if that changes later).
        write_ids(&db, &[a.id.clone(), "ghost-id".to_string()]).unwrap();

        assert_eq!(get_top8(&db).unwrap(), vec![a]);
    }

    #[test]
    fn top8_ids_returns_raw_ids_without_resolving() {
        let db = setup();
        let a = communities::create(&db, "A").unwrap();
        add_to_top8(&db, &a.id).unwrap();
        assert_eq!(top8_ids(&db).unwrap(), vec![a.id]);
    }
}
