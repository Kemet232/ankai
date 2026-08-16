//! Client for Stremio's public addon protocol.
//!
//! Addons are live-fetched and require no authentication. The supplied base URL is
//! deliberately preserved in full: real addons sometimes put configuration in a
//! path segment immediately before `manifest.json` (including plain or base64
//! configuration), so it must not be reduced to an origin or assumed to be the
//! addon root.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Error;

/// A reusable client for one installed/configured addon.
#[derive(Debug, Clone)]
pub struct AddonClient {
    client: reqwest::Client,
    base_url: reqwest::Url,
}

impl AddonClient {
    /// Creates a client from either an addon base URL or its `manifest.json` URL.
    pub fn new(base_url: &str) -> Result<Self, Error> {
        let mut base_url = reqwest::Url::parse(base_url)
            .map_err(|e| Error::Stremio(format!("invalid addon URL: {e}")))?;
        if !matches!(base_url.scheme(), "http" | "https") {
            return Err(Error::Stremio("addon URL must use http or https".into()));
        }
        base_url.set_query(None);
        base_url.set_fragment(None);
        if base_url.path().ends_with("/manifest.json") {
            let path = base_url.path().trim_end_matches("manifest.json").to_owned();
            base_url.set_path(&path);
        } else if !base_url.path().ends_with('/') {
            let path = format!("{}/", base_url.path());
            base_url.set_path(&path);
        }
        Ok(Self {
            client: reqwest::Client::new(),
            base_url,
        })
    }

    pub async fn manifest(&self) -> Result<Manifest, Error> {
        self.get(self.endpoint(&["manifest.json"])?, "manifest")
            .await
    }

    pub async fn catalog(
        &self,
        media_type: &str,
        catalog_id: &str,
    ) -> Result<Vec<MetaPreview>, Error> {
        self.catalog_with_extra(media_type, catalog_id, &[]).await
    }

    /// Fetches a catalog with Stremio's path-encoded extras, such as
    /// `search=naruto` or `genre=Anime&skip=100`.
    pub async fn catalog_with_extra(
        &self,
        media_type: &str,
        catalog_id: &str,
        extra: &[(&str, &str)],
    ) -> Result<Vec<MetaPreview>, Error> {
        let mut parts = vec!["catalog", media_type, catalog_id];
        let extra_segment;
        let file;
        if extra.is_empty() {
            file = format!("{catalog_id}.json");
            parts[2] = &file;
        } else {
            let mut encoded = reqwest::Url::parse("https://extras.invalid/").unwrap();
            encoded
                .query_pairs_mut()
                .extend_pairs(extra.iter().copied());
            extra_segment = format!("{}.json", encoded.query().unwrap_or_default());
            parts.push(&extra_segment);
        }
        let response: CatalogResponse = self.get(self.endpoint(&parts)?, "catalog").await?;
        Ok(response.metas)
    }

    pub async fn meta(&self, media_type: &str, id: &str) -> Result<Meta, Error> {
        let file = format!("{id}.json");
        let response: MetaResponse = self
            .get(self.endpoint(&["meta", media_type, &file])?, "meta")
            .await?;
        Ok(response.meta)
    }

    pub async fn streams(&self, media_type: &str, id: &str) -> Result<Vec<Stream>, Error> {
        let file = format!("{id}.json");
        let response: StreamResponse = self
            .get(self.endpoint(&["stream", media_type, &file])?, "stream")
            .await?;
        Ok(response.streams)
    }

    fn endpoint(&self, parts: &[&str]) -> Result<reqwest::Url, Error> {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|_| Error::Stremio("addon URL cannot be used as a base URL".into()))?
            .pop_if_empty()
            .extend(parts);
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
            .map_err(|e| Error::Stremio(format!("{resource} request failed: {e}")))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| Error::Stremio(format!("failed to read {resource} response: {e}")))?;
        if !status.is_success() {
            return Err(Error::Stremio(format!(
                "{resource} returned HTTP {status}: {body}"
            )));
        }
        serde_json::from_str(&body)
            .map_err(|e| Error::Stremio(format!("failed to parse {resource} response: {e}")))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Resource {
    Name(String),
    Descriptor {
        name: String,
        #[serde(default)]
        types: Vec<String>,
        #[serde(default, rename = "idPrefixes")]
        id_prefixes: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub resources: Vec<Resource>,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub catalogs: Vec<Catalog>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    #[serde(rename = "type")]
    pub media_type: String,
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub extra: Vec<CatalogExtra>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogExtra {
    pub name: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default, rename = "isRequired")]
    pub is_required: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaPreview {
    pub id: String,
    #[serde(rename = "type")]
    pub media_type: String,
    pub name: String,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default, rename = "releaseInfo")]
    pub release_info: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub id: String,
    #[serde(rename = "type")]
    pub media_type: String,
    pub name: String,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub cast: Vec<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub videos: Vec<Video>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Video {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub season: Option<u32>,
    #[serde(default)]
    pub episode: Option<u32>,
    #[serde(default)]
    pub released: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stream {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default, rename = "infoHash")]
    pub info_hash: Option<String>,
    #[serde(default, rename = "fileIdx")]
    pub file_idx: Option<u32>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

/// The exact playback fork a UI/player must handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamSource<'a> {
    Direct(&'a str),
    BitTorrent {
        info_hash: &'a str,
        file_idx: Option<u32>,
        sources: &'a [String],
    },
}

impl Stream {
    /// Returns a direct libmpv URL or a BitTorrent descriptor for a resolver.
    pub fn source(&self) -> Result<StreamSource<'_>, Error> {
        if let Some(url) = self.url.as_deref() {
            return Ok(StreamSource::Direct(url));
        }
        if let Some(info_hash) = self.info_hash.as_deref() {
            return Ok(StreamSource::BitTorrent {
                info_hash,
                file_idx: self.file_idx,
                sources: &self.sources,
            });
        }
        Err(Error::Stremio("stream has neither url nor infoHash".into()))
    }

    /// Reconstructs the magnet URI used to hand a torrent stream to a resolver.
    pub fn magnet_uri(&self) -> Result<Option<String>, Error> {
        let StreamSource::BitTorrent {
            info_hash, sources, ..
        } = self.source()?
        else {
            return Ok(None);
        };
        let mut magnet = reqwest::Url::parse("magnet:?xt=urn:btih:")
            .map_err(|e| Error::Stremio(format!("failed to build magnet URI: {e}")))?;
        magnet
            .query_pairs_mut()
            .clear()
            .append_pair("xt", &format!("urn:btih:{info_hash}"));
        for source in sources {
            magnet.query_pairs_mut().append_pair("tr", source);
        }
        Ok(Some(magnet.into()))
    }
}

#[derive(Deserialize)]
struct CatalogResponse {
    #[serde(default)]
    metas: Vec<MetaPreview>,
}
#[derive(Deserialize)]
struct MetaResponse {
    meta: Meta,
}
#[derive(Deserialize)]
struct StreamResponse {
    #[serde(default)]
    streams: Vec<Stream>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_manifest_url_keeps_its_path() {
        let client =
            AddonClient::new("https://example.com/c29tZS1jb25maWc=/manifest.json").unwrap();
        assert_eq!(
            client.endpoint(&["manifest.json"]).unwrap().as_str(),
            "https://example.com/c29tZS1jb25maWc=/manifest.json"
        );
    }

    #[test]
    fn stream_models_the_direct_and_torrent_fork() {
        let direct: Stream =
            serde_json::from_str(r#"{"url":"https://video.example/a.m3u8"}"#).unwrap();
        assert_eq!(
            direct.source().unwrap(),
            StreamSource::Direct("https://video.example/a.m3u8")
        );
        let torrent: Stream = serde_json::from_str(
            r#"{"infoHash":"abc123","fileIdx":2,"sources":["udp://tracker.example"]}"#,
        )
        .unwrap();
        assert!(matches!(
            torrent.source().unwrap(),
            StreamSource::BitTorrent {
                file_idx: Some(2),
                ..
            }
        ));
        assert_eq!(
            torrent.magnet_uri().unwrap().unwrap(),
            "magnet:?xt=urn%3Abtih%3Aabc123&tr=udp%3A%2F%2Ftracker.example"
        );
    }
}
