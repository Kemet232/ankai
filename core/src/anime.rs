//! Anime metadata (via AniList's public GraphQL API) + a local watchlist.
//!
//! **Two genuinely different halves, read this before touching either.**
//!
//! 1. **The AniList client** ([`search_anime`], [`trending_anime`],
//!    [`popular_anime`]) is a real network dependency on a third-party
//!    service ANKAI does not control: `https://graphql.anilist.co`. This is
//!    categorically different from `core::directory`/ADR-0008's server work
//!    — that was ANKAI choosing and building *its own* infrastructure and
//!    is a real architecture decision (hence an ADR); this is just
//!    consuming a public read API someone else runs, the same way any app
//!    calls a weather API. No ADR, no server code, nothing to accept.
//!
//!    **Confirmed 2026-08-15** (do not assume this is still true in a
//!    future session without re-checking — third-party API terms change):
//!    unauthenticated POST requests to `https://graphql.anilist.co` work
//!    today with no API key/OAuth token for public media data (search,
//!    trending, popular sort, etc.) — verified with real live `curl`
//!    requests during development, not just documentation. AniList's own
//!    published rate-limiting docs (as surfaced via web search — the
//!    `docs.anilist.co` pages themselves 403'd every automated fetch
//!    attempted here, so this project could not read them directly) widely
//!    cite "~90 requests/minute". The real response headers observed here,
//!    though, reported `x-ratelimit-limit: 30` (and a shrinking
//!    `x-ratelimit-remaining`) on every request made during development —
//!    noted as a real discrepancy, not silently assumed away: **treat 30
//!    requests/minute per IP as the operative ceiling** until re-verified,
//!    not the more generous number circulating in older writeups. A `429`
//!    (or any non-2xx status) surfaces as a real `Err(Error::Net(..))` —
//!    this module does not retry, back off, or queue.
//!
//!    **Also confirmed, the hard way, during this module's own
//!    development:** AniList's API is subject to real, whole-service
//!    outages independent of anything a caller does — mid-development, every
//!    endpoint started returning HTTP 403 with the body
//!    `{"errors":[{"message":"The AniList API has been temporarily disabled
//!    due to severe stability issues.", ...}]}`. This is a known, recurring,
//!    publicly-documented AniList-side event (see their own Discord/forum
//!    when it happens), not specific to this client, and not something this
//!    module tries to detect or special-case — it surfaces as the same
//!    `Err(Error::Net(..))` as any other non-2xx response, containing
//!    AniList's own message text. If [`trending_anime`]/[`popular_anime`]/
//!    [`search_anime`] all fail at once with a message like that, the right
//!    read is "AniList is down right now," not "this integration is
//!    broken" — retry later by hand, there is still no automatic retry here.
//!
//! 2. **The local watchlist** (`add_to_watchlist`/`update_progress`/
//!    `list_watchlist`/`remove_from_watchlist`) is exactly as real and
//!    local-only as `communities`/`hangouts`/`forum_posts`: a `watchlist`
//!    table (migration 8) in this device's own SQLCipher-encrypted DB.
//!    Rows are keyed by AniList's own numeric media id (a stable,
//!    globally-meaningful key already, unlike the random ids
//!    `communities`/`hangouts`/`forum_posts` generate for themselves), and
//!    cache a display title (romaji/English), cover image URL, and episode
//!    count *at add-time* — see "Caching, and its one real cost" below.
//!
//! **Caching, and its one real cost.** A watchlist entry stores its own
//! copy of `title_romaji`/`title_english`/`cover_image_url`/`episode_count`
//! rather than only the `anilist_id`, so the UI can render someone's whole
//! watchlist from the local DB alone — no re-fetch of AniList, no network
//! dependency, no rate-limit consumption just to draw a screen that already
//! has all the data it needs. The real cost: **this cached metadata can go
//! stale.** If a still-airing show's cover art changes, its final episode
//! count is announced, or a title is corrected on AniList after it was
//! added here, this module has no mechanism to notice or refresh it —
//! there is no background refresh job, and re-fetching happens only if the
//! caller explicitly calls [`search_anime`]/[`trending_anime`]/
//! [`popular_anime`] again and re-adds the same id (see [`add_to_watchlist`]
//! for exactly what re-adding does and does not touch). This is a
//! deliberate scope boundary, not an oversight — a background sync job is
//! real, separate future work if staleness turns out to matter in practice.
//!
//! **What this module explicitly does not attempt:**
//! - No write-back to AniList (or MAL, or any other tracker) — this is a
//!   one-way read from AniList plus a local list, not an account-sync
//!   integration. AniList never learns this device's watchlist exists.
//! - No offline queueing/retry if AniList is unreachable. A failed request
//!   is a real `Err` the caller sees immediately, same as `core::p2p`/
//!   `core::directory`'s existing network-error handling elsewhere in this
//!   crate — nothing here silently queues and retries later.
//! - No cover-art image bytes downloaded or cached locally — only the URL
//!   string is stored. Actually fetching/caching the image itself is
//!   separate, unstarted scope (same "don't build more than what's asked"
//!   boundary `top8.rs`/`hangouts.rs` apply to their own features).
//! - No login/user-specific AniList data (a signed-in AniList user's own
//!   list, score, notes) — only the public, unauthenticated media catalog.

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::Error;

/// AniList's public GraphQL endpoint. See the module doc comment for what's
/// confirmed (as of 2026-08-15) about its auth/rate-limit posture.
const ANILIST_ENDPOINT: &str = "https://graphql.anilist.co";

/// The one GraphQL query this module ever sends, parameterized by optional
/// search text and optional sort order. `$sort` is `[MediaSort]` (a list,
/// not a bare `MediaSort`) — AniList's schema requires the list form even
/// when only one sort key is given; confirmed empirically, a bare
/// `MediaSort` variable is rejected with a real GraphQL type error.
const MEDIA_QUERY: &str = "query ($search: String, $sort: [MediaSort], $perPage: Int) {
  Page(page: 1, perPage: $perPage) {
    media(search: $search, sort: $sort, type: ANIME) {
      id
      title {
        romaji
        english
      }
      coverImage {
        large
      }
      episodes
      averageScore
    }
  }
}";

/// One anime's public metadata, as returned by AniList's `Media` type
/// (only the fields this module actually asks for).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimeSummary {
    /// AniList's own numeric media id — the stable key used everywhere in
    /// this module (including as the watchlist table's primary key).
    pub id: i64,
    /// Romaji-transliterated title. AniList's schema allows this to be
    /// null for some entries, so this is `Option`, not a bare `String`.
    pub title_romaji: Option<String>,
    /// English title, if AniList has one on file. Frequently `None` for
    /// less mainstream or not-yet-localized titles — see the real
    /// `NARUTO×UT`/`ROAD OF NARUTO` fixture entries in this module's tests.
    pub title_english: Option<String>,
    /// Cover image URL (the "large" size AniList serves), if any.
    pub cover_image_url: Option<String>,
    /// Total episode count, if AniList has settled on one yet. `None` is
    /// common and expected for currently-airing shows without an announced
    /// final count — not a parsing failure.
    pub episodes: Option<i64>,
    /// AniList's community average score (0-100), if it has enough ratings
    /// to compute one yet.
    pub average_score: Option<i64>,
}

// --- Wire-format types: shaped to exactly match AniList's real GraphQL
// response envelope, not this module's own `AnimeSummary`. Kept private and
// converted via `From` below so `AnimeSummary`'s public shape can evolve
// independently of AniList's exact field names.

#[derive(Debug, Deserialize)]
struct GraphQlEnvelope {
    data: Option<PageData>,
    errors: Option<Vec<GraphQlErrorDetail>>,
}

#[derive(Debug, Deserialize)]
struct GraphQlErrorDetail {
    message: String,
}

#[derive(Debug, Deserialize)]
struct PageData {
    #[serde(rename = "Page")]
    page: MediaPage,
}

#[derive(Debug, Deserialize)]
struct MediaPage {
    media: Vec<MediaNode>,
}

#[derive(Debug, Deserialize)]
struct MediaNode {
    id: i64,
    title: MediaTitle,
    #[serde(rename = "coverImage")]
    cover_image: Option<CoverImage>,
    episodes: Option<i64>,
    #[serde(rename = "averageScore")]
    average_score: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct MediaTitle {
    romaji: Option<String>,
    english: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CoverImage {
    large: Option<String>,
}

impl From<MediaNode> for AnimeSummary {
    fn from(node: MediaNode) -> Self {
        AnimeSummary {
            id: node.id,
            title_romaji: node.title.romaji,
            title_english: node.title.english,
            cover_image_url: node.cover_image.and_then(|c| c.large),
            episodes: node.episodes,
            average_score: node.average_score,
        }
    }
}

/// Searches AniList's anime catalog by free-text title. Returns up to 25
/// matches, in whatever relevance order AniList's own `search` argument
/// applies (no explicit `sort` is sent).
pub async fn search_anime(query: &str) -> Result<Vec<AnimeSummary>, Error> {
    query_media(Some(query), None, 25).await
}

/// AniList's "trending" ranking (`TRENDING_DESC`) — activity-weighted, not
/// the same ordering as [`popular_anime`]. Returns up to `limit` entries.
pub async fn trending_anime(limit: usize) -> Result<Vec<AnimeSummary>, Error> {
    query_media(None, Some("TRENDING_DESC"), limit).await
}

/// AniList's "popular" ranking (`POPULARITY_DESC`, total list-adds across
/// all AniList users) — returns up to `limit` entries.
pub async fn popular_anime(limit: usize) -> Result<Vec<AnimeSummary>, Error> {
    query_media(None, Some("POPULARITY_DESC"), limit).await
}

/// Shared implementation behind all three public query functions: builds
/// the GraphQL request body, sends it, and parses the response. Real HTTP
/// via `reqwest`, real JSON via `serde` — no mocked responses.
async fn query_media(
    search: Option<&str>,
    sort: Option<&str>,
    per_page: usize,
) -> Result<Vec<AnimeSummary>, Error> {
    let body = serde_json::json!({
        "query": MEDIA_QUERY,
        "variables": {
            "search": search,
            "sort": sort.map(|s| vec![s]),
            "perPage": per_page as i64,
        },
    });

    let client = reqwest::Client::new();
    let response = client
        .post(ANILIST_ENDPOINT)
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Net(format!("AniList request failed: {e}")))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| Error::Net(format!("failed to read AniList response body: {e}")))?;

    if !status.is_success() {
        return Err(Error::Net(format!(
            "AniList returned HTTP {status}: {text}"
        )));
    }

    parse_media_response(&text)
}

/// Parses a raw AniList GraphQL response body into a list of
/// [`AnimeSummary`]. Split out from [`query_media`] so it can be
/// unit-tested against captured real response fixtures without any
/// network access — see this module's tests.
fn parse_media_response(text: &str) -> Result<Vec<AnimeSummary>, Error> {
    let envelope: GraphQlEnvelope = serde_json::from_str(text)
        .map_err(|e| Error::Net(format!("failed to parse AniList response as JSON: {e}")))?;

    if let Some(errors) = envelope.errors {
        let messages: Vec<String> = errors.into_iter().map(|e| e.message).collect();
        return Err(Error::Net(format!(
            "AniList returned GraphQL error(s): {}",
            messages.join("; ")
        )));
    }

    let data = envelope.data.ok_or_else(|| {
        Error::Net("AniList response had neither `data` nor `errors`".to_string())
    })?;

    Ok(data
        .page
        .media
        .into_iter()
        .map(AnimeSummary::from)
        .collect())
}

// --- Local watchlist ---------------------------------------------------

/// Where a watchlist entry stands. Matches this repo's existing
/// `CHECK`-constraint-backed enum convention (see `messages.direction` in
/// `db.rs`'s migration 6) rather than a free-text status field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WatchStatus {
    Watching,
    Completed,
    Planned,
    Dropped,
}

impl WatchStatus {
    fn as_db_str(self) -> &'static str {
        match self {
            WatchStatus::Watching => "watching",
            WatchStatus::Completed => "completed",
            WatchStatus::Planned => "planned",
            WatchStatus::Dropped => "dropped",
        }
    }

    fn from_db_str(s: &str) -> Result<Self, Error> {
        match s {
            "watching" => Ok(WatchStatus::Watching),
            "completed" => Ok(WatchStatus::Completed),
            "planned" => Ok(WatchStatus::Planned),
            "dropped" => Ok(WatchStatus::Dropped),
            other => Err(Error::Db(format!(
                "unknown watchlist status in database: {other:?}"
            ))),
        }
    }
}

/// A single watchlist row: an anime this device's user is tracking, plus
/// their own progress/status. `title_romaji`/`title_english`/
/// `cover_image_url`/`episode_count` are this device's *cached* copy of
/// that AniList metadata as of whenever it was last added/refreshed — see
/// the module doc comment's "Caching, and its one real cost" section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchlistEntry {
    pub anilist_id: i64,
    pub title_romaji: Option<String>,
    pub title_english: Option<String>,
    pub cover_image_url: Option<String>,
    /// Cached total episode count at add/refresh time. `None` if AniList
    /// hadn't settled on one yet — see [`AnimeSummary::episodes`].
    pub episode_count: Option<i64>,
    /// How many episodes the user has actually watched. Not clamped
    /// against `episode_count` here: that cached count can be stale (a
    /// still-airing show can announce more episodes after this row was
    /// added), so refusing progress past a possibly-outdated number would
    /// be its own bug, not real validation.
    pub watched_episodes: i64,
    pub status: WatchStatus,
}

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<WatchlistEntry> {
    let status_raw: String = row.get(6)?;
    let status = WatchStatus::from_db_str(&status_raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(e.to_string())),
        )
    })?;
    Ok(WatchlistEntry {
        anilist_id: row.get(0)?,
        title_romaji: row.get(1)?,
        title_english: row.get(2)?,
        cover_image_url: row.get(3)?,
        episode_count: row.get(4)?,
        watched_episodes: row.get(5)?,
        status,
    })
}

const SELECT_COLUMNS: &str = "anilist_id, title_romaji, title_english, cover_image_url, episode_count, watched_episodes, status";

/// Adds `anime` to the local watchlist with the given initial `status`.
///
/// This is an **upsert**, not an insert-or-error: adding an anime that's
/// already on the watchlist refreshes its cached
/// title/cover/episode-count/status from `anime` (useful if the caller
/// re-fetched fresher AniList data), but **does not touch
/// `watched_episodes`** — re-adding a show you're partway through must
/// never silently reset your progress back to zero. Use [`update_progress`]
/// to change progress.
pub fn add_to_watchlist(
    db: &Db,
    anime: &AnimeSummary,
    status: WatchStatus,
) -> Result<WatchlistEntry, Error> {
    db.connection()
        .execute(
            "INSERT INTO watchlist
                (anilist_id, title_romaji, title_english, cover_image_url, episode_count, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(anilist_id) DO UPDATE SET
                title_romaji    = excluded.title_romaji,
                title_english   = excluded.title_english,
                cover_image_url = excluded.cover_image_url,
                episode_count   = excluded.episode_count,
                status          = excluded.status,
                updated_at      = datetime('now')",
            rusqlite::params![
                anime.id,
                anime.title_romaji,
                anime.title_english,
                anime.cover_image_url,
                anime.episodes,
                status.as_db_str(),
            ],
        )
        .map_err(|e| {
            Error::Db(format!(
                "failed to add anime {} to watchlist: {e}",
                anime.id
            ))
        })?;

    get_watchlist_entry(db, anime.id)?.ok_or_else(|| {
        Error::Db(format!(
            "watchlist entry for {} missing immediately after insert",
            anime.id
        ))
    })
}

/// Updates how many episodes the user has watched for an anime already on
/// the watchlist. Errors (without writing anything) if `watched_episodes`
/// is negative, or if `anilist_id` isn't currently on the watchlist.
pub fn update_progress(
    db: &Db,
    anilist_id: i64,
    watched_episodes: i64,
) -> Result<WatchlistEntry, Error> {
    if watched_episodes < 0 {
        return Err(Error::Db(format!(
            "watched_episodes cannot be negative, got {watched_episodes}"
        )));
    }

    let rows_changed = db
        .connection()
        .execute(
            "UPDATE watchlist SET watched_episodes = ?1, updated_at = datetime('now')
             WHERE anilist_id = ?2",
            rusqlite::params![watched_episodes, anilist_id],
        )
        .map_err(|e| Error::Db(format!("failed to update progress for {anilist_id}: {e}")))?;

    if rows_changed == 0 {
        return Err(Error::Db(format!(
            "anime {anilist_id} is not on the watchlist"
        )));
    }

    get_watchlist_entry(db, anilist_id)?.ok_or_else(|| {
        Error::Db(format!(
            "watchlist entry for {anilist_id} missing immediately after update"
        ))
    })
}

/// Updates the status (watching/completed/planned/dropped) of an anime
/// already on the watchlist, without needing a fresh [`AnimeSummary`] the
/// way re-calling [`add_to_watchlist`] would. Errors if `anilist_id` isn't
/// currently on the watchlist.
pub fn update_status(
    db: &Db,
    anilist_id: i64,
    status: WatchStatus,
) -> Result<WatchlistEntry, Error> {
    let rows_changed = db
        .connection()
        .execute(
            "UPDATE watchlist SET status = ?1, updated_at = datetime('now')
             WHERE anilist_id = ?2",
            rusqlite::params![status.as_db_str(), anilist_id],
        )
        .map_err(|e| Error::Db(format!("failed to update status for {anilist_id}: {e}")))?;

    if rows_changed == 0 {
        return Err(Error::Db(format!(
            "anime {anilist_id} is not on the watchlist"
        )));
    }

    get_watchlist_entry(db, anilist_id)?.ok_or_else(|| {
        Error::Db(format!(
            "watchlist entry for {anilist_id} missing immediately after update"
        ))
    })
}

/// Lists watchlist entries, oldest-added first, optionally filtered to a
/// single `status` (e.g. `Some(WatchStatus::Watching)` for "currently
/// watching"). `None` returns the entire watchlist regardless of status.
pub fn list_watchlist(db: &Db, status: Option<WatchStatus>) -> Result<Vec<WatchlistEntry>, Error> {
    let conn = db.connection();

    match status {
        Some(status) => {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {SELECT_COLUMNS} FROM watchlist WHERE status = ?1 ORDER BY added_at ASC"
                ))
                .map_err(|e| Error::Db(format!("failed to prepare watchlist query: {e}")))?;
            let rows = stmt
                .query_map(rusqlite::params![status.as_db_str()], row_to_entry)
                .map_err(|e| Error::Db(format!("failed to query watchlist: {e}")))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::Db(format!("failed to read watchlist row: {e}")))
        }
        None => {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {SELECT_COLUMNS} FROM watchlist ORDER BY added_at ASC"
                ))
                .map_err(|e| Error::Db(format!("failed to prepare watchlist query: {e}")))?;
            let rows = stmt
                .query_map([], row_to_entry)
                .map_err(|e| Error::Db(format!("failed to query watchlist: {e}")))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::Db(format!("failed to read watchlist row: {e}")))
        }
    }
}

/// Fetches a single watchlist entry by AniList id, or `None` if it isn't on
/// the watchlist.
pub fn get_watchlist_entry(db: &Db, anilist_id: i64) -> Result<Option<WatchlistEntry>, Error> {
    db.connection()
        .query_row(
            &format!("SELECT {SELECT_COLUMNS} FROM watchlist WHERE anilist_id = ?1"),
            rusqlite::params![anilist_id],
            row_to_entry,
        )
        .optional()
        .map_err(|e| Error::Db(format!("failed to read watchlist entry {anilist_id}: {e}")))
}

/// Removes an anime from the watchlist, if present. A no-op (not an error)
/// if it wasn't on the watchlist — matching `top8::remove_from_top8`'s
/// "removing something already absent is fine" convention.
pub fn remove_from_watchlist(db: &Db, anilist_id: i64) -> Result<(), Error> {
    db.connection()
        .execute(
            "DELETE FROM watchlist WHERE anilist_id = ?1",
            rusqlite::params![anilist_id],
        )
        .map_err(|e| Error::Db(format!("failed to remove {anilist_id} from watchlist: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- GraphQL response parsing, against real captured AniList response
    // bodies (not hand-written approximations) — see the module doc
    // comment's note on how/when these were captured. No network access;
    // these run as part of the normal `cargo test --workspace` suite.

    /// A real response body captured 2026-08-15 from
    /// `POST https://graphql.anilist.co` with
    /// `{ Page(page:1,perPage:3){ media(sort:TRENDING_DESC,type:ANIME){...} } }`.
    /// Deliberately includes an entry with `episodes: null` (an
    /// airing-with-no-announced-final-count show), the exact real-world
    /// shape this module's `Option<i64>` handling exists for.
    const REAL_TRENDING_RESPONSE_FIXTURE: &str = r#"{"data":{"Page":{"media":[{"id":185874,"title":{"romaji":"BLEACH: Sennen Kessen-hen - Kashin-tan","english":"BLEACH: Thousand-Year Blood War - The Calamity"},"coverImage":{"large":"https:\/\/s4.anilist.co\/file\/anilistcdn\/media\/anime\/cover\/medium\/bx185874-aU3e6tBT6wwA.jpg"},"episodes":10,"averageScore":88},{"id":182205,"title":{"romaji":"Tensei Shitara Slime Datta Ken 4th Season","english":"That Time I Got Reincarnated as a Slime Season 4"},"coverImage":{"large":"https:\/\/s4.anilist.co\/file\/anilistcdn\/media\/anime\/cover\/medium\/bx182205-q2AeO1owuQbO.jpg"},"episodes":null,"averageScore":82},{"id":187538,"title":{"romaji":"BLACK TORCH","english":"BLACK TORCH"},"coverImage":{"large":"https:\/\/s4.anilist.co\/file\/anilistcdn\/media\/anime\/cover\/medium\/bx187538-fXVXKYUA3VV6.jpg"},"episodes":null,"averageScore":71}]}}}"#;

    /// A real GraphQL error-response body captured 2026-08-15 by
    /// deliberately sending a query field that doesn't exist
    /// (`{ ThisFieldDoesNotExist }`) — the real shape AniList uses for
    /// GraphQL-level (not HTTP-level) errors.
    const REAL_GRAPHQL_ERROR_FIXTURE: &str = r#"{"errors":[{"message":"Cannot query field \"ThisFieldDoesNotExist\" on type \"Query\".","status":400,"locations":[{"line":1,"column":7}]}],"data":null}"#;

    #[test]
    fn parses_a_real_captured_trending_response() {
        let result = parse_media_response(REAL_TRENDING_RESPONSE_FIXTURE).unwrap();
        assert_eq!(result.len(), 3);

        assert_eq!(result[0].id, 185874);
        assert_eq!(
            result[0].title_romaji.as_deref(),
            Some("BLEACH: Sennen Kessen-hen - Kashin-tan")
        );
        assert_eq!(
            result[0].title_english.as_deref(),
            Some("BLEACH: Thousand-Year Blood War - The Calamity")
        );
        assert_eq!(result[0].episodes, Some(10));
        assert_eq!(result[0].average_score, Some(88));
        assert!(result[0]
            .cover_image_url
            .as_deref()
            .unwrap()
            .starts_with("https://s4.anilist.co/"));

        // The real-world "airing show with no announced final episode
        // count yet" case: episodes must parse as None, not error or 0.
        assert_eq!(result[1].id, 182205);
        assert_eq!(result[1].episodes, None);
    }

    #[test]
    fn parses_a_real_captured_graphql_error_response_as_an_error() {
        let err = parse_media_response(REAL_GRAPHQL_ERROR_FIXTURE).unwrap_err();
        assert!(
            err.to_string().contains("ThisFieldDoesNotExist"),
            "error message should surface AniList's own GraphQL error text, got: {err}"
        );
    }

    #[test]
    fn rejects_malformed_non_json_response() {
        let err = parse_media_response("not json at all").unwrap_err();
        assert!(matches!(err, Error::Net(_)));
    }

    #[test]
    fn rejects_response_with_neither_data_nor_errors() {
        let err = parse_media_response("{}").unwrap_err();
        assert!(matches!(err, Error::Net(_)));
    }

    // --- Local watchlist, in-memory DB, no network. Mirrors
    // hangouts.rs/top8.rs's testing rigor: round-trip, filtering,
    // idempotence/no-op behavior, and rejection paths.

    fn setup() -> Db {
        Db::open_in_memory("correct horse battery staple").unwrap()
    }

    fn sample_anime(id: i64, romaji: &str) -> AnimeSummary {
        AnimeSummary {
            id,
            title_romaji: Some(romaji.to_string()),
            title_english: Some(format!("{romaji} (EN)")),
            cover_image_url: Some(format!("https://example.invalid/cover/{id}.jpg")),
            episodes: Some(24),
            average_score: Some(85),
        }
    }

    #[test]
    fn add_to_watchlist_persists_and_round_trips() {
        let db = setup();
        let anime = sample_anime(20, "NARUTO");

        let entry = add_to_watchlist(&db, &anime, WatchStatus::Planned).unwrap();
        assert_eq!(entry.anilist_id, 20);
        assert_eq!(entry.title_romaji.as_deref(), Some("NARUTO"));
        assert_eq!(entry.watched_episodes, 0);
        assert_eq!(entry.status, WatchStatus::Planned);

        // Round-trips through a fresh read, not just the return value.
        assert_eq!(get_watchlist_entry(&db, 20).unwrap(), Some(entry));
    }

    #[test]
    fn add_to_watchlist_is_an_upsert_that_preserves_progress() {
        let db = setup();
        let anime = sample_anime(20, "NARUTO");

        add_to_watchlist(&db, &anime, WatchStatus::Watching).unwrap();
        update_progress(&db, 20, 15).unwrap();

        // Re-adding with fresher (different) cached metadata + a new
        // status must refresh the cache and status, but must NOT reset
        // watched_episodes back to 0.
        let mut refreshed = sample_anime(20, "NARUTO (Remastered Title)");
        refreshed.episodes = Some(220);
        let entry = add_to_watchlist(&db, &refreshed, WatchStatus::Completed).unwrap();

        assert_eq!(
            entry.title_romaji.as_deref(),
            Some("NARUTO (Remastered Title)")
        );
        assert_eq!(entry.episode_count, Some(220));
        assert_eq!(entry.status, WatchStatus::Completed);
        assert_eq!(
            entry.watched_episodes, 15,
            "re-adding must not silently wipe existing progress"
        );
    }

    #[test]
    fn update_progress_persists_and_rejects_negative_and_unknown() {
        let db = setup();
        add_to_watchlist(&db, &sample_anime(1, "A"), WatchStatus::Watching).unwrap();

        let entry = update_progress(&db, 1, 5).unwrap();
        assert_eq!(entry.watched_episodes, 5);
        assert_eq!(
            get_watchlist_entry(&db, 1)
                .unwrap()
                .unwrap()
                .watched_episodes,
            5
        );

        let err = update_progress(&db, 1, -1).unwrap_err();
        assert!(err.to_string().contains("negative"));

        let err = update_progress(&db, 999, 3).unwrap_err();
        assert!(err.to_string().contains("not on the watchlist"));
    }

    #[test]
    fn update_status_persists_and_rejects_unknown() {
        let db = setup();
        add_to_watchlist(&db, &sample_anime(1, "A"), WatchStatus::Watching).unwrap();

        let entry = update_status(&db, 1, WatchStatus::Dropped).unwrap();
        assert_eq!(entry.status, WatchStatus::Dropped);
        assert_eq!(
            get_watchlist_entry(&db, 1).unwrap().unwrap().status,
            WatchStatus::Dropped
        );

        assert!(update_status(&db, 999, WatchStatus::Watching).is_err());
    }

    #[test]
    fn list_watchlist_filters_by_status_and_defaults_to_everything() {
        let db = setup();
        add_to_watchlist(&db, &sample_anime(1, "Watching A"), WatchStatus::Watching).unwrap();
        add_to_watchlist(&db, &sample_anime(2, "Watching B"), WatchStatus::Watching).unwrap();
        add_to_watchlist(&db, &sample_anime(3, "Completed C"), WatchStatus::Completed).unwrap();
        add_to_watchlist(&db, &sample_anime(4, "Planned D"), WatchStatus::Planned).unwrap();

        let watching = list_watchlist(&db, Some(WatchStatus::Watching)).unwrap();
        assert_eq!(
            watching.iter().map(|e| e.anilist_id).collect::<Vec<_>>(),
            vec![1, 2]
        );

        let completed = list_watchlist(&db, Some(WatchStatus::Completed)).unwrap();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].anilist_id, 3);

        let dropped = list_watchlist(&db, Some(WatchStatus::Dropped)).unwrap();
        assert_eq!(dropped, Vec::new());

        let all = list_watchlist(&db, None).unwrap();
        assert_eq!(
            all.iter().map(|e| e.anilist_id).collect::<Vec<_>>(),
            vec![1, 2, 3, 4],
            "unfiltered list should return everything, oldest-added first"
        );
    }

    #[test]
    fn remove_from_watchlist_removes_and_is_a_no_op_if_absent() {
        let db = setup();
        add_to_watchlist(&db, &sample_anime(1, "A"), WatchStatus::Watching).unwrap();
        add_to_watchlist(&db, &sample_anime(2, "B"), WatchStatus::Watching).unwrap();

        remove_from_watchlist(&db, 1).unwrap();
        assert_eq!(get_watchlist_entry(&db, 1).unwrap(), None);
        assert!(get_watchlist_entry(&db, 2).unwrap().is_some());

        // Removing again (already gone) is a no-op, not an error.
        remove_from_watchlist(&db, 1).unwrap();
    }

    #[test]
    fn get_watchlist_entry_returns_none_for_unknown_id() {
        let db = setup();
        assert_eq!(get_watchlist_entry(&db, 424242).unwrap(), None);
    }
}
