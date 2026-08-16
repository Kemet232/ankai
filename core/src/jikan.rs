//! Typed, read-only client for Jikan v4's public anime catalog.
//!
//! Jikan is an unauthenticated community API which reads public
//! MyAnimeList data; it is not an official MyAnimeList API and must not be
//! treated as an account or list-sync backend. This module deliberately
//! exposes only `GET` operations. It performs no MAL mutations, stores no
//! credentials, and needs no user installation or configuration.
//!
//! Responses are always fetched live. Jikan applies server-side rate limits,
//! so consumers should debounce search and avoid issuing one request per
//! keystroke. In particular, HTTP 429 is surfaced as a clear [`Error::Jikan`]
//! containing the server's `Retry-After` value when one is supplied; this
//! client does not silently retry or queue a request.

use serde::{Deserialize, Serialize};

use crate::error::Error;

/// The zero-configuration Jikan v4 endpoint bundled with ANKAI.
pub const JIKAN_API_BASE: &str = "https://api.jikan.moe/v4";

/// Jikan currently accepts at most 25 results per page.
pub const MAX_PAGE_SIZE: u8 = 25;

/// A reusable, connection-pooling client for Jikan's public read API.
#[derive(Debug, Clone)]
pub struct JikanClient {
    client: reqwest::Client,
    base_url: reqwest::Url,
}

impl Default for JikanClient {
    fn default() -> Self {
        Self::new()
    }
}

impl JikanClient {
    /// Creates a client for the bundled public endpoint. No API key or other
    /// runtime configuration is required.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: reqwest::Url::parse(JIKAN_API_BASE)
                .expect("the bundled Jikan API URL is a valid absolute URL"),
        }
    }

    /// Creates a client with an alternate API root.
    ///
    /// This is primarily useful for deterministic integration tests and
    /// compatible proxies. Normal application code should use [`Self::new`].
    pub fn with_base_url(base_url: &str) -> Result<Self, Error> {
        let mut base_url = reqwest::Url::parse(base_url)
            .map_err(|e| Error::Jikan(format!("invalid API URL: {e}")))?;
        if !matches!(base_url.scheme(), "http" | "https") {
            return Err(Error::Jikan("API URL must use http or https".into()));
        }
        base_url.set_query(None);
        base_url.set_fragment(None);
        if base_url.path().ends_with('/') {
            let path = base_url.path().trim_end_matches('/').to_owned();
            base_url.set_path(&path);
        }
        Ok(Self {
            client: reqwest::Client::new(),
            base_url,
        })
    }

    /// Searches anime by title. Safe-search is always enabled for this
    /// application-facing search endpoint.
    pub async fn search_anime(&self, query: &str, limit: u8) -> Result<AnimePage, Error> {
        let query = validate_query(query)?;
        validate_limit(limit)?;
        let url = self.endpoint(
            &["anime"],
            &[
                ("q", query.to_owned()),
                ("sfw", "true".to_owned()),
                ("limit", limit.to_string()),
            ],
        )?;
        self.get(url, "anime search").await
    }

    /// Returns Jikan's current all-anime ranking.
    pub async fn top_anime(&self, limit: u8) -> Result<AnimePage, Error> {
        validate_limit(limit)?;
        let url = self.endpoint(&["top", "anime"], &[("limit", limit.to_string())])?;
        self.get(url, "top anime").await
    }

    /// Returns anime in the currently-airing season.
    pub async fn seasonal_now(&self, limit: u8) -> Result<AnimePage, Error> {
        validate_limit(limit)?;
        let url = self.endpoint(&["seasons", "now"], &[("limit", limit.to_string())])?;
        self.get(url, "current season").await
    }

    /// Returns Jikan's full public metadata for one MAL anime id.
    pub async fn anime_full(&self, mal_id: u64) -> Result<Anime, Error> {
        validate_mal_id(mal_id)?;
        let id = mal_id.to_string();
        let response: DataResponse<Anime> = self
            .get(self.endpoint(&["anime", &id, "full"], &[])?, "anime detail")
            .await?;
        Ok(response.data)
    }

    /// Returns one page of episodes for a MAL anime id.
    pub async fn anime_episodes(&self, mal_id: u64, page: u32) -> Result<EpisodePage, Error> {
        validate_mal_id(mal_id)?;
        if page == 0 {
            return Err(Error::Jikan("episode page must be at least 1".into()));
        }
        let id = mal_id.to_string();
        let url = self.endpoint(&["anime", &id, "episodes"], &[("page", page.to_string())])?;
        self.get(url, "anime episodes").await
    }

    fn endpoint(&self, parts: &[&str], query: &[(&str, String)]) -> Result<reqwest::Url, Error> {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|_| Error::Jikan("API URL cannot be used as a base URL".into()))?
            .pop_if_empty()
            .extend(parts);
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(key, value)| (*key, value.as_str())));
        }
        Ok(url)
    }

    async fn get<T: for<'de> Deserialize<'de>>(
        &self,
        url: reqwest::Url,
        resource: &str,
    ) -> Result<T, Error> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| Error::Jikan(format!("{resource} request failed: {e}")))?;
        let status = response.status();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response
            .text()
            .await
            .map_err(|e| Error::Jikan(format!("failed to read {resource} response: {e}")))?;

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry = retry_after
                .as_deref()
                .map(|value| format!("; retry after {value}"))
                .unwrap_or_default();
            let message = response_message(&body);
            return Err(Error::Jikan(format!(
                "{resource} was rate limited (HTTP 429){retry}: {message}"
            )));
        }
        if !status.is_success() {
            return Err(Error::Jikan(format!(
                "{resource} returned HTTP {status}: {}",
                response_message(&body)
            )));
        }

        serde_json::from_str(&body)
            .map_err(|e| Error::Jikan(format!("failed to parse {resource} response: {e}")))
    }
}

fn validate_query(query: &str) -> Result<&str, Error> {
    let query = query.trim();
    if query.is_empty() {
        return Err(Error::Jikan("search query cannot be empty".into()));
    }
    Ok(query)
}

fn validate_limit(limit: u8) -> Result<(), Error> {
    if !(1..=MAX_PAGE_SIZE).contains(&limit) {
        return Err(Error::Jikan(format!(
            "result limit must be between 1 and {MAX_PAGE_SIZE}"
        )));
    }
    Ok(())
}

fn validate_mal_id(mal_id: u64) -> Result<(), Error> {
    if mal_id == 0 {
        return Err(Error::Jikan("MAL anime id must be greater than 0".into()));
    }
    Ok(())
}

fn response_message(body: &str) -> String {
    #[derive(Deserialize)]
    struct ApiError {
        message: Option<String>,
    }

    let message = serde_json::from_str::<ApiError>(body)
        .ok()
        .and_then(|error| error.message)
        .unwrap_or_else(|| body.trim().to_owned());
    let mut chars = message.chars();
    let shortened: String = chars.by_ref().take(500).collect();
    if chars.next().is_some() {
        format!("{shortened}…")
    } else if shortened.is_empty() {
        "empty response body".into()
    } else {
        shortened
    }
}

/// A Jikan list endpoint's response and pagination information.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page<T> {
    pub data: Vec<T>,
    #[serde(default)]
    pub pagination: Pagination,
}

pub type AnimePage = Page<Anime>;
pub type EpisodePage = Page<Episode>;

/// Pagination metadata returned by Jikan list endpoints.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pagination {
    #[serde(default)]
    pub last_visible_page: u32,
    #[serde(default)]
    pub has_next_page: bool,
    #[serde(default)]
    pub current_page: u32,
    #[serde(default)]
    pub items: PaginationItems,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaginationItems {
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub per_page: u32,
}

#[derive(Debug, Deserialize)]
struct DataResponse<T> {
    data: T,
}

/// Public anime metadata shared by search, ranking, season, and detail
/// responses. Fields which Jikan/MAL can legitimately omit remain optional.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Anime {
    pub mal_id: u64,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub images: AnimeImages,
    #[serde(default)]
    pub trailer: Trailer,
    #[serde(default)]
    pub titles: Vec<AnimeTitle>,
    pub title: String,
    #[serde(default)]
    pub title_english: Option<String>,
    #[serde(default)]
    pub title_japanese: Option<String>,
    #[serde(default)]
    pub title_synonyms: Vec<String>,
    #[serde(default, rename = "type")]
    pub anime_type: Option<String>,
    #[serde(default)]
    pub episodes: Option<u32>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub synopsis: Option<String>,
    #[serde(default)]
    pub score: Option<f32>,
    #[serde(default)]
    pub year: Option<u32>,
    #[serde(default)]
    pub season: Option<String>,
    #[serde(default)]
    pub genres: Vec<NamedResource>,
    #[serde(default)]
    pub studios: Vec<NamedResource>,
}

impl Anime {
    /// Picks the highest-quality poster Jikan provided, preferring WebP.
    pub fn large_poster_url(&self) -> Option<&str> {
        self.images
            .webp
            .large_image_url
            .as_deref()
            .or(self.images.jpg.large_image_url.as_deref())
            .or(self.images.webp.image_url.as_deref())
            .or(self.images.jpg.image_url.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimeTitle {
    #[serde(rename = "type")]
    pub title_type: String,
    pub title: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimeImages {
    #[serde(default)]
    pub jpg: ImageUrls,
    #[serde(default)]
    pub webp: ImageUrls,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageUrls {
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub small_image_url: Option<String>,
    #[serde(default)]
    pub large_image_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trailer {
    #[serde(default)]
    pub youtube_id: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub embed_url: Option<String>,
}

/// MAL resource reference embedded in anime metadata (genre, studio, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedResource {
    pub mal_id: u64,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub name: String,
    pub url: String,
}

/// One episode entry returned by `/anime/{id}/episodes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Episode {
    pub mal_id: u32,
    #[serde(default)]
    pub url: Option<String>,
    pub title: String,
    #[serde(default)]
    pub title_japanese: Option<String>,
    /// Jikan's wire field is genuinely spelled `title_romanji`.
    #[serde(default)]
    pub title_romanji: Option<String>,
    #[serde(default)]
    pub duration: Option<u32>,
    #[serde(default)]
    pub aired: Option<String>,
    #[serde(default)]
    pub filler: bool,
    #[serde(default)]
    pub recap: bool,
    #[serde(default)]
    pub forum_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANIME_PAGE: &str = r#"
    {
      "pagination": {
        "last_visible_page": 4,
        "has_next_page": true,
        "current_page": 1,
        "items": { "count": 1, "total": 87, "per_page": 25 }
      },
      "data": [{
        "mal_id": 20,
        "url": "https://myanimelist.net/anime/20/Naruto",
        "images": {
          "jpg": { "image_url": "https://img.example/naruto.jpg", "small_image_url": null, "large_image_url": "https://img.example/naruto-large.jpg" },
          "webp": { "image_url": "https://img.example/naruto.webp", "small_image_url": null, "large_image_url": "https://img.example/naruto-large.webp" }
        },
        "trailer": { "youtube_id": "abc", "url": "https://youtube.com/watch?v=abc", "embed_url": "https://youtube.com/embed/abc" },
        "titles": [{ "type": "Default", "title": "Naruto" }],
        "title": "Naruto",
        "title_english": "Naruto",
        "title_japanese": "NARUTO -ナルト-",
        "title_synonyms": ["NARUTO"],
        "type": "TV",
        "episodes": 220,
        "status": "Finished Airing",
        "synopsis": "A ninja story.",
        "score": 8.0,
        "year": 2002,
        "season": "fall",
        "genres": [{ "mal_id": 1, "type": "anime", "name": "Action", "url": "https://myanimelist.net/anime/genre/1/Action" }],
        "studios": [{ "mal_id": 1, "type": "anime", "name": "Pierrot", "url": "https://myanimelist.net/anime/producer/1/Pierrot" }]
      }]
    }
    "#;

    const EPISODE_PAGE: &str = r#"
    {
      "pagination": { "last_visible_page": 11, "has_next_page": true },
      "data": [{
        "mal_id": 1,
        "url": "https://myanimelist.net/anime/20/Naruto/episode/1",
        "title": "Enter: Naruto Uzumaki!",
        "title_japanese": "参上!うずまきナルト",
        "title_romanji": "Sanjou! Uzumaki Naruto",
        "duration": 1380,
        "aired": "2002-10-03T00:00:00+00:00",
        "filler": false,
        "recap": false,
        "forum_url": "https://myanimelist.net/forum/?topicid=1"
      }]
    }
    "#;

    #[test]
    fn parses_anime_images_metadata_and_pagination() {
        let page: AnimePage = serde_json::from_str(ANIME_PAGE).unwrap();
        assert_eq!(page.pagination.current_page, 1);
        assert_eq!(page.pagination.items.total, 87);
        assert_eq!(page.data[0].mal_id, 20);
        assert_eq!(page.data[0].anime_type.as_deref(), Some("TV"));
        assert_eq!(page.data[0].genres[0].name, "Action");
        assert_eq!(page.data[0].studios[0].name, "Pierrot");
        assert_eq!(
            page.data[0].large_poster_url(),
            Some("https://img.example/naruto-large.webp")
        );
    }

    #[test]
    fn parses_episode_page_with_sparse_pagination() {
        let page: EpisodePage = serde_json::from_str(EPISODE_PAGE).unwrap();
        assert_eq!(page.pagination.current_page, 0);
        assert_eq!(page.data[0].mal_id, 1);
        assert_eq!(page.data[0].duration, Some(1380));
        assert!(!page.data[0].filler);
    }

    #[test]
    fn constructs_percent_encoded_urls() {
        let client = JikanClient::with_base_url("https://example.test/v4/").unwrap();
        let url = client
            .endpoint(
                &["anime"],
                &[
                    ("q", "Frieren: Beyond Journey's End".to_owned()),
                    ("sfw", "true".to_owned()),
                    ("limit", "12".to_owned()),
                ],
            )
            .unwrap();
        assert_eq!(
            url.as_str(),
            "https://example.test/v4/anime?q=Frieren%3A+Beyond+Journey%27s+End&sfw=true&limit=12"
        );
        let episodes = client
            .endpoint(&["anime", "52991", "episodes"], &[("page", "2".into())])
            .unwrap();
        assert_eq!(
            episodes.as_str(),
            "https://example.test/v4/anime/52991/episodes?page=2"
        );
    }

    #[test]
    fn rejects_invalid_input_before_network_io() {
        assert!(validate_query("  ").is_err());
        assert!(validate_limit(0).is_err());
        assert!(validate_limit(MAX_PAGE_SIZE + 1).is_err());
        assert!(validate_mal_id(0).is_err());
        assert!(JikanClient::with_base_url("file:///tmp/jikan").is_err());
    }

    #[test]
    fn extracts_and_bounds_api_error_messages() {
        assert_eq!(
            response_message(r#"{"status": 404, "message": "Not Found"}"#),
            "Not Found"
        );
        assert_eq!(response_message(""), "empty response body");
        assert_eq!(response_message(&"x".repeat(501)).chars().count(), 501);
    }

    #[tokio::test]
    #[ignore = "calls the public Jikan service"]
    async fn live_search_smoke_test() {
        let page = JikanClient::new().search_anime("Naruto", 1).await.unwrap();
        assert!(!page.data.is_empty());
    }
}
