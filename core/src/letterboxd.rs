//! Read-only Letterboxd member activity via Letterboxd's public RSS feed.
//!
//! Letterboxd does not provide a generally available public API for reviews.
//! This module therefore uses only the supported public member feed at
//! `https://letterboxd.com/{username}/rss/`. It deliberately does not scrape
//! profile, film, or review HTML. RSS descriptions contain small HTML
//! fragments; those are reduced to a bounded plain-text excerpt and, when
//! present, an HTTPS poster URL.

use std::time::Duration;

use roxmltree::{Document, Node};

use crate::error::Error;

const LETTERBOXD_BASE: &str = "https://letterboxd.com";
const REVIEW_EXCERPT_CHARS: usize = 500;

/// One film activity item from a member's public RSS feed.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedEntry {
    pub film_title: String,
    pub film_year: Option<u16>,
    /// Letterboxd's zero-to-five member rating, including half-star values.
    pub member_rating: Option<f32>,
    /// A sanitized, whitespace-normalized excerpt; never HTML or Markdown.
    pub review_excerpt: Option<String>,
    pub link: String,
    /// Extracted only from an HTTPS `img src` in the RSS description.
    pub poster_url: Option<String>,
    /// RFC 2822 as supplied by RSS. Kept losslessly rather than imposing a
    /// date/time dependency on callers.
    pub published_at: Option<String>,
    pub guid: String,
}

/// Small reusable client for Letterboxd's public member RSS endpoint.
#[derive(Clone, Debug)]
pub struct LetterboxdClient {
    http: reqwest::Client,
}

impl LetterboxdClient {
    pub fn new() -> Result<Self, Error> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("ANKAI/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|error| {
                Error::Letterboxd(format!("failed to construct HTTP client: {error}"))
            })?;
        Ok(Self { http })
    }

    /// Fetches a member's latest public Letterboxd film activity.
    ///
    /// Usernames are validated before URL construction. Letterboxd usernames
    /// may contain ASCII letters, digits, `_`, and `-`; path separators,
    /// percent escapes, whitespace, and URL syntax are rejected.
    pub async fn member_feed(&self, username: &str) -> Result<Vec<FeedEntry>, Error> {
        validate_username(username)?;
        let url = format!("{LETTERBOXD_BASE}/{username}/rss/");
        let response = self
            .http
            .get(&url)
            .header("Accept", "application/rss+xml, application/xml;q=0.9")
            .send()
            .await
            .map_err(|error| {
                Error::Letterboxd(format!("request for member {username:?} failed: {error}"))
            })?;

        let status = response.status();
        let body = response.text().await.map_err(|error| {
            Error::Letterboxd(format!(
                "failed to read RSS response for member {username:?}: {error}"
            ))
        })?;
        if !status.is_success() {
            return Err(Error::Letterboxd(format!(
                "member feed for {username:?} returned HTTP {status}"
            )));
        }

        parse_member_feed(&body)
    }
}

/// Convenience wrapper for callers that do not need to retain a client.
pub async fn member_feed(username: &str) -> Result<Vec<FeedEntry>, Error> {
    LetterboxdClient::new()?.member_feed(username).await
}

fn validate_username(username: &str) -> Result<(), Error> {
    const MAX_USERNAME_LEN: usize = 40;
    if username.is_empty() {
        return Err(Error::Letterboxd("username cannot be empty".into()));
    }
    if username.len() > MAX_USERNAME_LEN {
        return Err(Error::Letterboxd(format!(
            "username is longer than {MAX_USERNAME_LEN} characters"
        )));
    }
    if !username
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(Error::Letterboxd(
            "username may contain only ASCII letters, digits, '_' and '-'".into(),
        ));
    }
    Ok(())
}

fn parse_member_feed(xml: &str) -> Result<Vec<FeedEntry>, Error> {
    let document = Document::parse(xml)
        .map_err(|error| Error::Letterboxd(format!("invalid RSS XML: {error}")))?;
    let items: Vec<_> = document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "item")
        .collect();

    let mut entries = Vec::with_capacity(items.len());
    for (index, item) in items.into_iter().enumerate() {
        entries.push(parse_item(item, index)?);
    }
    Ok(entries)
}

fn parse_item(item: Node<'_, '_>, index: usize) -> Result<FeedEntry, Error> {
    let required = |name| {
        child_text(item, name).ok_or_else(|| {
            Error::Letterboxd(format!("RSS item {} is missing required {name}", index + 1))
        })
    };

    let film_title = required("filmTitle")?;
    let link = required("link")?;
    let guid = required("guid")?;
    if !is_safe_https_url(&link) {
        return Err(Error::Letterboxd(format!(
            "RSS item {} contains a non-HTTPS or invalid link",
            index + 1
        )));
    }

    let film_year = child_text(item, "filmYear")
        .map(|value| {
            value.parse::<u16>().map_err(|error| {
                Error::Letterboxd(format!(
                    "RSS item {} has invalid filmYear {value:?}: {error}",
                    index + 1
                ))
            })
        })
        .transpose()?;

    let member_rating = child_text(item, "memberRating")
        .map(|value| {
            let rating = value.parse::<f32>().map_err(|error| {
                Error::Letterboxd(format!(
                    "RSS item {} has invalid memberRating {value:?}: {error}",
                    index + 1
                ))
            })?;
            if !(0.0..=5.0).contains(&rating) {
                return Err(Error::Letterboxd(format!(
                    "RSS item {} has out-of-range memberRating {rating}",
                    index + 1
                )));
            }
            Ok(rating)
        })
        .transpose()?;

    let description = child_text(item, "description");
    let review_excerpt = description
        .as_deref()
        .map(html_to_plain_text)
        .filter(|text| !text.is_empty());
    let poster_url = description.as_deref().and_then(extract_https_poster);

    Ok(FeedEntry {
        film_title,
        film_year,
        member_rating,
        review_excerpt,
        link,
        poster_url,
        published_at: child_text(item, "pubDate"),
        guid,
    })
}

fn child_text(node: Node<'_, '_>, local_name: &str) -> Option<String> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == local_name)
        .and_then(|child| child.text())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn is_safe_https_url(candidate: &str) -> bool {
    reqwest::Url::parse(candidate).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

fn extract_https_poster(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(relative_start) = lower[search_from..].find("<img") {
        let tag_start = search_from + relative_start;
        let tag_end = find_tag_end(html, tag_start + 4)?;
        let tag = &html[tag_start + 4..tag_end];
        if let Some(src) = attribute_value(tag, "src") {
            let decoded = decode_html_entities(src.trim());
            if is_safe_https_url(&decoded) {
                return Some(decoded);
            }
        }
        search_from = tag_end + 1;
    }
    None
}

fn find_tag_end(input: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (offset, character) in input[start..].char_indices() {
        match (quote, character) {
            (None, '\'' | '"') => quote = Some(character),
            (Some(active), current) if active == current => quote = None,
            (None, '>') => return Some(start + offset),
            _ => {}
        }
    }
    None
}

fn attribute_value<'a>(tag: &'a str, wanted: &str) -> Option<&'a str> {
    let bytes = tag.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        while cursor < bytes.len() && (bytes[cursor].is_ascii_whitespace() || bytes[cursor] == b'/')
        {
            cursor += 1;
        }
        let name_start = cursor;
        while cursor < bytes.len()
            && (bytes[cursor].is_ascii_alphanumeric() || matches!(bytes[cursor], b'-' | b'_'))
        {
            cursor += 1;
        }
        if cursor == name_start {
            cursor += 1;
            continue;
        }
        let name = &tag[name_start..cursor];
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'=' {
            continue;
        }
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() {
            return None;
        }

        let (value_start, value_end) = if matches!(bytes[cursor], b'\'' | b'"') {
            let quote = bytes[cursor];
            cursor += 1;
            let start = cursor;
            while cursor < bytes.len() && bytes[cursor] != quote {
                cursor += 1;
            }
            (start, cursor)
        } else {
            let start = cursor;
            while cursor < bytes.len() && !bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            (start, cursor)
        };
        if name.eq_ignore_ascii_case(wanted) {
            return Some(&tag[value_start..value_end]);
        }
    }
    None
}

fn html_to_plain_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len().min(REVIEW_EXCERPT_CHARS));
    let mut rest = html;
    while let Some(tag_start) = rest.find('<') {
        text.push_str(&rest[..tag_start]);
        let Some(relative_end) = rest[tag_start..].find('>') else {
            break;
        };
        let tag = rest[tag_start + 1..tag_start + relative_end]
            .trim_start_matches('/')
            .split_ascii_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(tag.as_str(), "br" | "div" | "li" | "p") {
            text.push(' ');
        }
        rest = &rest[tag_start + relative_end + 1..];
    }
    text.push_str(rest);

    let decoded = decode_html_entities(&text);
    let normalized = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&normalized, REVIEW_EXCERPT_CHARS)
}

fn decode_html_entities(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(ampersand) = rest.find('&') {
        output.push_str(&rest[..ampersand]);
        rest = &rest[ampersand..];
        let Some(semicolon) = rest.find(';') else {
            output.push_str(rest);
            return output;
        };
        let entity = &rest[1..semicolon];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                u32::from_str_radix(&entity[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if entity.starts_with('#') => {
                entity[1..].parse::<u32>().ok().and_then(char::from_u32)
            }
            _ => None,
        };
        if let Some(character) = decoded {
            output.push(character);
        } else {
            output.push_str(&rest[..=semicolon]);
        }
        rest = &rest[semicolon + 1..];
    }
    output.push_str(rest);
    output
}

fn truncate_chars(input: &str, limit: usize) -> String {
    let mut characters = input.chars();
    let prefix: String = characters.by_ref().take(limit).collect();
    if characters.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSS_FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:letterboxd="https://letterboxd.com">
  <channel>
    <title>Films by ahmed</title>
    <item>
      <title>ahmed watched Perfect Blue</title>
      <link>https://letterboxd.com/ahmed/film/perfect-blue/</link>
      <guid isPermaLink="false">letterboxd-watch-123</guid>
      <pubDate>Sat, 15 Aug 2026 20:00:00 +0000</pubDate>
      <letterboxd:filmTitle>Perfect Blue</letterboxd:filmTitle>
      <letterboxd:filmYear>1997</letterboxd:filmYear>
      <letterboxd:memberRating>4.5</letterboxd:memberRating>
      <description><![CDATA[<p><img src="https://a.ltrbxd.com/resized/perfect-blue.jpg"/></p><p>A beautiful &amp; unsettling <strong>masterpiece</strong>.</p>]]></description>
    </item>
    <item>
      <title>ahmed watched Millennium Actress</title>
      <link>https://letterboxd.com/ahmed/film/millennium-actress/</link>
      <guid>letterboxd-watch-124</guid>
      <letterboxd:filmTitle>Millennium Actress</letterboxd:filmTitle>
      <letterboxd:filmYear>2001</letterboxd:filmYear>
      <description><![CDATA[<p>No rating, still lovely.</p>]]></description>
    </item>
  </channel>
</rss>"#;

    #[test]
    fn parses_namespaced_film_fields_and_sanitizes_description() {
        let entries = parse_member_feed(RSS_FIXTURE).unwrap();
        assert_eq!(entries.len(), 2);

        let first = &entries[0];
        assert_eq!(first.film_title, "Perfect Blue");
        assert_eq!(first.film_year, Some(1997));
        assert_eq!(first.member_rating, Some(4.5));
        assert_eq!(
            first.review_excerpt.as_deref(),
            Some("A beautiful & unsettling masterpiece.")
        );
        assert_eq!(
            first.poster_url.as_deref(),
            Some("https://a.ltrbxd.com/resized/perfect-blue.jpg")
        );
        assert_eq!(first.guid, "letterboxd-watch-123");
        assert!(first.published_at.is_some());

        assert_eq!(entries[1].member_rating, None);
        assert_eq!(entries[1].poster_url, None);
        assert_eq!(entries[1].published_at, None);
    }

    #[test]
    fn username_validation_blocks_path_and_url_injection() {
        for bad in [
            "",
            "two words",
            "../admin",
            "name/rss",
            "name%2fadmin",
            "💫",
        ] {
            assert!(validate_username(bad).is_err(), "accepted {bad:?}");
        }
        for good in ["a", "Ahmed_42", "film-fan"] {
            validate_username(good).unwrap();
        }
    }

    #[test]
    fn unsafe_poster_sources_are_not_exposed() {
        let html = r#"<img src="javascript:alert(1)"><img src="http://example.com/poster.jpg">"#;
        assert_eq!(extract_https_poster(html), None);
    }

    #[test]
    fn rejects_missing_required_fields_and_bad_rating() {
        let missing_guid = RSS_FIXTURE.replace(
            "<guid isPermaLink=\"false\">letterboxd-watch-123</guid>",
            "",
        );
        assert!(parse_member_feed(&missing_guid)
            .unwrap_err()
            .to_string()
            .contains("guid"));

        let bad_rating = RSS_FIXTURE.replace(">4.5<", ">9.0<");
        assert!(parse_member_feed(&bad_rating)
            .unwrap_err()
            .to_string()
            .contains("out-of-range"));
    }

    #[test]
    fn excerpts_are_unicode_safe_and_bounded() {
        let long = "✨".repeat(REVIEW_EXCERPT_CHARS + 1);
        let excerpt = html_to_plain_text(&long);
        assert_eq!(excerpt.chars().count(), REVIEW_EXCERPT_CHARS + 1);
        assert!(excerpt.ends_with('…'));
    }
}
