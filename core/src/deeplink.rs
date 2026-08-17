//! Deep-link parsing for search, discover, detail, video, and addon-install
//! routes (S21 of `docs/roadmaps/stremio-competitive-parity.md`).
//!
//! This module only parses and validates; it does not register an OS-level
//! custom URL scheme handler (no `ankai://` protocol is wired into the
//! platform launcher yet — that's a packaging step for a later release
//! phase, not a parsing one) and it does not perform any navigation itself.
//! A client hands [`parse`] whatever string it received — a paste into an
//! "open a link" field today, an OS-delivered URL once one is registered —
//! and acts on the returned [`DeepLink`].
//!
//! Five route kinds are recognized, all under the `ankai://` scheme:
//!
//! - `ankai://search?q=<text>`
//! - `ankai://discover?type=<t>&addon=<https addon URL>&catalog=<id>&genre=<g>`
//!   (every parameter optional — an empty discover link means "browse
//!   everything")
//! - `ankai://detail?type=<t>&id=<id>&addon=<https addon URL>`
//! - `ankai://video?type=<t>&id=<meta id>&video=<video id>&addon=<https addon URL>`
//! - `ankai://addon-install?url=<https manifest URL>`
//!
//! Two extra forms are accepted and normalized to [`DeepLink::AddonInstall`]
//! because they are exactly what a real user hands ANKAI in practice:
//!
//! - A `stremio://host/path` link — the real format Stremio's own "Install"
//!   buttons and addon catalogs produce. Rewriting `stremio://` to
//!   `https://` is not a workaround; it is what the scheme is documented to
//!   mean (see [`crate::stremio::AddonTransportOutcome::UnsupportedStremioDeepLink`]'s
//!   rejection message).
//! - A bare `https://…manifest.json`-shaped URL pasted directly, with no
//!   `ankai://` wrapper at all.
//!
//! Every embedded addon/manifest URL is validated with
//! [`crate::stremio::validate_public_https_url_str`] — the exact same
//! public/HTTPS/credential-free policy the addon client itself enforces —
//! so a deep link cannot be used to smuggle a request to a private,
//! loopback, or non-HTTPS target past that boundary. Query parameters are
//! rejected as ambiguous if repeated (`?id=1&id=2`), and required
//! parameters missing from a route make that link invalid rather than
//! guessed at.

use std::collections::HashMap;

use crate::error::Error;
use crate::stremio::validate_public_https_url_str;

const MAX_LINK_LENGTH: usize = 4096;

/// One recognized, validated ANKAI deep link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLink {
    Search {
        query: String,
    },
    Discover {
        media_type: Option<String>,
        addon_manifest_url: Option<String>,
        catalog_id: Option<String>,
        genre: Option<String>,
    },
    Detail {
        media_type: String,
        id: String,
        addon_manifest_url: Option<String>,
    },
    Video {
        media_type: String,
        id: String,
        video_id: String,
        addon_manifest_url: Option<String>,
    },
    AddonInstall {
        manifest_url: String,
    },
}

/// Parses and validates a deep link string. See the module doc comment for
/// the recognized forms and the safety rules applied to embedded URLs.
pub fn parse(raw: &str) -> Result<DeepLink, Error> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(Error::Stremio("deep link is empty".into()));
    }
    if raw.len() > MAX_LINK_LENGTH {
        return Err(Error::Stremio("deep link is too long".into()));
    }

    if let Some(rest) = raw.strip_prefix("stremio://") {
        return parse_addon_install(&format!("https://{rest}"));
    }
    if raw.starts_with("https://") {
        return parse_addon_install(raw);
    }

    let url = reqwest::Url::parse(raw)
        .map_err(|error| Error::Stremio(format!("unrecognized deep link: {error}")))?;
    match url.scheme() {
        "ankai" => parse_ankai_route(&url),
        other => Err(Error::Stremio(format!(
            "unsupported deep link scheme {other:?}"
        ))),
    }
}

fn parse_ankai_route(url: &reqwest::Url) -> Result<DeepLink, Error> {
    let route = url
        .host_str()
        .ok_or_else(|| Error::Stremio("deep link is missing a route".into()))?
        .to_ascii_lowercase();
    let params = single_valued_query(url)?;

    match route.as_str() {
        "search" => {
            let query = required_field(&params, "q", 512)?;
            Ok(DeepLink::Search { query })
        }
        "discover" => Ok(DeepLink::Discover {
            media_type: optional_field(&params, "type", 64)?,
            addon_manifest_url: optional_addon_url(&params)?,
            catalog_id: optional_field(&params, "catalog", 1024)?,
            genre: optional_field(&params, "genre", 512)?,
        }),
        "detail" => Ok(DeepLink::Detail {
            media_type: required_field(&params, "type", 64)?,
            id: required_field(&params, "id", 1024)?,
            addon_manifest_url: optional_addon_url(&params)?,
        }),
        "video" => Ok(DeepLink::Video {
            media_type: required_field(&params, "type", 64)?,
            id: required_field(&params, "id", 1024)?,
            video_id: required_field(&params, "video", 1024)?,
            addon_manifest_url: optional_addon_url(&params)?,
        }),
        "addon-install" => {
            let manifest_url = required_field(&params, "url", MAX_LINK_LENGTH)?;
            parse_addon_install(&manifest_url)
        }
        other => Err(Error::Stremio(format!(
            "unsupported deep link route {other:?}"
        ))),
    }
}

fn parse_addon_install(url: &str) -> Result<DeepLink, Error> {
    validate_public_https_url_str(url, "addon install URL")?;
    Ok(DeepLink::AddonInstall {
        manifest_url: url.to_string(),
    })
}

/// Collects query parameters into a map, rejecting a link that repeats the
/// same key — an ambiguous target (which value would win?) rather than a
/// safe one to silently resolve by "last wins".
fn single_valued_query(url: &reqwest::Url) -> Result<HashMap<String, String>, Error> {
    let mut map = HashMap::new();
    for (key, value) in url.query_pairs() {
        if map.insert(key.to_string(), value.to_string()).is_some() {
            return Err(Error::Stremio(format!(
                "deep link parameter {key:?} is ambiguous (repeated)"
            )));
        }
    }
    Ok(map)
}

fn required_field(
    params: &HashMap<String, String>,
    key: &str,
    max_len: usize,
) -> Result<String, Error> {
    optional_field(params, key, max_len)?
        .ok_or_else(|| Error::Stremio(format!("deep link is missing required parameter {key:?}")))
}

fn optional_field(
    params: &HashMap<String, String>,
    key: &str,
    max_len: usize,
) -> Result<Option<String>, Error> {
    let Some(value) = params.get(key).map(|value| value.trim()) else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > max_len {
        return Err(Error::Stremio(format!(
            "deep link parameter {key:?} is too long"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(Error::Stremio(format!(
            "deep link parameter {key:?} contains control characters"
        )));
    }
    Ok(Some(value.to_string()))
}

fn optional_addon_url(params: &HashMap<String, String>) -> Result<Option<String>, Error> {
    let Some(url) = optional_field(params, "addon", MAX_LINK_LENGTH)? else {
        return Ok(None);
    };
    validate_public_https_url_str(&url, "deep link addon URL")?;
    Ok(Some(url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search() {
        let link = parse("ankai://search?q=frieren").unwrap();
        assert_eq!(
            link,
            DeepLink::Search {
                query: "frieren".into()
            }
        );
    }

    #[test]
    fn rejects_empty_search_query() {
        assert!(parse("ankai://search?q=").is_err());
        assert!(parse("ankai://search").is_err());
    }

    #[test]
    fn parses_discover_with_all_fields_optional() {
        assert_eq!(
            parse("ankai://discover").unwrap(),
            DeepLink::Discover {
                media_type: None,
                addon_manifest_url: None,
                catalog_id: None,
                genre: None,
            }
        );

        let link = parse(
            "ankai://discover?type=series&catalog=top&genre=Action&addon=https%3A%2F%2Faddon.example%2Fmanifest.json",
        )
        .unwrap();
        assert_eq!(
            link,
            DeepLink::Discover {
                media_type: Some("series".into()),
                addon_manifest_url: Some("https://addon.example/manifest.json".into()),
                catalog_id: Some("top".into()),
                genre: Some("Action".into()),
            }
        );
    }

    #[test]
    fn parses_detail_and_requires_type_and_id() {
        let link = parse("ankai://detail?type=movie&id=tt1234567").unwrap();
        assert_eq!(
            link,
            DeepLink::Detail {
                media_type: "movie".into(),
                id: "tt1234567".into(),
                addon_manifest_url: None,
            }
        );
        assert!(parse("ankai://detail?type=movie").is_err());
        assert!(parse("ankai://detail?id=tt1234567").is_err());
    }

    #[test]
    fn parses_video_and_requires_video_id() {
        let link = parse("ankai://video?type=series&id=tt1&video=tt1:1:2").unwrap();
        assert_eq!(
            link,
            DeepLink::Video {
                media_type: "series".into(),
                id: "tt1".into(),
                video_id: "tt1:1:2".into(),
                addon_manifest_url: None,
            }
        );
        assert!(parse("ankai://video?type=series&id=tt1").is_err());
    }

    #[test]
    fn parses_addon_install_from_all_three_forms() {
        let expected = DeepLink::AddonInstall {
            manifest_url: "https://addon.example/manifest.json".into(),
        };
        assert_eq!(
            parse("ankai://addon-install?url=https%3A%2F%2Faddon.example%2Fmanifest.json").unwrap(),
            expected
        );
        assert_eq!(
            parse("stremio://addon.example/manifest.json").unwrap(),
            expected
        );
        assert_eq!(
            parse("https://addon.example/manifest.json").unwrap(),
            expected
        );
    }

    #[test]
    fn rejects_unsafe_or_ambiguous_addon_targets() {
        assert!(
            parse("ankai://addon-install?url=http%3A%2F%2Faddon.example%2Fmanifest.json").is_err()
        );
        assert!(
            parse("ankai://addon-install?url=https%3A%2F%2F127.0.0.1%2Fmanifest.json").is_err()
        );
        assert!(parse("stremio://localhost/manifest.json").is_err());
        assert!(
            parse("ankai://detail?type=movie&id=tt1&addon=http%3A%2F%2Faddon.example").is_err()
        );
    }

    #[test]
    fn rejects_repeated_parameters_as_ambiguous() {
        assert!(parse("ankai://detail?type=movie&type=series&id=tt1").is_err());
    }

    #[test]
    fn rejects_unsupported_schemes_and_routes() {
        assert!(parse("http://search?q=x").is_err());
        assert!(parse("ankai://unknown-route").is_err());
        assert!(parse("").is_err());
    }
}
