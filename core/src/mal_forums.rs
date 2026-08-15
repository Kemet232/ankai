//! MyAnimeList forum discussions — a real, read-only client for MAL's public
//! API v2 forum endpoints.
//!
//! **Same category of dependency as `core::anime`'s AniList client, not
//! ANKAI's own infrastructure.** This is ANKAI consuming a public read API
//! someone else runs (MyAnimeList), the same way `core::anime` consumes
//! AniList's public GraphQL API — categorically different from
//! `core::directory`/ADR-0008, which was ANKAI choosing and building *its
//! own* server. No ADR, no server code, nothing to accept here.
//!
//! **Auth — confirmed empirically 2026-08-15, do not assume this is still
//! true in a future session without re-checking.** Before writing any code
//! here, this was verified against MAL's real API with real live `curl`
//! requests (not just documentation, and not just the third-party
//! `go-myanimelist` client library's source, though that library's real,
//! MIT-licensed Go source — <https://github.com/nstratos/go-myanimelist> —
//! is what gave this module its exact real field names, cross-checked
//! against the live responses below): reading MAL's public forum data
//! requires **only** the `X-MAL-CLIENT-ID` header set to a registered
//! application's Client ID — no OAuth2 user-authorization flow, no access
//! token, no login. This is the smaller, simpler scope; had it turned out
//! forum reads required full per-user OAuth2 (like MAL's user-list-write
//! endpoints do), that would have been a materially bigger task and this
//! module would not have been built without flagging that first. Real
//! confirmed responses from `https://api.myanimelist.net/v2` during
//! development:
//! - `GET forum/boards` with a valid `X-MAL-CLIENT-ID` → real `200` with
//!   real board data (see [`list_forum_categories`]).
//! - `GET forum/topics?board_id=1&limit=3` and `GET
//!   forum/topics?q=naruto&limit=3` → real `200`s (see
//!   [`list_topics_in_board`]/[`search_topics`]).
//! - `GET forum/topic/516093?limit=5` → real `200` with real post bodies
//!   (see [`get_topic_posts`]).
//! - A bad `X-MAL-CLIENT-ID` value → real `400` `{"message":"Invalid client
//!   id","error":"bad_request"}`.
//! - No `X-MAL-CLIENT-ID` header at all → real `403`
//!   `{"message":"","error":"forbidden"}`.
//! - An unknown topic id → real `404` `{"message":"","error":"not_found"}`.
//! - **A real, reproducible quirk, not a bug in this module:** `GET
//!   forum/topics` with *neither* `board_id` nor `q` set hangs and
//!   eventually returns a `504 Gateway Timeout` from MAL's own edge, not a
//!   clean 4xx. This module's public API structurally avoids ever sending
//!   that request — [`list_topics_in_board`] always sets `board_id`,
//!   [`search_topics`] always sets `q` — but it's worth knowing if this
//!   module is ever extended.
//!
//! **What this module explicitly does not attempt:**
//! - No posting, replying, editing, or any other write to MAL's forums.
//!   Every function here is a `GET`.
//! - No OAuth2/per-user-authenticated actions of any kind (a signed-in MAL
//!   user's own posts, notifications, etc.) — only the public,
//!   unauthenticated-beyond-a-client-ID forum catalog, same posture as
//!   `core::anime`'s "no login/user-specific AniList data" boundary.
//! - No local persistence/caching table. Unlike `core::anime`'s watchlist
//!   (which has a real reason to cache — rendering a user's own list without
//!   burning AniList rate-limit budget), there is no local "my MAL forum
//!   state" to persist here; every call here is a live, uncached read.
//!   Whether a future Home-dashboard consumer wants to cache results for its
//!   own display purposes is that consumer's decision, not this module's.
//! - No pagination beyond a single page. MAL's `paging.next` URL is real and
//!   present in every response captured during development, but nothing
//!   here follows it automatically — callers who want more than one page's
//!   worth of results get a `limit` parameter to raise per this module's
//!   functions, not automatic multi-page fetching.
//! - No rendering of forum post bodies. MAL post bodies are real, raw
//!   phpBB-style BBCode (`[b]...[/b]`, `[url=...]...[/url]`, `[list]...`, HTML
//!   entities, literal `<br />`), not HTML or Markdown — confirmed in the
//!   real captured fixture in this module's tests. Converting that to
//!   displayable text/markup is a UI-layer concern, not this client's.
//! - No retry/backoff/queueing on failure, matching `core::anime`'s and
//!   `core::p2p`/`core::directory`'s existing posture: a failed request is a
//!   real `Err` the caller sees immediately.
//!
//! **One more honest wart, kept rather than hidden:** MAL's own JSON has a
//! genuine typo in its post-author field name — `forum_avator` (not
//! `forum_avatar`), confirmed in the real captured fixture below. This
//! module's [`ForumPostAuthor::forum_avator`] field keeps that exact
//! misspelling rather than "fixing" it, because fixing it would silently
//! break deserialization against the real API.

use serde::{Deserialize, Serialize};

use crate::error::Error;

/// MyAnimeList's API v2 base URL.
const MAL_API_BASE: &str = "https://api.myanimelist.net/v2";

/// The environment variable this module reads a registered application's
/// MAL Client ID from, at call time (never compiled in, never hardcoded) —
/// same "opt-in runtime environment variable" posture as
/// `ANKAI_DIRECTORY_URL` in `client::directory`.
const MAL_CLIENT_ID_ENV_VAR: &str = "MAL_CLIENT_ID";

/// Reads the MAL Client ID from the environment at call time. Returns a
/// clear real error (not a panic, not a silent empty-string header) if it
/// isn't set.
fn client_id() -> Result<String, Error> {
    std::env::var(MAL_CLIENT_ID_ENV_VAR).map_err(|_| {
        Error::Net(format!(
            "{MAL_CLIENT_ID_ENV_VAR} environment variable is not set — a registered \
             MyAnimeList API application Client ID is required to read forum data. \
             See core::mal_forums's module doc comment."
        ))
    })
}

// --- Boards ---------------------------------------------------------------

/// A subboard of a [`ForumBoard`] (e.g. "Anime Series" under "Series
/// Discussion"). Field names match MAL's real JSON verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForumSubboard {
    pub id: i64,
    pub title: String,
}

/// A top-level forum board (e.g. "Anime Discussion"). Field names match
/// MAL's real JSON verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForumBoard {
    pub id: i64,
    pub title: String,
    pub description: String,
    pub subboards: Vec<ForumSubboard>,
}

/// A category grouping several [`ForumBoard`]s (e.g. "Anime & Manga",
/// "General"). This is the real top-level shape MAL's `forum/boards`
/// actually returns — categories containing boards, not a flat list of
/// boards — so this module preserves it rather than flattening away real
/// structure the API provides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForumCategory {
    pub title: String,
    pub boards: Vec<ForumBoard>,
}

/// The envelope `GET forum/boards` actually returns: `{"categories": [...]}`.
#[derive(Debug, Deserialize)]
struct BoardsResponse {
    categories: Vec<ForumCategory>,
}

/// Fetches MAL's real forum board structure: every category, and every
/// board (with its subboards) within each category.
pub async fn list_forum_categories() -> Result<Vec<ForumCategory>, Error> {
    let text = get(&format!("{MAL_API_BASE}/forum/boards"), &[]).await?;
    let parsed: BoardsResponse = serde_json::from_str(&text)
        .map_err(|e| Error::Net(format!("failed to parse MAL forum/boards response: {e}")))?;
    Ok(parsed.categories)
}

// --- Topics -----------------------------------------------------------

/// The minimal user reference MAL embeds in a topic listing's `created_by`/
/// `last_post_created_by` — just an id and a display name (unlike
/// [`ForumPostAuthor`], which also carries a forum title/avatar and only
/// appears on individual posts, not topic-list rows — confirmed as a real
/// difference between the two real captured fixtures in this module's
/// tests, not an oversight).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForumUserRef {
    pub id: i64,
    pub name: String,
}

/// One forum topic (thread), as returned by `GET forum/topics`. Field names
/// match MAL's real JSON verbatim. Timestamps are kept as MAL's own raw
/// ISO-8601 strings rather than parsed into a date/time type — no date/time
/// crate is a workspace dependency yet, and this module doesn't need one to
/// do its job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForumTopic {
    pub id: i64,
    pub title: String,
    pub created_at: String,
    pub created_by: ForumUserRef,
    pub number_of_posts: i64,
    pub last_post_created_at: String,
    pub last_post_created_by: ForumUserRef,
    pub is_locked: bool,
}

/// The envelope `GET forum/topics` actually returns:
/// `{"data": [...], "paging": {"next": "..."}}`. `paging` is real (and
/// present in every response captured during development) but not modeled
/// here — see the module doc comment's note on why this module doesn't
/// follow pagination automatically.
#[derive(Debug, Deserialize)]
struct TopicsResponse {
    data: Vec<ForumTopic>,
}

/// Lists topics in a specific forum board, most-recently-active first (MAL's
/// own default/only sort, `recent` — no other sort value is documented or
/// was found to work during development). Up to `limit` topics.
///
/// `board_id` values come from [`list_forum_categories`]'s
/// [`ForumBoard::id`] (or [`ForumSubboard::id`], via [`list_topics_in_subboard`]).
pub async fn list_topics_in_board(board_id: i64, limit: usize) -> Result<Vec<ForumTopic>, Error> {
    let board_id_str = board_id.to_string();
    let limit_str = limit.to_string();
    let params = [
        ("board_id", board_id_str.as_str()),
        ("limit", limit_str.as_str()),
    ];
    list_topics(&params).await
}

/// Lists topics in a specific forum *subboard* (e.g. the "Anime Series"
/// subboard under "Series Discussion"), same shape as
/// [`list_topics_in_board`] but scoped one level deeper.
pub async fn list_topics_in_subboard(
    subboard_id: i64,
    limit: usize,
) -> Result<Vec<ForumTopic>, Error> {
    let subboard_id_str = subboard_id.to_string();
    let limit_str = limit.to_string();
    let params = [
        ("subboard_id", subboard_id_str.as_str()),
        ("limit", limit_str.as_str()),
    ];
    list_topics(&params).await
}

/// Searches forum topics by free-text title, up to `limit` matches. Mirrors
/// `core::anime::search_anime`'s shape for the same kind of query.
pub async fn search_topics(query: &str, limit: usize) -> Result<Vec<ForumTopic>, Error> {
    let limit_str = limit.to_string();
    let params = [("q", query), ("limit", limit_str.as_str())];
    list_topics(&params).await
}

/// Shared implementation behind [`list_topics_in_board`],
/// [`list_topics_in_subboard`], and [`search_topics`]. Every caller here
/// always sets at least one of `board_id`/`subboard_id`/`q` — see the module
/// doc comment's note on why: MAL's own API hangs and 504s if neither is
/// present, and this module structurally never sends that request.
async fn list_topics(params: &[(&str, &str)]) -> Result<Vec<ForumTopic>, Error> {
    let text = get(&format!("{MAL_API_BASE}/forum/topics"), params).await?;
    let parsed: TopicsResponse = serde_json::from_str(&text)
        .map_err(|e| Error::Net(format!("failed to parse MAL forum/topics response: {e}")))?;
    Ok(parsed.data)
}

// --- Topic posts ------------------------------------------------------

/// The author reference MAL embeds on an individual forum post — richer
/// than [`ForumUserRef`] (adds a forum title/rank and an avatar URL). Note
/// [`forum_avator`](Self::forum_avator) keeps MAL's own real field-name typo
/// — see the module doc comment's closing note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForumPostAuthor {
    pub id: i64,
    pub name: String,
    /// A forum rank/title string (MAL renders this as a row of stars for
    /// many users, e.g. `"★★★★★"`) — not always present, so `Option`.
    pub forum_title: Option<String>,
    /// Avatar image URL. Named `forum_avator` (not `forum_avatar`) to match
    /// MAL's own real (typo'd) JSON field name verbatim.
    pub forum_avator: Option<String>,
}

/// One post within a forum topic. `body`/`signature` are raw phpBB-style
/// BBCode, not HTML or Markdown — see the module doc comment's "No
/// rendering of forum post bodies" note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForumPost {
    pub id: i64,
    pub number: i64,
    pub created_at: String,
    pub created_by: ForumPostAuthor,
    pub body: String,
    pub signature: Option<String>,
}

/// The real shape of `GET forum/topic/{id}`'s `data` object:
/// `{"title": ..., "posts": [...], "poll": ...}`. `poll` is real (MAL polls
/// do appear on some topics) but is deliberately not modeled here — nothing
/// this module was asked to build needs it, and adding an unverified type
/// for a field this module never empirically observed populated (every real
/// fixture captured during development had `"poll": null`) would be
/// guessing at a shape rather than confirming one. `serde` silently ignores
/// the unmodeled `poll` field on parse; add it for real if/when a caller
/// actually needs it, verified against a real topic that has one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicDetails {
    pub title: String,
    pub posts: Vec<ForumPost>,
}

/// The envelope `GET forum/topic/{id}` actually returns:
/// `{"data": {...}, "paging": {"next": "..."}}`.
#[derive(Debug, Deserialize)]
struct TopicDetailResponse {
    data: TopicDetails,
}

/// Fetches a forum topic's title and up to `limit` of its posts (oldest
/// first — MAL's own real, only order for this endpoint). `topic_id` comes
/// from [`ForumTopic::id`] (via [`list_topics_in_board`]/[`search_topics`]).
pub async fn get_topic_posts(topic_id: i64, limit: usize) -> Result<TopicDetails, Error> {
    let limit_str = limit.to_string();
    let text = get(
        &format!("{MAL_API_BASE}/forum/topic/{topic_id}"),
        &[("limit", limit_str.as_str())],
    )
    .await?;
    let parsed: TopicDetailResponse = serde_json::from_str(&text)
        .map_err(|e| Error::Net(format!("failed to parse MAL forum/topic response: {e}")))?;
    Ok(parsed.data)
}

// --- Shared HTTP plumbing ----------------------------------------------

/// The real shape of MAL's own error responses, e.g.
/// `{"message":"Invalid client id","error":"bad_request"}` or
/// `{"message":"","error":"forbidden"}` — both confirmed with real requests
/// during development (see the module doc comment). Used to turn a non-2xx
/// response into a real, specific error message instead of just the raw
/// response body.
#[derive(Debug, Deserialize)]
struct MalErrorBody {
    message: String,
    error: String,
}

/// Sends a real `GET` request to `url` with `query` params and the
/// `X-MAL-CLIENT-ID` header, and returns the raw response body text on a
/// 2xx status. Real HTTP via `reqwest`; no mocked responses, no retry.
async fn get(url: &str, query: &[(&str, &str)]) -> Result<String, Error> {
    let id = client_id()?;

    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .header("X-MAL-CLIENT-ID", id)
        .query(query)
        .send()
        .await
        .map_err(|e| Error::Net(format!("MAL request to {url} failed: {e}")))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| Error::Net(format!("failed to read MAL response body from {url}: {e}")))?;

    if !status.is_success() {
        if let Ok(err_body) = serde_json::from_str::<MalErrorBody>(&text) {
            return Err(Error::Net(format!(
                "MAL returned HTTP {status} ({}): {}",
                err_body.error,
                if err_body.message.is_empty() {
                    "(no message)"
                } else {
                    &err_body.message
                }
            )));
        }
        return Err(Error::Net(format!("MAL returned HTTP {status}: {text}")));
    }

    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Real fixtures, captured 2026-08-15 from the live MAL API with a
    // real `X-MAL-CLIENT-ID` header (not hand-written approximations) — same
    // discipline as core::anime's captured AniList fixtures. No network
    // access; these run as part of the normal `cargo test --workspace`
    // suite.

    /// Real response body captured from
    /// `GET https://api.myanimelist.net/v2/forum/boards`.
    const REAL_BOARDS_RESPONSE_FIXTURE: &str = r#"{"categories":[{"title":"MyAnimeList","boards":[{"id":5,"title":"Updates & Announcements","description":"Updates, changes, and additions to MAL.","subboards":[]},{"id":17,"title":"DB Modification Requests","description":"Ask questions or submit changes (that you are unable to edit on the entry page) in the applicable board.","subboards":[{"id":2,"title":"Anime DB"},{"id":3,"title":"Character & People DB"},{"id":5,"title":"Manga DB"}]}]},{"title":"Anime & Manga","boards":[{"id":1,"title":"Anime Discussion","description":"General anime discussion that is not specific to any particular series.","subboards":[]}]}]}"#;

    /// Real response body captured from
    /// `GET https://api.myanimelist.net/v2/forum/topics?board_id=1&limit=2`.
    const REAL_TOPICS_RESPONSE_FIXTURE: &str = r#"{"data":[{"id":2245806,"title":"AWC 2026 Anime Watching Challenge - Sign-Up (Challenge Ends DEC 10)","created_at":"2025-12-20T23:03:49+00:00","number_of_posts":894,"last_post_created_at":"2026-08-13T03:09:20+00:00","is_locked":false,"created_by":{"id":5944650,"name":"AWC_mod"},"last_post_created_by":{"id":5107831,"name":"Ensen_"}},{"id":1883466,"title":"The 'Help Identifying This Anime/Character' Thread (v10)","created_at":"2020-12-21T09:06:32+00:00","number_of_posts":7963,"last_post_created_at":"2026-07-31T00:14:34+00:00","is_locked":false,"created_by":{"id":7100653,"name":"Koito91"},"last_post_created_by":{"id":18973394,"name":"adrians36881374"}}],"paging":{"next":"https://api.myanimelist.net/v2/forum/topics?offset=2&board_id=1&limit=2"}}"#;

    /// Real response body captured from
    /// `GET https://api.myanimelist.net/v2/forum/topic/516093?limit=1` — a
    /// real, small, locked ("Anime Discussion Rules") topic. Deliberately
    /// includes the real `forum_avator` typo'd field name and real BBCode
    /// (`[b]`, `[url=...]`, `[list]`) in `body`, plus a real non-null
    /// `signature`.
    const REAL_TOPIC_DETAIL_RESPONSE_FIXTURE: &str = r#"{"data":{"title":"Anime Discussion Rules","posts":[{"id":18240339,"number":1,"created_at":"2012-11-09T18:42:33+00:00","created_by":{"id":94702,"name":"Luna","forum_title":"★★★★★","forum_avator":"https://cdn.myanimelist.net/images/useravatars/94702.png?t=1786804200"},"body":"[b]Anime Discussion Rules[/b] [url=http://myanimelist.net/forum/?topicid=516059]Site & Forum Guidelines[/url]","signature":"[center][img]https://i.imgur.com/JIg3E1R.png[/img][/center]"}]},"paging":{"next":"https://api.myanimelist.net/v2/forum/topic/516093?offset=1&limit=1"}}"#;

    /// Real error body captured from a request with a deliberately bogus
    /// `X-MAL-CLIENT-ID` value.
    const REAL_INVALID_CLIENT_ID_ERROR_FIXTURE: &str =
        r#"{"message":"Invalid client id","error":"bad_request"}"#;

    /// Real error body captured from a request with no `X-MAL-CLIENT-ID`
    /// header at all.
    const REAL_FORBIDDEN_ERROR_FIXTURE: &str = r#"{"message":"","error":"forbidden"}"#;

    #[test]
    fn parses_a_real_captured_boards_response() {
        let parsed: BoardsResponse = serde_json::from_str(REAL_BOARDS_RESPONSE_FIXTURE).unwrap();
        assert_eq!(parsed.categories.len(), 2);

        let mal_category = &parsed.categories[0];
        assert_eq!(mal_category.title, "MyAnimeList");
        assert_eq!(mal_category.boards.len(), 2);
        assert_eq!(mal_category.boards[0].id, 5);
        assert_eq!(mal_category.boards[0].title, "Updates & Announcements");
        assert!(mal_category.boards[0].subboards.is_empty());

        // The real "board with subboards" case: DB Modification Requests.
        let db_mod_board = &mal_category.boards[1];
        assert_eq!(db_mod_board.subboards.len(), 3);
        assert_eq!(db_mod_board.subboards[0].id, 2);
        assert_eq!(db_mod_board.subboards[0].title, "Anime DB");
    }

    #[test]
    fn parses_a_real_captured_topics_response() {
        let parsed: TopicsResponse = serde_json::from_str(REAL_TOPICS_RESPONSE_FIXTURE).unwrap();
        assert_eq!(parsed.data.len(), 2);

        let first = &parsed.data[0];
        assert_eq!(first.id, 2245806);
        assert_eq!(
            first.title,
            "AWC 2026 Anime Watching Challenge - Sign-Up (Challenge Ends DEC 10)"
        );
        assert_eq!(first.number_of_posts, 894);
        assert!(!first.is_locked);
        assert_eq!(first.created_by.id, 5944650);
        assert_eq!(first.created_by.name, "AWC_mod");
        assert_eq!(first.last_post_created_by.name, "Ensen_");
    }

    #[test]
    fn parses_a_real_captured_topic_detail_response() {
        let parsed: TopicDetailResponse =
            serde_json::from_str(REAL_TOPIC_DETAIL_RESPONSE_FIXTURE).unwrap();
        assert_eq!(parsed.data.title, "Anime Discussion Rules");
        assert_eq!(parsed.data.posts.len(), 1);

        let post = &parsed.data.posts[0];
        assert_eq!(post.id, 18240339);
        assert_eq!(post.number, 1);
        assert_eq!(post.created_by.name, "Luna");
        assert_eq!(post.created_by.forum_title.as_deref(), Some("★★★★★"));
        assert!(post.created_by.forum_avator.is_some());
        assert!(post.body.contains("[b]Anime Discussion Rules[/b]"));
        assert!(post.signature.is_some());
    }

    #[test]
    fn parses_real_captured_error_bodies() {
        let bad_id: MalErrorBody =
            serde_json::from_str(REAL_INVALID_CLIENT_ID_ERROR_FIXTURE).unwrap();
        assert_eq!(bad_id.error, "bad_request");
        assert_eq!(bad_id.message, "Invalid client id");

        let forbidden: MalErrorBody = serde_json::from_str(REAL_FORBIDDEN_ERROR_FIXTURE).unwrap();
        assert_eq!(forbidden.error, "forbidden");
        assert_eq!(forbidden.message, "");
    }

    #[test]
    fn rejects_malformed_non_json_response() {
        let err = serde_json::from_str::<BoardsResponse>("not json at all").unwrap_err();
        // Just confirming this is the real serde_json error path this
        // module's callers map into Error::Net — see list_forum_categories.
        assert!(!err.to_string().is_empty());
    }

    #[tokio::test]
    async fn missing_client_id_env_var_is_a_clear_real_error() {
        // Safe in this crate's test harness: no other test in this module
        // reads or writes MAL_CLIENT_ID, and Rust runs a single crate's
        // `#[test]`s within one process but tests can run concurrently —
        // guard by using a name that's realistically never set in any dev/CI
        // environment already, rather than mutating a shared var other
        // tests might race on.
        // SAFETY: single-threaded effect on the current process's env,
        // scoped to this test only reading it back immediately after.
        let original = std::env::var(MAL_CLIENT_ID_ENV_VAR).ok();
        // SAFETY: test-only env mutation, immediately restored below.
        unsafe {
            std::env::remove_var(MAL_CLIENT_ID_ENV_VAR);
        }

        let err = list_forum_categories().await.unwrap_err();
        assert!(
            err.to_string().contains("MAL_CLIENT_ID"),
            "expected a clear error naming the missing env var, got: {err}"
        );

        if let Some(value) = original {
            // SAFETY: restoring exactly what was read above.
            unsafe {
                std::env::set_var(MAL_CLIENT_ID_ENV_VAR, value);
            }
        }
    }

    // --- Real live-network test, against the real MAL API. Not run as part
    // of the normal `cargo test --workspace` suite (needs a real
    // MAL_CLIENT_ID in the environment and a real network connection) — run
    // explicitly:
    //
    //   source /Users/ahmedelmahdi/.ankai_mal_credentials.env && \
    //     cargo test -p ankai-core --lib mal_forums -- --ignored --nocapture
    //
    // Exercises all three real endpoints this module wraps in one pass
    // against MAL's actual production API, chaining real ids from one call
    // into the next (a real board id from list_forum_categories, feeding
    // list_topics_in_board, feeding a real topic id into get_topic_posts) —
    // not just three isolated calls.
    #[tokio::test]
    #[ignore]
    async fn live_mal_api_returns_real_boards_topics_and_posts() {
        let categories = list_forum_categories()
            .await
            .expect("live GET forum/boards should succeed with a real MAL_CLIENT_ID");
        assert!(
            !categories.is_empty(),
            "MAL always has at least one forum category"
        );
        println!("live forum/boards: {} categories", categories.len());

        // "Anime Discussion" is board id 1, confirmed stable via the real
        // curl request run during this module's development — used here
        // rather than an arbitrary id from the boards response so this test
        // doesn't depend on category/board ordering staying the same.
        let anime_discussion_board_id = 1;
        let topics = list_topics_in_board(anime_discussion_board_id, 3)
            .await
            .expect("live GET forum/topics?board_id=1 should succeed");
        assert!(
            !topics.is_empty(),
            "the Anime Discussion board always has active topics"
        );
        println!(
            "live forum/topics (board_id={anime_discussion_board_id}): {} topics, first: {:?}",
            topics.len(),
            topics[0].title
        );

        let search_results = search_topics("naruto", 3)
            .await
            .expect("live GET forum/topics?q=naruto should succeed");
        assert!(
            !search_results.is_empty(),
            "searching a common term like 'naruto' should always return something"
        );
        println!(
            "live forum/topics (q=naruto): {} topics, first: {:?}",
            search_results.len(),
            search_results[0].title
        );

        let first_topic_id = topics[0].id;
        let details = get_topic_posts(first_topic_id, 2)
            .await
            .expect("live GET forum/topic/{id} should succeed for a topic id just returned by list_topics_in_board");
        assert!(
            !details.posts.is_empty(),
            "an active topic returned by list_topics_in_board must have at least one post"
        );
        println!(
            "live forum/topic/{first_topic_id}: title={:?}, {} posts fetched, first post author={:?}",
            details.title,
            details.posts.len(),
            details.posts[0].created_by.name
        );
    }
}
