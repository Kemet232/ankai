//! Manual, ignored-by-default end-to-end check that `core::anime`'s AniList
//! client genuinely works against the real, live
//! `https://graphql.anilist.co` API right now — not just that the request-
//! building/response-parsing code compiles and passes fixture-based unit
//! tests (see `core/src/anime.rs`'s own `#[cfg(test)]` module for those).
//!
//! `#[ignore]`d by default so `cargo test --workspace`/CI doesn't depend on
//! outbound internet access or spend AniList's real (and, per
//! `core::anime`'s module doc comment, observed-to-be-fairly-tight) rate
//! limit on every ordinary test run. Run explicitly with:
//!
//! ```text
//! cargo test -p ankai-core --test anilist_live -- --ignored --nocapture
//! ```

use ankai_core::anime::{popular_anime, search_anime, trending_anime};

/// Runs all three real-network calls in one test (rather than three
/// separate `#[ignore]`d tests) so a single `--ignored` run only spends
/// three real requests against AniList's observed ~30-requests/minute
/// ceiling, not three independent test-binary invocations that could
/// overlap with other traffic.
#[tokio::test]
#[ignore = "hits the real live AniList API over the network; run explicitly with --ignored"]
async fn real_anilist_api_returns_real_looking_data() {
    let trending = trending_anime(5)
        .await
        .expect("trending_anime should succeed against the real AniList API");
    assert!(
        !trending.is_empty(),
        "trending_anime returned no results from the real API"
    );
    for entry in &trending {
        assert!(
            entry.title_romaji.is_some() || entry.title_english.is_some(),
            "trending entry {} had no title at all: {entry:?}",
            entry.id
        );
    }
    println!("trending_anime(5) real results:");
    for entry in &trending {
        println!(
            "  id={} romaji={:?} english={:?} episodes={:?} score={:?}",
            entry.id, entry.title_romaji, entry.title_english, entry.episodes, entry.average_score
        );
    }

    let popular = popular_anime(5)
        .await
        .expect("popular_anime should succeed against the real AniList API");
    assert!(
        !popular.is_empty(),
        "popular_anime returned no results from the real API"
    );
    println!("popular_anime(5) real results:");
    for entry in &popular {
        println!(
            "  id={} romaji={:?} english={:?} episodes={:?} score={:?}",
            entry.id, entry.title_romaji, entry.title_english, entry.episodes, entry.average_score
        );
    }

    let search_results = search_anime("Naruto")
        .await
        .expect("search_anime should succeed against the real AniList API");
    assert!(
        !search_results.is_empty(),
        "search_anime(\"Naruto\") returned no results from the real API"
    );
    let found_the_real_naruto = search_results.iter().any(|entry| entry.id == 20);
    assert!(
        found_the_real_naruto,
        "search_anime(\"Naruto\") should surface AniList id 20 (the real 'NARUTO' entry) \
         among its results, got: {search_results:?}"
    );
    println!("search_anime(\"Naruto\") real results:");
    for entry in &search_results {
        println!(
            "  id={} romaji={:?} english={:?} episodes={:?} score={:?}",
            entry.id, entry.title_romaji, entry.title_english, entry.episodes, entry.average_score
        );
    }
}
