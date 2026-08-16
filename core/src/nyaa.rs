//! Read-only anime release search using Nyaa's first-party RSS endpoint.
//!
//! Nyaa does not publish an API for comments. Its RSS feed does expose the
//! number of comments on each individual release, plus that release's
//! `https://nyaa.si/view/{id}` page. ANKAI therefore surfaces only a
//! per-release `comments_page_url`; it does not scrape comment bodies or imply
//! that Nyaa provides one title-level discussion thread.
//!
//! Search results can contain user-supplied torrent names. Callers should
//! continue to treat titles as untrusted display text. URLs are accepted only
//! when they use HTTPS, have the exact `nyaa.si` host, and match Nyaa's known
//! release or torrent-download path shapes.

use std::time::Duration;

use roxmltree::{Document, Node};

use crate::error::Error;

/// Nyaa's public site and RSS origin.
pub const NYAA_BASE_URL: &str = "https://nyaa.si/";

const MAX_QUERY_CHARS: usize = 120;
const MAX_QUERY_BYTES: usize = 512;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_RESULTS: usize = 75;
const MAX_TITLE_CHARS: usize = 600;

/// Anime subcategories understood by Nyaa's RSS search.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AnimeCategory {
    /// All anime releases (`1_0`).
    #[default]
    All,
    /// Anime music videos (`1_1`).
    MusicVideo,
    /// English-translated anime (`1_2`).
    EnglishTranslated,
    /// Non-English-translated anime (`1_3`).
    NonEnglishTranslated,
    /// Raw anime (`1_4`).
    Raw,
}

impl AnimeCategory {
    fn query_value(self) -> &'static str {
        match self {
            Self::All => "1_0",
            Self::MusicVideo => "1_1",
            Self::EnglishTranslated => "1_2",
            Self::NonEnglishTranslated => "1_3",
            Self::Raw => "1_4",
        }
    }
}

/// One release returned by Nyaa's RSS feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub release_id: u64,
    pub title: String,
    /// The release page also acts as the only honest comments-page link.
    pub view_url: String,
    /// Alias of [`Self::view_url`] named for its UI purpose.
    pub comments_page_url: String,
    /// Direct `.torrent` download URL, when supplied by the feed.
    pub torrent_url: Option<String>,
    /// Lower-case, 40-character BitTorrent v1 info hash, when supplied.
    pub info_hash: Option<String>,
    pub comments: u64,
    pub trusted: bool,
    pub remake: bool,
    pub seeders: u64,
    pub leechers: u64,
    pub downloads: u64,
    pub size: String,
    /// RFC 2822 text as supplied by RSS.
    pub published_at: String,
    pub category_id: String,
    pub category: String,
}

/// Reusable, connection-pooling client for Nyaa's public RSS search.
#[derive(Clone, Debug)]
pub struct NyaaClient {
    http: reqwest::Client,
}

impl NyaaClient {
    pub fn new() -> Result<Self, Error> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("ANKAI/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|error| Error::Nyaa(format!("failed to construct HTTP client: {error}")))?;
        Ok(Self { http })
    }

    /// Searches all Nyaa anime subcategories.
    pub async fn search_anime(&self, query: &str) -> Result<Vec<Release>, Error> {
        self.search_anime_category(query, AnimeCategory::All).await
    }

    /// Searches one explicit Nyaa anime category using only its RSS endpoint.
    pub async fn search_anime_category(
        &self,
        query: &str,
        category: AnimeCategory,
    ) -> Result<Vec<Release>, Error> {
        let url = search_url(query, category)?;
        let mut response = self.http.get(url).send().await.map_err(|error| {
            Error::Nyaa(format!("anime release search request failed: {error}"))
        })?;
        let status = response.status();

        if let Some(length) = response.content_length() {
            if length > MAX_RESPONSE_BYTES as u64 {
                return Err(Error::Nyaa(format!(
                    "RSS response is larger than {MAX_RESPONSE_BYTES} bytes"
                )));
            }
        }

        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| Error::Nyaa(format!("failed to read RSS response: {error}")))?
        {
            if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(Error::Nyaa(format!(
                    "RSS response is larger than {MAX_RESPONSE_BYTES} bytes"
                )));
            }
            body.extend_from_slice(&chunk);
        }

        if !status.is_success() {
            return Err(Error::Nyaa(format!(
                "anime release search returned HTTP {status}"
            )));
        }
        let xml = std::str::from_utf8(&body)
            .map_err(|error| Error::Nyaa(format!("RSS response is not valid UTF-8: {error}")))?;
        parse_feed(xml)
    }
}

impl Default for NyaaClient {
    fn default() -> Self {
        Self::new().expect("the bundled Nyaa HTTP client configuration is valid")
    }
}

/// Convenience wrapper for callers that do not need to retain a client.
pub async fn search_anime(query: &str) -> Result<Vec<Release>, Error> {
    NyaaClient::new()?.search_anime(query).await
}

fn search_url(query: &str, category: AnimeCategory) -> Result<reqwest::Url, Error> {
    let query = validate_query(query)?;
    let mut url =
        reqwest::Url::parse(NYAA_BASE_URL).expect("the bundled Nyaa URL is a valid absolute URL");
    url.query_pairs_mut()
        .append_pair("page", "rss")
        .append_pair("q", query)
        .append_pair("c", category.query_value())
        .append_pair("f", "0");
    Ok(url)
}

fn validate_query(query: &str) -> Result<&str, Error> {
    let query = query.trim();
    if query.is_empty() {
        return Err(Error::Nyaa("search query cannot be empty".into()));
    }
    if query.len() > MAX_QUERY_BYTES || query.chars().count() > MAX_QUERY_CHARS {
        return Err(Error::Nyaa(format!(
            "search query must be at most {MAX_QUERY_CHARS} characters and {MAX_QUERY_BYTES} bytes"
        )));
    }
    if query.chars().any(char::is_control) {
        return Err(Error::Nyaa(
            "search query cannot contain control characters".into(),
        ));
    }
    Ok(query)
}

fn parse_feed(xml: &str) -> Result<Vec<Release>, Error> {
    if xml.len() > MAX_RESPONSE_BYTES {
        return Err(Error::Nyaa(format!(
            "RSS response is larger than {MAX_RESPONSE_BYTES} bytes"
        )));
    }
    let document =
        Document::parse(xml).map_err(|error| Error::Nyaa(format!("invalid RSS XML: {error}")))?;
    document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "item")
        .take(MAX_RESULTS)
        .enumerate()
        .map(|(index, item)| parse_item(item, index))
        .collect()
}

fn parse_item(item: Node<'_, '_>, index: usize) -> Result<Release, Error> {
    let item_number = index + 1;
    let title = required_text(item, "title", item_number)?;
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(Error::Nyaa(format!(
            "RSS item {item_number} title is longer than {MAX_TITLE_CHARS} characters"
        )));
    }

    let view_url = required_text(item, "guid", item_number)?;
    let release_id = validate_release_url(&view_url, item_number)?;

    let torrent_url = child_text(item, "link")
        .map(|url| validate_torrent_url(&url, release_id, item_number).map(|()| url))
        .transpose()?;
    let info_hash = child_text(item, "infoHash")
        .map(|hash| validate_info_hash(&hash, item_number))
        .transpose()?;

    let category_id = required_text(item, "categoryId", item_number)?;
    if !matches!(category_id.as_str(), "1_0" | "1_1" | "1_2" | "1_3" | "1_4") {
        return Err(Error::Nyaa(format!(
            "RSS item {item_number} is not in an anime category"
        )));
    }

    Ok(Release {
        release_id,
        title,
        comments_page_url: view_url.clone(),
        view_url,
        torrent_url,
        info_hash,
        comments: required_u64(item, "comments", item_number)?,
        trusted: required_yes_no(item, "trusted", item_number)?,
        remake: required_yes_no(item, "remake", item_number)?,
        seeders: required_u64(item, "seeders", item_number)?,
        leechers: required_u64(item, "leechers", item_number)?,
        downloads: required_u64(item, "downloads", item_number)?,
        size: required_text(item, "size", item_number)?,
        published_at: required_text(item, "pubDate", item_number)?,
        category_id,
        category: required_text(item, "category", item_number)?,
    })
}

fn required_text(node: Node<'_, '_>, name: &str, item_number: usize) -> Result<String, Error> {
    child_text(node, name)
        .ok_or_else(|| Error::Nyaa(format!("RSS item {item_number} is missing required {name}")))
}

fn child_text(node: Node<'_, '_>, local_name: &str) -> Option<String> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == local_name)
        .and_then(|child| child.text())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn required_u64(node: Node<'_, '_>, name: &str, item_number: usize) -> Result<u64, Error> {
    let value = required_text(node, name, item_number)?;
    value.parse().map_err(|error| {
        Error::Nyaa(format!(
            "RSS item {item_number} has invalid {name} {value:?}: {error}"
        ))
    })
}

fn required_yes_no(node: Node<'_, '_>, name: &str, item_number: usize) -> Result<bool, Error> {
    let value = required_text(node, name, item_number)?;
    match value.as_str() {
        "Yes" => Ok(true),
        "No" => Ok(false),
        _ => Err(Error::Nyaa(format!(
            "RSS item {item_number} has invalid {name} flag {value:?}"
        ))),
    }
}

fn validate_release_url(candidate: &str, item_number: usize) -> Result<u64, Error> {
    let url = validate_nyaa_https_url(candidate, "release", item_number)?;
    let release_id = url
        .path()
        .strip_prefix("/view/")
        .filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or_else(|| {
            Error::Nyaa(format!(
                "RSS item {item_number} has an invalid Nyaa release path"
            ))
        })?;
    release_id.parse().map_err(|error| {
        Error::Nyaa(format!(
            "RSS item {item_number} has an invalid release id: {error}"
        ))
    })
}

fn validate_torrent_url(candidate: &str, release_id: u64, item_number: usize) -> Result<(), Error> {
    let url = validate_nyaa_https_url(candidate, "torrent", item_number)?;
    let expected = format!("/download/{release_id}.torrent");
    if url.path() != expected {
        return Err(Error::Nyaa(format!(
            "RSS item {item_number} torrent URL does not match its release id"
        )));
    }
    Ok(())
}

fn validate_nyaa_https_url(
    candidate: &str,
    kind: &str,
    item_number: usize,
) -> Result<reqwest::Url, Error> {
    let url = reqwest::Url::parse(candidate).map_err(|error| {
        Error::Nyaa(format!(
            "RSS item {item_number} has an invalid {kind} URL: {error}"
        ))
    })?;
    if url.scheme() != "https"
        || url.host_str() != Some("nyaa.si")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::Nyaa(format!(
            "RSS item {item_number} {kind} URL must be an unqualified HTTPS nyaa.si URL"
        )));
    }
    Ok(url)
}

fn validate_info_hash(hash: &str, item_number: usize) -> Result<String, Error> {
    if hash.len() != 40 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::Nyaa(format!(
            "RSS item {item_number} has an invalid BitTorrent v1 info hash"
        )));
    }
    Ok(hash.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss xmlns:nyaa="https://nyaa.si/xmlns/nyaa" version="2.0">
  <channel>
    <title>Nyaa anime search</title>
    <item>
      <title>[SubsPlease] Frieren - 28 (1080p)</title>
      <link>https://nyaa.si/download/1234567.torrent</link>
      <guid isPermaLink="true">https://nyaa.si/view/1234567</guid>
      <pubDate>Fri, 15 Aug 2026 19:00:00 -0000</pubDate>
      <nyaa:seeders>412</nyaa:seeders>
      <nyaa:leechers>7</nyaa:leechers>
      <nyaa:downloads>2048</nyaa:downloads>
      <nyaa:infoHash>0123456789ABCDEF0123456789ABCDEF01234567</nyaa:infoHash>
      <nyaa:categoryId>1_2</nyaa:categoryId>
      <nyaa:category>Anime - English-translated</nyaa:category>
      <nyaa:size>1.4 GiB</nyaa:size>
      <nyaa:comments>23</nyaa:comments>
      <nyaa:trusted>Yes</nyaa:trusted>
      <nyaa:remake>No</nyaa:remake>
    </item>
  </channel>
</rss>"#;

    #[test]
    fn parses_release_metadata_and_release_level_comments_link() {
        let releases = parse_feed(RSS).unwrap();
        assert_eq!(releases.len(), 1);
        let release = &releases[0];
        assert_eq!(release.release_id, 1_234_567);
        assert_eq!(release.title, "[SubsPlease] Frieren - 28 (1080p)");
        assert_eq!(release.view_url, "https://nyaa.si/view/1234567");
        assert_eq!(release.comments_page_url, release.view_url);
        assert_eq!(
            release.torrent_url.as_deref(),
            Some("https://nyaa.si/download/1234567.torrent")
        );
        assert_eq!(
            release.info_hash.as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        assert_eq!(release.comments, 23);
        assert!(release.trusted);
        assert!(!release.remake);
        assert_eq!(release.seeders, 412);
        assert_eq!(release.leechers, 7);
        assert_eq!(release.downloads, 2048);
        assert_eq!(release.size, "1.4 GiB");
        assert_eq!(release.category_id, "1_2");
    }

    #[test]
    fn accepts_missing_optional_torrent_and_info_hash() {
        let xml = RSS
            .replace(
                "      <link>https://nyaa.si/download/1234567.torrent</link>\n",
                "",
            )
            .replace(
                "      <nyaa:infoHash>0123456789ABCDEF0123456789ABCDEF01234567</nyaa:infoHash>\n",
                "",
            );
        let release = parse_feed(&xml).unwrap().remove(0);
        assert_eq!(release.torrent_url, None);
        assert_eq!(release.info_hash, None);
    }

    #[test]
    fn search_url_is_encoded_and_locked_to_anime() {
        let url = search_url("Frieren & Himmel", AnimeCategory::EnglishTranslated).unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("nyaa.si"));
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(pairs.get("page").map(|value| value.as_ref()), Some("rss"));
        assert_eq!(
            pairs.get("q").map(|value| value.as_ref()),
            Some("Frieren & Himmel")
        );
        assert_eq!(pairs.get("c").map(|value| value.as_ref()), Some("1_2"));
        assert_eq!(pairs.get("f").map(|value| value.as_ref()), Some("0"));
    }

    #[test]
    fn query_is_trimmed_and_bounded() {
        assert_eq!(validate_query("  Naruto  ").unwrap(), "Naruto");
        assert!(validate_query("\n\t").is_err());
        assert!(validate_query("Naruto\0Shippuden").is_err());
        assert!(validate_query(&"a".repeat(MAX_QUERY_CHARS + 1)).is_err());
        assert!(validate_query(&"界".repeat(MAX_QUERY_CHARS)).is_ok());
        assert!(validate_query(&"界".repeat(MAX_QUERY_CHARS + 1)).is_err());
    }

    #[test]
    fn rejects_non_nyaa_or_ambiguous_release_urls() {
        for unsafe_url in [
            "http://nyaa.si/view/1234567",
            "https://nyaa.si.evil.example/view/1234567",
            "https://nyaa.si@evil.example/view/1234567",
            "https://nyaa.si:444/view/1234567",
            "https://nyaa.si/view/1234567?next=https://evil.example",
            "https://nyaa.si/view/not-a-number",
        ] {
            let xml = RSS.replace("https://nyaa.si/view/1234567", unsafe_url);
            assert!(parse_feed(&xml).is_err(), "accepted {unsafe_url}");
        }
    }

    #[test]
    fn rejects_torrent_url_with_a_different_release_id() {
        let xml = RSS.replace(
            "https://nyaa.si/download/1234567.torrent",
            "https://nyaa.si/download/7654321.torrent",
        );
        assert!(parse_feed(&xml).is_err());
    }

    #[test]
    fn rejects_invalid_info_hash_and_non_anime_category() {
        let bad_hash = RSS.replace(
            "0123456789ABCDEF0123456789ABCDEF01234567",
            "../../not-a-hash",
        );
        assert!(parse_feed(&bad_hash).is_err());

        let non_anime = RSS.replace(
            "<nyaa:categoryId>1_2</nyaa:categoryId>",
            "<nyaa:categoryId>3_1</nyaa:categoryId>",
        );
        assert!(parse_feed(&non_anime).is_err());
    }

    #[test]
    fn rejects_malformed_numeric_and_boolean_metadata() {
        assert!(parse_feed(&RSS.replace(
            "<nyaa:seeders>412</nyaa:seeders>",
            "<nyaa:seeders>-1</nyaa:seeders>"
        ))
        .is_err());
        assert!(parse_feed(&RSS.replace(
            "<nyaa:trusted>Yes</nyaa:trusted>",
            "<nyaa:trusted>Maybe</nyaa:trusted>"
        ))
        .is_err());
    }

    #[test]
    fn caps_the_number_of_feed_items() {
        let item = RSS
            .split_once("<item>")
            .unwrap()
            .1
            .split_once("</item>")
            .unwrap()
            .0;
        let items = (0..(MAX_RESULTS + 10))
            .map(|_| format!("<item>{item}</item>"))
            .collect::<String>();
        let feed = format!(
            r#"<rss xmlns:nyaa="https://nyaa.si/xmlns/nyaa"><channel>{items}</channel></rss>"#
        );
        assert_eq!(parse_feed(&feed).unwrap().len(), MAX_RESULTS);
    }
}
