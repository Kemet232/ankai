//! Manual, ignored-by-default end-to-end check that `core::lastfm`'s
//! Last.fm client genuinely works against the real, live
//! `https://ws.audioscrobbler.com/2.0/` API right now — not just that the
//! request-building/response-parsing code compiles and passes fixture-based
//! unit tests (see `core/src/lastfm.rs`'s own `#[cfg(test)]` module for
//! those).
//!
//! Requires a real `LASTFM_API_KEY` in the environment (see
//! `~/.ankai_lastfm_credentials.env`, sourced by hand before running this —
//! never hardcoded here, never committed).
//!
//! `#[ignore]`d by default so `cargo test --workspace`/CI doesn't depend on
//! outbound internet access or a locally-configured API key. Run explicitly
//! with:
//!
//! ```text
//! source ~/.ankai_lastfm_credentials.env
//! cargo test -p ankai-core --test lastfm_live -- --ignored --nocapture
//! ```

use ankai_core::lastfm::current_now_playing;

/// Real public, well-known Last.fm username (Last.fm's own co-founder's
/// account) — used the same way `anilist_live.rs` picks a real public
/// AniList query rather than anything fabricated. This account is not
/// expected to be playing anything at test-run time (no persistent "now
/// playing" account was provisioned for this project — see
/// `core::lastfm`'s module doc comment), so this test only asserts the real
/// request/response path works end-to-end, not any specific now-playing
/// state.
const REAL_PUBLIC_USERNAME: &str = "rj";

#[tokio::test]
#[ignore = "hits the real live Last.fm API over the network; run explicitly with --ignored"]
async fn real_lastfm_api_returns_a_real_response() {
    let result = current_now_playing(REAL_PUBLIC_USERNAME)
        .await
        .expect("current_now_playing should succeed against the real live Last.fm API");

    match &result {
        Some(now_playing) => {
            println!(
                "real now-playing result for {REAL_PUBLIC_USERNAME}: {} — {} (album: {:?}, art: {:?})",
                now_playing.artist, now_playing.track, now_playing.album, now_playing.album_art_url
            );
        }
        None => {
            println!(
                "real result for {REAL_PUBLIC_USERNAME}: nothing playing right now (a real, \
                 honest empty result, not a failure)"
            );
        }
    }

    // Also confirm the real "unknown username" failure path works against
    // the live API, not just the fixture-based unit test.
    let bad_username_result =
        current_now_playing("this_username_should_not_exist_ankai_probe_2026").await;
    assert!(
        bad_username_result.is_err(),
        "a nonexistent Last.fm username should be a real Err, got {bad_username_result:?}"
    );
    let err_text = bad_username_result.unwrap_err().to_string();
    println!("real error for a nonexistent username: {err_text}");
    assert!(
        err_text.contains("404") || err_text.to_lowercase().contains("not found"),
        "expected a real 'user not found' style error, got: {err_text}"
    );
}
