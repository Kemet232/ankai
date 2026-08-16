//! Last.fm "now playing" — a real, read-only client for Last.fm's public
//! `user.getRecentTracks` endpoint, used to show what a configured Last.fm
//! username is currently listening to.
//!
//! **Same category of dependency as `core::anime`'s AniList client and
//! ANKAI's other third-party metadata clients, not ANKAI's own
//! infrastructure.** This is ANKAI consuming a public read API someone else
//! runs, the same way those modules consume public third-party metadata APIs.
//! No ADR, no server code, nothing to accept here.
//!
//! **Why Last.fm and not Spotify/YouTube.** Recorded in `PROGRESS.md`
//! (session 14): Spotify's "currently playing" data requires a Premium
//! account and full OAuth (confirmed live, blocking for a general feature);
//! YouTube has no official "what a user is watching right now" API at all.
//! Last.fm was chosen instead — real, free, registered API account, works
//! for anyone who scrobbles (directly or via a Spotify/Apple
//! Music/`last.fm` bridge), no OAuth needed for this read-only endpoint.
//!
//! **Auth — confirmed empirically 2026-08-16, do not assume this is still
//! true in a future session without re-checking.** Real live `curl` requests
//! against `https://ws.audioscrobbler.com/2.0/` during development confirmed
//! `method=user.getrecenttracks` works with **only** an API key as a query
//! parameter — no OAuth2 session, no login, no shared-secret request
//! signing. (The shared secret in
//! `~/.ankai_lastfm_credentials.env` is provisioned for future
//! write/authenticated calls this module doesn't make; it is not read here.)
//! This is a *different* required input than the API key: the endpoint also
//! needs a Last.fm **username** (`user=...`) — whoever's "now playing" state
//! is being asked for — which is why this module's public function takes one
//! as a parameter rather than assuming a single account tied to the API key.
//!
//! **Real confirmed response shapes**, from real requests made during
//! development (see this module's own tests for the exact captured JSON):
//! - A track that is **not** currently playing (i.e. it's just the most
//!   recent scrobble in someone's history) has a `date` field (`{"uts":
//!   ..., "#text": ...}`) and no `@attr` field on the track object itself.
//! - A user with **zero** scrobble history ever returns a `track` array of
//!   length zero — no error, just nothing to show.
//! - An unknown/nonexistent username returns real HTTP `404` with body
//!   `{"message":"User not found","error":6}` — confirmed live.
//! - **The now-playing signal itself** (`"@attr": {"nowplaying": "true"}`
//!   present on the first track, and no `date` field on that same track,
//!   since a currently-playing track hasn't been scrobbled with a timestamp
//!   yet) could not be captured from a real live currently-playing account
//!   during this module's development: finding one required either
//!   scrobbling a track live under a specific test account (out of scope —
//!   no such account was provisioned) or enumerating real people's live
//!   listening activity to find one who happened to be playing something at
//!   that instant, which this session correctly declined to do as a privacy
//!   concern (checking a specific, deliberately-configured username is fine;
//!   trawling a friends graph to catch strangers mid-listen is not the same
//!   thing). Instead this shape is cross-referenced against `pylast`
//!   (<https://github.com/pylast/pylast>, a mature, widely-used third-party
//!   Last.fm client) real parsing source, which does exactly this: treats a
//!   present `nowplaying` attribute as the now-playing signal and expects no
//!   `date` element on that track — same "confirm against a real, working
//!   third-party client's source" methodology used by the other API clients.
//!   **This one detail (the now-playing shape itself, not the rest of this
//!   module) is doc-cross-referenced rather than live-captured — flagged
//!   honestly here, not silently presented as independently verified. A
//!   future session with a real scrobbling account handy should capture a
//!   live now-playing fixture and replace this note.**
//!
//! **What this module explicitly does not attempt:**
//! - No playback position/duration/progress. Last.fm's `user.getRecentTracks`
//!   does not return elapsed-time or track-length data for a now-playing
//!   entry — there is nothing here for a UI to build a real progress bar
//!   from. Any progress-looking UI element built on top of this module's
//!   data is necessarily decorative, not a real scrubber, and must not claim
//!   to show real elapsed/remaining time.
//! - No scrobbling, loving, or any other write to Last.fm. Every function
//!   here is a `GET`; the shared secret is not used.
//! - No OAuth2/session-key flow. Only the public, API-key-only read surface.
//! - No local persistence/caching table, matching the other live-feed clients'
//!   posture — every call here is a live, uncached read; callers that want
//!   to poll periodically own their own refresh cadence.
//! - No retry/backoff/queueing on failure, matching every other network
//!   module in this crate: a failed request is a real `Err` the caller sees
//!   immediately.

use serde::Deserialize;

use crate::error::Error;

/// Last.fm's API 2.0 base URL.
const LASTFM_API_BASE: &str = "https://ws.audioscrobbler.com/2.0/";

/// The environment variable this module reads a registered application's
/// Last.fm API key from, at call time (never compiled in, never hardcoded) —
/// same "opt-in runtime environment variable" posture as
/// `ANKAI_DIRECTORY_URL` elsewhere in this crate.
const LASTFM_API_KEY_ENV_VAR: &str = "LASTFM_API_KEY";

/// Reads the Last.fm API key from the environment at call time. Returns a
/// clear real error (not a panic, not a silent empty-string param) if it
/// isn't set.
fn api_key() -> Result<String, Error> {
    std::env::var(LASTFM_API_KEY_ENV_VAR).map_err(|_| {
        Error::Net(format!(
            "{LASTFM_API_KEY_ENV_VAR} environment variable is not set — a registered Last.fm \
             API key is required to read now-playing data. See core::lastfm's module doc \
             comment."
        ))
    })
}

/// What a caller of [`current_now_playing`] actually gets when a track is
/// genuinely playing right now: artist, track title, and — if Last.fm
/// provided one — an album art image URL. Deliberately has no
/// elapsed/duration/progress fields; see the module doc comment's "What this
/// module explicitly does not attempt."
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowPlaying {
    pub artist: String,
    pub track: String,
    /// Real album title, if Last.fm's response had a non-empty one (its own
    /// JSON always includes the `album` object, but with an empty `#text`
    /// string when it doesn't know one — confirmed live; this module treats
    /// that empty string as "no album," not as a real empty-titled album).
    pub album: Option<String>,
    /// The largest real album art URL Last.fm provided, if any (its `image`
    /// array's `"extralarge"` entry can itself be an empty string —
    /// confirmed live, same treatment as `album` above).
    pub album_art_url: Option<String>,
}

/// Fetches `username`'s current now-playing track from Last.fm's real,
/// public `user.getRecentTracks` endpoint.
///
/// - `Ok(Some(NowPlaying))` — something is genuinely playing right now
///   (Last.fm's own `nowplaying` flag was set on the most recent track).
/// - `Ok(None)` — a real, successful response, but nothing is playing right
///   now. This covers both "the most recent scrobble is history, not
///   current" and "this account has never scrobbled anything." Callers that
///   need to distinguish "not configured" (no username at all) from this
///   honest "nothing playing" state must do so themselves before calling —
///   this function has no way to tell the difference between an empty
///   username and one that just isn't listening to anything, so it always
///   requires a real, non-empty `username`.
/// - `Err(Error::Net(..))` — a real failure: network error, malformed
///   response, or Last.fm itself returning a non-2xx status (e.g. unknown
///   username, real HTTP `404` with body `{"message":"User not
///   found","error":6}` — confirmed live).
pub async fn current_now_playing(username: &str) -> Result<Option<NowPlaying>, Error> {
    let key = api_key()?;
    let client = reqwest::Client::new();
    let response = client
        .get(LASTFM_API_BASE)
        .query(&[
            ("method", "user.getrecenttracks"),
            ("user", username),
            ("api_key", key.as_str()),
            ("format", "json"),
            ("limit", "1"),
        ])
        .send()
        .await
        .map_err(|e| Error::Net(format!("Last.fm request failed: {e}")))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| Error::Net(format!("failed to read Last.fm response body: {e}")))?;

    if !status.is_success() {
        if let Ok(err_body) = serde_json::from_str::<LastfmErrorBody>(&text) {
            return Err(Error::Net(format!(
                "Last.fm returned HTTP {status} (error {}): {}",
                err_body.error, err_body.message
            )));
        }
        return Err(Error::Net(format!(
            "Last.fm returned HTTP {status}: {text}"
        )));
    }

    parse_now_playing(&text)
}

/// Parses a raw `user.getRecentTracks` JSON body into
/// `Ok(Some(NowPlaying))`/`Ok(None)`. Split out from [`current_now_playing`]
/// so it can be exercised directly against real captured fixtures in this
/// module's tests, without a live network call.
fn parse_now_playing(text: &str) -> Result<Option<NowPlaying>, Error> {
    let parsed: RecentTracksEnvelope = serde_json::from_str(text)
        .map_err(|e| Error::Net(format!("failed to parse Last.fm response: {e}")))?;

    let Some(first) = parsed.recenttracks.track.into_iter().next() else {
        // Real, successful response; this account has never scrobbled
        // anything. Honestly "nothing playing," not an error.
        return Ok(None);
    };

    let is_now_playing = first.attr.as_ref().and_then(|a| a.nowplaying.as_deref()) == Some("true");

    if !is_now_playing {
        return Ok(None);
    }

    let album = non_empty(first.album.text);
    let album_art_url = first
        .image
        .into_iter()
        .find(|img| img.size == "extralarge")
        .and_then(|img| non_empty(img.text));

    Ok(Some(NowPlaying {
        artist: first.artist.text,
        track: first.name,
        album,
        album_art_url,
    }))
}

/// Last.fm's real JSON habit of using an empty string rather than omitting a
/// field entirely for "no value" (confirmed live for `album`/`image`) —
/// normalizes that into a real `Option`.
fn non_empty(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

// --- Real response shapes, field names verbatim from live capture ---------

#[derive(Debug, Deserialize)]
struct RecentTracksEnvelope {
    recenttracks: RecentTracksBody,
}

#[derive(Debug, Deserialize)]
struct RecentTracksBody {
    track: Vec<RawTrack>,
}

#[derive(Debug, Deserialize)]
struct RawTrack {
    artist: RawText,
    name: String,
    album: RawText,
    #[serde(default)]
    image: Vec<RawImage>,
    #[serde(rename = "@attr")]
    attr: Option<RawTrackAttr>,
}

/// Last.fm nests `artist`/`album` display text under a real `#text` key
/// (alongside an `mbid` this module doesn't use) — confirmed live.
#[derive(Debug, Deserialize)]
struct RawText {
    #[serde(rename = "#text")]
    text: String,
}

#[derive(Debug, Deserialize)]
struct RawImage {
    size: String,
    #[serde(rename = "#text")]
    text: String,
}

/// Present only on the currently-playing track, per the module doc comment's
/// note on how this shape was confirmed.
#[derive(Debug, Deserialize)]
struct RawTrackAttr {
    nowplaying: Option<String>,
}

/// Real shape of Last.fm's own error responses, e.g. `{"message":"User not
/// found","error":6}` — confirmed live during development.
#[derive(Debug, Deserialize)]
struct LastfmErrorBody {
    message: String,
    error: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real response body captured live during development (2026-08-16)
    /// for a well-known public account's most recent scrobble — history, not
    /// a currently-playing track (no `@attr` on the track, a real `date`
    /// present). Used here as a fixture rather than re-fetched over the
    /// network on every ordinary test run.
    const REAL_HISTORY_ONLY_RESPONSE: &str = r##"{"recenttracks":{"track":[{"artist":{"mbid":"","#text":"Shaggy"},"streamable":"0","image":[{"size":"small","#text":"https://lastfm-img.freetls.fastly.net/i/u/34s/438632d5d209f458075adb800be67696.jpg"},{"size":"medium","#text":"https://lastfm-img.freetls.fastly.net/i/u/64s/438632d5d209f458075adb800be67696.jpg"},{"size":"large","#text":"https://lastfm-img.freetls.fastly.net/i/u/174s/438632d5d209f458075adb800be67696.jpg"},{"size":"extralarge","#text":"https://lastfm-img.freetls.fastly.net/i/u/300x300/438632d5d209f458075adb800be67696.jpg"}],"mbid":"d098301a-3e73-3079-af4a-c324c7943e45","album":{"mbid":"100aee98-d7e5-49e1-bd38-a73bd4400193","#text":"Hot Shot"},"name":"Angel","url":"https://www.last.fm/music/Shaggy/_/Angel","date":{"uts":"1784536469","#text":"20 Jul 2026, 08:34"}}],"@attr":{"user":"RJ","totalPages":"151481","page":"1","perPage":"1","total":"151481"}}}"##;

    /// A shape matching Last.fm's own documented/`pylast`-confirmed
    /// now-playing track (`@attr.nowplaying == "true"`, no `date`) — not
    /// itself a live capture, see the module doc comment's honest note on
    /// why.
    const NOWPLAYING_SHAPE: &str = r##"{"recenttracks":{"track":[{"artist":{"mbid":"","#text":"Deftones"},"streamable":"0","image":[{"size":"small","#text":""},{"size":"medium","#text":""},{"size":"large","#text":""},{"size":"extralarge","#text":"https://example.invalid/cover.jpg"}],"mbid":"","album":{"mbid":"","#text":"White Pony"},"name":"Change (In the House of Flies)","url":"https://www.last.fm/music/Deftones/_/Change","@attr":{"nowplaying":"true"}}],"@attr":{"user":"testuser","totalPages":"1","page":"1","perPage":"1","total":"1"}}}"##;

    /// A user with no scrobble history at all — real, empty `track` array.
    const EMPTY_HISTORY_RESPONSE: &str = r##"{"recenttracks":{"track":[],"@attr":{"user":"nobody","totalPages":"0","page":"1","perPage":"1","total":"0"}}}"##;

    #[test]
    fn history_only_track_is_not_now_playing() {
        let result = parse_now_playing(REAL_HISTORY_ONLY_RESPONSE).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn empty_history_is_not_now_playing() {
        let result = parse_now_playing(EMPTY_HISTORY_RESPONSE).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn nowplaying_attr_true_is_recognized() {
        let result = parse_now_playing(NOWPLAYING_SHAPE).unwrap();
        assert_eq!(
            result,
            Some(NowPlaying {
                artist: "Deftones".to_string(),
                track: "Change (In the House of Flies)".to_string(),
                album: Some("White Pony".to_string()),
                album_art_url: Some("https://example.invalid/cover.jpg".to_string()),
            })
        );
    }

    #[test]
    fn real_error_body_parses() {
        let real_error = r#"{"message":"User not found","error":6}"#;
        let parsed: LastfmErrorBody = serde_json::from_str(real_error).unwrap();
        assert_eq!(parsed.message, "User not found");
        assert_eq!(parsed.error, 6);
    }

    #[test]
    fn missing_api_key_is_a_real_error_not_a_panic() {
        // SAFETY: test-only env mutation, no concurrent access to this var
        // within this crate's test suite.
        unsafe {
            std::env::remove_var(LASTFM_API_KEY_ENV_VAR);
        }
        let err = api_key().unwrap_err();
        assert!(err.to_string().contains(LASTFM_API_KEY_ENV_VAR));
    }
}
