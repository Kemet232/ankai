//! Client for Stremio's public addon protocol.
//!
//! Addons are live-fetched and require no authentication. The supplied base URL is
//! deliberately preserved in full: real addons sometimes put configuration in a
//! path segment immediately before `manifest.json` (including plain or base64
//! configuration), so it must not be reduced to an origin or assumed to be the
//! addon root.

use std::collections::{HashMap, HashSet};
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Error;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_REDIRECTS: usize = 3;
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_CATALOG_BYTES: usize = 4 * 1024 * 1024;
const MAX_META_BYTES: usize = 4 * 1024 * 1024;
const MAX_STREAM_BYTES: usize = 2 * 1024 * 1024;
const MAX_SUBTITLES_BYTES: usize = 2 * 1024 * 1024;
const MAX_ADDON_CATALOG_BYTES: usize = 4 * 1024 * 1024;
const MAX_ERROR_EXCERPT_BYTES: usize = 512;
const MAX_URL_LENGTH: usize = 8 * 1024;
const MAX_CATALOG_ITEMS: usize = 500;
const MAX_STREAM_ITEMS: usize = 500;
const MAX_SUBTITLE_ITEMS: usize = 500;
const MAX_ADDON_CATALOG_ITEMS: usize = 128;
const MAX_VIDEO_ITEMS: usize = 2_000;
const MAX_COLLECTION_ITEMS: usize = 256;
const MAX_EXTRA_FIELDS: usize = 64;
const MAX_EXTRA_DEPTH: usize = 8;
const MAX_EXTRA_NODES: usize = 2_048;
const MAX_SHORT_STRING: usize = 1_024;
const MAX_LONG_STRING: usize = 64 * 1024;

/// A reusable client for one installed/configured addon.
#[derive(Debug, Clone)]
pub struct AddonClient {
    client: reqwest::Client,
    base_url: reqwest::Url,
}

/// The transport outcome ANKAI assigns to an addon URL.
///
/// Stremio's protocol has historically been transported over HTTP, IPFS and
/// IPNS as well as HTTPS. ANKAI's network boundary intentionally implements
/// only public HTTPS today. Keeping the unsupported outcomes distinct lets the
/// addon manager explain what needs a future gateway instead of collapsing
/// every non-HTTPS URL into an unhelpful parse error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddonTransportOutcome {
    SupportedHttps,
    UnsupportedInsecureHttp,
    UnsupportedLegacyV1,
    UnsupportedLegacyV2,
    UnsupportedIpfs,
    UnsupportedIpns,
    UnsupportedStremioDeepLink,
    UnsupportedScheme(String),
}

impl AddonTransportOutcome {
    pub fn is_supported(&self) -> bool {
        matches!(self, Self::SupportedHttps)
    }

    fn rejection_reason(&self) -> Option<String> {
        match self {
            Self::SupportedHttps => None,
            Self::UnsupportedInsecureHttp => Some(
                "plain HTTP addon transports are not supported; use a public HTTPS endpoint".into(),
            ),
            Self::UnsupportedLegacyV1 => {
                Some("legacy Stremio v1 addon transports are not supported".into())
            }
            Self::UnsupportedLegacyV2 => {
                Some("legacy Stremio v2 addon transports are not supported".into())
            }
            Self::UnsupportedIpfs => {
                Some("IPFS addon transports require a trusted gateway and are not supported yet".into())
            }
            Self::UnsupportedIpns => {
                Some("IPNS addon transports require a trusted gateway and are not supported yet".into())
            }
            Self::UnsupportedStremioDeepLink => Some(
                "stremio:// is an install deep link, not a fetch transport; provide its HTTPS manifest URL"
                    .into(),
            ),
            Self::UnsupportedScheme(scheme) => {
                Some(format!("unsupported addon transport scheme {scheme:?}; use HTTPS"))
            }
        }
    }
}

/// DNS resolver that returns only public destination addresses.
///
/// Resolution happens inside reqwest's connection path, so the addresses this
/// implementation validates are the addresses the connector receives. Both
/// clients using it disable environment proxies; otherwise a proxy could
/// resolve the destination independently and bypass this check.
#[derive(Debug, Clone, Copy, Default)]
pub struct PublicDnsResolver;

impl reqwest::dns::Resolve for PublicDnsResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let addresses = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|error| {
                    Box::new(error) as Box<dyn std::error::Error + Send + Sync + 'static>
                })?
                .filter(|address| !is_non_public_ip(address.ip()))
                .collect::<Vec<SocketAddr>>();
            if addresses.is_empty() {
                return Err(Box::new(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "DNS name did not resolve to a public address",
                ))
                    as Box<dyn std::error::Error + Send + Sync + 'static>);
            }
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

impl AddonClient {
    /// Classifies a manifest/base URL without silently rewriting legacy or
    /// distributed transports through a third-party gateway.
    pub fn transport_outcome(url: &str) -> Result<AddonTransportOutcome, Error> {
        let parsed = reqwest::Url::parse(url)
            .map_err(|e| Error::Stremio(format!("invalid addon URL: {e}")))?;
        let normalized_path = parsed.path().trim_end_matches('/');
        if normalized_path.ends_with("/stremio/v1") {
            return Ok(AddonTransportOutcome::UnsupportedLegacyV1);
        }
        if normalized_path.ends_with("/stremio/v2") {
            return Ok(AddonTransportOutcome::UnsupportedLegacyV2);
        }
        Ok(match parsed.scheme() {
            "https" => AddonTransportOutcome::SupportedHttps,
            "http" => AddonTransportOutcome::UnsupportedInsecureHttp,
            "ipfs" => AddonTransportOutcome::UnsupportedIpfs,
            "ipns" => AddonTransportOutcome::UnsupportedIpns,
            "stremio" => AddonTransportOutcome::UnsupportedStremioDeepLink,
            scheme => AddonTransportOutcome::UnsupportedScheme(scheme.to_owned()),
        })
    }

    /// Creates a client from either an addon base URL or its `manifest.json` URL.
    pub fn new(base_url: &str) -> Result<Self, Error> {
        let transport = Self::transport_outcome(base_url)?;
        if let Some(reason) = transport.rejection_reason() {
            return Err(Error::Stremio(reason));
        }
        let mut base_url = reqwest::Url::parse(base_url)
            .map_err(|e| Error::Stremio(format!("invalid addon URL: {e}")))?;
        validate_public_https_url(&base_url, "addon URL")?;
        base_url.set_query(None);
        base_url.set_fragment(None);
        if base_url.path().ends_with("/manifest.json") {
            let path = base_url.path().trim_end_matches("manifest.json").to_owned();
            base_url.set_path(&path);
        } else if !base_url.path().ends_with('/') {
            let path = format!("{}/", base_url.path());
            base_url.set_path(&path);
        }
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .no_proxy()
            .dns_resolver(PublicDnsResolver)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let Some(initial) = attempt.previous().first() else {
                    return attempt.error("redirect has no initial request URL");
                };
                if attempt.previous().len() > MAX_REDIRECTS {
                    return attempt.error("too many addon redirects");
                }
                if !same_origin(initial, attempt.url())
                    && !is_official_cinemeta_redirect(initial, attempt.url())
                {
                    return attempt.error("addon redirect changed origin");
                }
                if validate_public_https_url(attempt.url(), "addon redirect URL").is_err() {
                    return attempt.error("addon redirect URL is not public HTTPS");
                }
                attempt.follow()
            }))
            .user_agent(concat!("ankai/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::Stremio(format!("failed to build addon HTTP client: {e}")))?;
        Ok(Self { client, base_url })
    }

    pub async fn manifest(&self) -> Result<Manifest, Error> {
        let manifest: Manifest = self
            .get(
                self.endpoint(&["manifest.json"])?,
                "manifest",
                MAX_MANIFEST_BYTES,
            )
            .await?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Returns the canonical manifest URL this client will request.
    ///
    /// Keeping this normalization here matters for installed-addon
    /// persistence: a base URL and the equivalent explicit `manifest.json`
    /// URL should identify one installation, while configured path segments
    /// remain part of that identity.
    pub fn manifest_url(&self) -> Result<String, Error> {
        Ok(self.endpoint(&["manifest.json"])?.into())
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
        Ok(self
            .catalog_page_with_extra(media_type, catalog_id, extra)
            .await?
            .metas)
    }

    /// Fetches a catalog page along with the response-level cache hints
    /// (`cacheMaxAge`, `staleRevalidate`, `staleError`) the addon protocol
    /// documents alongside `metas`. [`crate::board`]'s stale-while-revalidate
    /// cache reads these to decide how long a page stays fresh; callers that
    /// only need the items themselves should keep using
    /// [`Self::catalog_with_extra`].
    pub async fn catalog_page_with_extra(
        &self,
        media_type: &str,
        catalog_id: &str,
        extra: &[(&str, &str)],
    ) -> Result<CatalogPage, Error> {
        validate_path_value("catalog type", media_type)?;
        validate_path_value("catalog id", catalog_id)?;
        if extra.len() > 16 {
            return Err(Error::Stremio(
                "catalog request has too many extra parameters".into(),
            ));
        }
        for (name, value) in extra {
            validate_string("catalog extra name", name, 64, true)?;
            validate_string("catalog extra value", value, 512, false)?;
        }
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
        let response: CatalogResponse = self
            .get(self.endpoint(&parts)?, "catalog", MAX_CATALOG_BYTES)
            .await?;
        if response.metas.len() > MAX_CATALOG_ITEMS {
            return Err(Error::Stremio(format!(
                "catalog contains too many items (maximum {MAX_CATALOG_ITEMS})"
            )));
        }
        for meta in &response.metas {
            meta.validate()?;
        }
        Ok(CatalogPage {
            metas: response.metas,
            cache_max_age: response.cache_max_age,
            stale_revalidate: response.stale_revalidate,
            stale_error: response.stale_error,
        })
    }

    pub async fn meta(&self, media_type: &str, id: &str) -> Result<Meta, Error> {
        validate_path_value("meta type", media_type)?;
        validate_path_value("meta id", id)?;
        let file = format!("{id}.json");
        let response: MetaResponse = self
            .get(
                self.endpoint(&["meta", media_type, &file])?,
                "meta",
                MAX_META_BYTES,
            )
            .await?;
        response.meta.validate()?;
        Ok(response.meta)
    }

    pub async fn streams(&self, media_type: &str, id: &str) -> Result<Vec<Stream>, Error> {
        validate_path_value("stream type", media_type)?;
        validate_path_value("stream id", id)?;
        let file = format!("{id}.json");
        let response: StreamResponse = self
            .get(
                self.endpoint(&["stream", media_type, &file])?,
                "stream",
                MAX_STREAM_BYTES,
            )
            .await?;
        if response.streams.len() > MAX_STREAM_ITEMS {
            return Err(Error::Stremio(format!(
                "stream response contains too many items (maximum {MAX_STREAM_ITEMS})"
            )));
        }
        for stream in &response.streams {
            stream.validate()?;
        }
        Ok(response.streams)
    }

    /// Fetches subtitles for a video or OpenSubtitles hash. Extra arguments
    /// commonly include `videoHash`, `videoSize`, `filename` and `videoId`.
    pub async fn subtitles(&self, media_type: &str, id: &str) -> Result<Vec<Subtitle>, Error> {
        self.subtitles_with_extra(media_type, id, &[]).await
    }

    pub async fn subtitles_with_extra(
        &self,
        media_type: &str,
        id: &str,
        extra: &[(&str, &str)],
    ) -> Result<Vec<Subtitle>, Error> {
        validate_path_value("subtitles type", media_type)?;
        validate_path_value("subtitles id", id)?;
        let endpoint = self.resource_endpoint("subtitles", media_type, id, extra)?;
        let response: SubtitlesResponse =
            self.get(endpoint, "subtitles", MAX_SUBTITLES_BYTES).await?;
        ensure_collection_limit(
            "subtitle response",
            response.subtitles.len(),
            MAX_SUBTITLE_ITEMS,
        )?;
        for subtitle in &response.subtitles {
            subtitle.validate()?;
        }
        Ok(response.subtitles)
    }

    /// Fetches a manifest repository exposed through Stremio's
    /// `addon_catalog` resource.
    pub async fn addon_catalog(
        &self,
        media_type: &str,
        catalog_id: &str,
    ) -> Result<Vec<AddonCatalogEntry>, Error> {
        self.addon_catalog_with_extra(media_type, catalog_id, &[])
            .await
    }

    pub async fn addon_catalog_with_extra(
        &self,
        media_type: &str,
        catalog_id: &str,
        extra: &[(&str, &str)],
    ) -> Result<Vec<AddonCatalogEntry>, Error> {
        validate_path_value("addon catalog type", media_type)?;
        validate_path_value("addon catalog id", catalog_id)?;
        let endpoint = self.resource_endpoint("addon_catalog", media_type, catalog_id, extra)?;
        let response: AddonCatalogResponse = self
            .get(endpoint, "addon catalog", MAX_ADDON_CATALOG_BYTES)
            .await?;
        ensure_collection_limit(
            "addon catalog response",
            response.addons.len(),
            MAX_ADDON_CATALOG_ITEMS,
        )?;
        for addon in &response.addons {
            addon.validate()?;
        }
        Ok(response.addons)
    }

    fn resource_endpoint(
        &self,
        resource: &str,
        media_type: &str,
        id: &str,
        extra: &[(&str, &str)],
    ) -> Result<reqwest::Url, Error> {
        if extra.len() > 16 {
            return Err(Error::Stremio(format!(
                "{resource} request has too many extra parameters"
            )));
        }
        for (name, value) in extra {
            validate_string("resource extra name", name, 64, true)?;
            validate_string("resource extra value", value, 512, false)?;
        }
        if extra.is_empty() {
            let file = format!("{id}.json");
            return self.endpoint(&[resource, media_type, &file]);
        }
        let mut encoded = reqwest::Url::parse("https://extras.invalid/")
            .map_err(|e| Error::Stremio(format!("failed to encode addon extras: {e}")))?;
        encoded
            .query_pairs_mut()
            .extend_pairs(extra.iter().copied());
        let extra_file = format!("{}.json", encoded.query().unwrap_or_default());
        self.endpoint(&[resource, media_type, id, &extra_file])
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
        max_bytes: usize,
    ) -> Result<T, Error> {
        validate_public_https_url(&url, "addon endpoint URL")?;
        let response = self.client.get(url).send().await.map_err(|e| {
            Error::Stremio(format!("{resource} request failed: {}", e.without_url()))
        })?;
        let status = response.status();
        if !status.is_success() {
            let body = read_error_excerpt(response).await;
            return Err(Error::Stremio(format!(
                "{resource} returned HTTP {status}: {body}"
            )));
        }
        let body = read_bounded_body(response, max_bytes, resource).await?;
        serde_json::from_slice(&body)
            .map_err(|e| Error::Stremio(format!("failed to parse {resource} response: {e}")))
    }
}

fn same_origin(left: &reqwest::Url, right: &reqwest::Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

/// Cinemeta's official public endpoint delegates catalogs to this exact
/// sibling host. This narrow exception preserves the bundled catalog without
/// granting arbitrary third-party addons a general cross-origin redirect.
fn is_official_cinemeta_redirect(initial: &reqwest::Url, next: &reqwest::Url) -> bool {
    initial.scheme() == "https"
        && initial.host_str() == Some("v3-cinemeta.strem.io")
        && initial.port_or_known_default() == Some(443)
        && next.scheme() == "https"
        && next.host_str() == Some("cinemeta-catalogs.strem.io")
        && next.port_or_known_default() == Some(443)
}

/// Parses `url` and validates it as a public, credential-free HTTPS target.
///
/// This is the same policy [`AddonClient`] applies to every manifest,
/// resource, redirect, and direct-stream URL it touches, exposed for other
/// modules (namely [`crate::deeplink`]) that need to judge whether an
/// externally supplied URL is safe to open or fetch without duplicating the
/// public/local-address and credential checks.
pub fn validate_public_https_url_str(url: &str, label: &str) -> Result<(), Error> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| Error::Stremio(format!("{label}: invalid URL: {e}")))?;
    validate_public_https_url(&parsed, label)
}

fn validate_public_https_url(url: &reqwest::Url, label: &str) -> Result<(), Error> {
    if url.as_str().len() > MAX_URL_LENGTH {
        return Err(Error::Stremio(format!("{label} is too long")));
    }
    if url.scheme() != "https" {
        return Err(Error::Stremio(format!("{label} must use HTTPS")));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Error::Stremio(format!(
            "{label} must not contain credentials"
        )));
    }
    let host = url
        .host_str()
        .ok_or_else(|| Error::Stremio(format!("{label} must have a host")))?;
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_non_public_ip(ip) {
            return Err(Error::Stremio(format!(
                "{label} must not target a local or non-public address"
            )));
        }
    } else if is_obviously_local_name(host) {
        return Err(Error::Stremio(format!(
            "{label} must not target a local hostname"
        )));
    }
    Ok(())
}

fn is_obviously_local_name(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    !host.contains('.')
        || host == "localhost"
        || [
            ".localhost",
            ".local",
            ".localdomain",
            ".lan",
            ".home",
            ".internal",
            ".intranet",
            ".test",
            ".invalid",
        ]
        .iter()
        .any(|suffix| host.ends_with(suffix))
}

fn is_non_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_non_public_ipv4(ip),
        IpAddr::V6(ip) => is_non_public_ipv6(ip),
    }
}

fn is_non_public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_documentation()
        || a == 0
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0)
        || (a == 198 && (18..=19).contains(&b))
        || a >= 240
}

fn is_non_public_ipv6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] & 0xffc0) == 0xfec0
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || ip.to_ipv4().is_some_and(is_non_public_ipv4)
}

async fn read_bounded_body(
    mut response: reqwest::Response,
    max_bytes: usize,
    resource: &str,
) -> Result<Vec<u8>, Error> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(Error::Stremio(format!(
            "{resource} response is larger than {max_bytes} bytes"
        )));
    }

    let mut body = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or_default()
            .min(max_bytes as u64) as usize,
    );
    while let Some(chunk) = response.chunk().await.map_err(|e| {
        Error::Stremio(format!(
            "failed to read {resource} response: {}",
            e.without_url()
        ))
    })? {
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(Error::Stremio(format!(
                "{resource} response is larger than {max_bytes} bytes"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn read_error_excerpt(mut response: reqwest::Response) -> String {
    let mut body = Vec::with_capacity(MAX_ERROR_EXCERPT_BYTES);
    let mut truncated = false;
    loop {
        let Ok(next) = response.chunk().await else {
            return "response body could not be read".into();
        };
        let Some(chunk) = next else {
            break;
        };
        let remaining = MAX_ERROR_EXCERPT_BYTES.saturating_sub(body.len());
        if chunk.len() > remaining {
            body.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        body.extend_from_slice(&chunk);
        if body.len() == MAX_ERROR_EXCERPT_BYTES {
            truncated = true;
            break;
        }
    }
    let mut excerpt = String::from_utf8_lossy(&body)
        .chars()
        .map(|character| {
            if character.is_control() && !matches!(character, '\n' | '\t') {
                '�'
            } else {
                character
            }
        })
        .collect::<String>();
    if truncated {
        excerpt.push('…');
    }
    if excerpt.is_empty() {
        "empty response body".into()
    } else {
        excerpt
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
    pub logo: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub resources: Vec<Resource>,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default, rename = "idPrefixes")]
    pub id_prefixes: Vec<String>,
    #[serde(default)]
    pub catalogs: Vec<Catalog>,
    #[serde(default, rename = "addonCatalogs")]
    pub addon_catalogs: Vec<Catalog>,
    #[serde(default, rename = "contactEmail")]
    pub contact_email: Option<String>,
    #[serde(default, rename = "behaviorHints")]
    pub behavior_hints: ManifestBehaviorHints,
    #[serde(default)]
    pub config: Vec<ManifestConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ManifestBehaviorHints {
    pub adult: bool,
    pub p2p: bool,
    pub configurable: bool,
    #[serde(rename = "configurationRequired")]
    pub configuration_required: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestConfig {
    pub key: String,
    #[serde(rename = "type")]
    pub config_type: ManifestConfigType,
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub required: bool,
}

/// Input controls used by the official SDK plus aliases found in existing
/// addon manifests. Unknown values remain representable for forward
/// compatibility instead of making an otherwise usable manifest unparseable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ManifestConfigType {
    Text,
    String,
    Number,
    Password,
    Checkbox,
    Boolean,
    Select,
    #[serde(other)]
    Unknown,
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
    #[serde(default, rename = "optionsLimit")]
    pub options_limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PosterShape {
    Poster,
    Square,
    Landscape,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaLink {
    pub name: String,
    pub category: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyTrailer {
    pub source: String,
    #[serde(rename = "type")]
    pub trailer_type: String,
}

/// Trailer payloads exist in both the legacy YouTube `{source, type}` form
/// and the newer full Stream form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MetaTrailer {
    Legacy(LegacyTrailer),
    Stream(Box<Stream>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MetaBehaviorHints {
    #[serde(rename = "defaultVideoId")]
    pub default_video_id: Option<String>,
    #[serde(rename = "hasScheduledVideos")]
    pub has_scheduled_videos: Option<bool>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaPreview {
    pub id: String,
    #[serde(rename = "type")]
    pub media_type: String,
    pub name: String,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default, rename = "posterShape")]
    pub poster_shape: Option<PosterShape>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub genre: Vec<String>,
    #[serde(default, rename = "releaseInfo")]
    pub release_info: Option<String>,
    #[serde(default, rename = "imdbRating")]
    pub imdb_rating: Option<String>,
    #[serde(default)]
    pub director: Vec<String>,
    #[serde(default)]
    pub writer: Vec<String>,
    #[serde(default)]
    pub cast: Vec<String>,
    #[serde(default)]
    pub links: Vec<MetaLink>,
    #[serde(default)]
    pub trailers: Vec<MetaTrailer>,
    #[serde(default, rename = "trailerStreams")]
    pub trailer_streams: Vec<Stream>,
    #[serde(default)]
    pub released: Option<String>,
    #[serde(default)]
    pub year: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub awards: Option<String>,
    #[serde(default, rename = "behaviorHints")]
    pub behavior_hints: MetaBehaviorHints,
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
    #[serde(default, rename = "posterShape")]
    pub poster_shape: Option<PosterShape>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub cast: Vec<String>,
    #[serde(default)]
    pub director: Vec<String>,
    #[serde(default)]
    pub writer: Vec<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    /// Legacy Cinemeta alias retained alongside `genres`.
    #[serde(default)]
    pub genre: Vec<String>,
    #[serde(default, rename = "releaseInfo")]
    pub release_info: Option<String>,
    #[serde(default, rename = "imdbRating")]
    pub imdb_rating: Option<String>,
    #[serde(default)]
    pub released: Option<String>,
    #[serde(default)]
    pub year: Option<String>,
    #[serde(default, rename = "dvdRelease")]
    pub dvd_release: Option<String>,
    #[serde(default)]
    pub trailers: Vec<MetaTrailer>,
    #[serde(default, rename = "trailerStreams")]
    pub trailer_streams: Vec<Stream>,
    #[serde(default)]
    pub links: Vec<MetaLink>,
    #[serde(default)]
    pub videos: Vec<Video>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub awards: Option<String>,
    #[serde(default)]
    pub website: Option<String>,
    #[serde(default, rename = "behaviorHints")]
    pub behavior_hints: MetaBehaviorHints,
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
    #[serde(default)]
    pub streams: Vec<Stream>,
    #[serde(default)]
    pub available: Option<bool>,
    #[serde(default)]
    pub trailers: Vec<Stream>,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stream {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default, rename = "infoHash")]
    pub info_hash: Option<String>,
    #[serde(default, rename = "fileIdx")]
    pub file_idx: Option<u32>,
    #[serde(default, rename = "fileMustInclude")]
    pub file_must_include: Option<String>,
    #[serde(default, rename = "ytId")]
    pub yt_id: Option<String>,
    #[serde(default, rename = "nzbUrl")]
    pub nzb_url: Option<String>,
    #[serde(default)]
    pub servers: Vec<String>,
    #[serde(default, rename = "rarUrls")]
    pub rar_urls: Vec<ArchiveSource>,
    #[serde(default, rename = "zipUrls")]
    pub zip_urls: Vec<ArchiveSource>,
    #[serde(default, rename = "7zipUrls")]
    pub seven_zip_urls: Vec<ArchiveSource>,
    #[serde(default, rename = "tgzUrls")]
    pub tgz_urls: Vec<ArchiveSource>,
    #[serde(default, rename = "tarUrls")]
    pub tar_urls: Vec<ArchiveSource>,
    #[serde(default, rename = "externalUrl")]
    pub external_url: Option<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub subtitles: Vec<Subtitle>,
    #[serde(default, rename = "behaviorHints")]
    pub behavior_hints: StreamBehaviorHints,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveSource {
    pub url: String,
    #[serde(default)]
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamBehaviorHints {
    #[serde(rename = "countryWhitelist")]
    pub country_whitelist: Vec<String>,
    #[serde(rename = "notWebReady")]
    pub not_web_ready: bool,
    #[serde(rename = "bingeGroup")]
    pub binge_group: Option<String>,
    /// Deprecated upstream alias kept for existing addons.
    pub group: Option<String>,
    #[serde(rename = "proxyHeaders")]
    pub proxy_headers: Option<StreamProxyHeaders>,
    pub filename: Option<String>,
    #[serde(rename = "videoHash")]
    pub video_hash: Option<String>,
    #[serde(rename = "videoSize")]
    pub video_size: Option<u64>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamProxyHeaders {
    pub request: HashMap<String, String>,
    pub response: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subtitle {
    pub id: String,
    pub url: String,
    pub lang: String,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AddonCatalogEntry {
    #[serde(rename = "transportName")]
    pub transport_name: String,
    #[serde(rename = "transportUrl")]
    pub transport_url: String,
    pub manifest: Manifest,
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

/// Every source form documented by the Stremio addon protocol. This describes
/// addon output; it does not imply that ANKAI has a resolver for the transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamTarget<'a> {
    DirectUrl(&'a str),
    YouTube(&'a str),
    BitTorrent {
        info_hash: &'a str,
        file_idx: Option<u32>,
        sources: &'a [String],
    },
    Nzb {
        url: &'a str,
        servers: &'a [String],
    },
    Rar(&'a [ArchiveSource]),
    Zip(&'a [ArchiveSource]),
    SevenZip(&'a [ArchiveSource]),
    Tgz(&'a [ArchiveSource]),
    Tar(&'a [ArchiveSource]),
    ExternalUrl(&'a str),
}

impl Stream {
    /// Returns the single transport target described by this stream.
    pub fn target(&self) -> Result<StreamTarget<'_>, Error> {
        let mut targets = Vec::with_capacity(10);
        if let Some(value) = self.url.as_deref() {
            targets.push(StreamTarget::DirectUrl(value));
        }
        if let Some(value) = self.yt_id.as_deref() {
            targets.push(StreamTarget::YouTube(value));
        }
        if let Some(value) = self.info_hash.as_deref() {
            targets.push(StreamTarget::BitTorrent {
                info_hash: value,
                file_idx: self.file_idx,
                sources: &self.sources,
            });
        }
        if let Some(value) = self.nzb_url.as_deref() {
            targets.push(StreamTarget::Nzb {
                url: value,
                servers: &self.servers,
            });
        }
        if !self.rar_urls.is_empty() {
            targets.push(StreamTarget::Rar(&self.rar_urls));
        }
        if !self.zip_urls.is_empty() {
            targets.push(StreamTarget::Zip(&self.zip_urls));
        }
        if !self.seven_zip_urls.is_empty() {
            targets.push(StreamTarget::SevenZip(&self.seven_zip_urls));
        }
        if !self.tgz_urls.is_empty() {
            targets.push(StreamTarget::Tgz(&self.tgz_urls));
        }
        if !self.tar_urls.is_empty() {
            targets.push(StreamTarget::Tar(&self.tar_urls));
        }
        if let Some(value) = self.external_url.as_deref() {
            targets.push(StreamTarget::ExternalUrl(value));
        }
        match targets.as_slice() {
            [target] => Ok(*target),
            [] => Err(Error::Stremio(
                "stream has no supported protocol target field".into(),
            )),
            _ => Err(Error::Stremio(
                "stream declares more than one protocol target field".into(),
            )),
        }
    }

    /// Returns a public HTTPS libmpv URL or a BitTorrent descriptor for a resolver.
    ///
    /// Addon output is untrusted. Direct playback deliberately accepts only
    /// credential-free, public HTTPS URLs; local paths and libmpv pseudo
    /// protocols (`file:`, `data:`, `concat:`, and similar) never cross this
    /// resolver boundary.
    pub fn source(&self) -> Result<StreamSource<'_>, Error> {
        match self.target()? {
            StreamTarget::DirectUrl(url) => {
                let parsed = reqwest::Url::parse(url)
                    .map_err(|e| Error::Stremio(format!("invalid direct stream URL: {e}")))?;
                validate_public_https_url(&parsed, "direct stream URL")?;
                Ok(StreamSource::Direct(url))
            }
            StreamTarget::BitTorrent {
                info_hash,
                file_idx,
                sources,
            } => {
                validate_string("stream infoHash", info_hash, 128, true)?;
                Ok(StreamSource::BitTorrent {
                    info_hash,
                    file_idx,
                    sources,
                })
            }
            StreamTarget::YouTube(_) => Err(Error::Stremio(
                "YouTube stream requires a YouTube resolver".into(),
            )),
            StreamTarget::Nzb { .. } => Err(Error::Stremio(
                "NZB stream requires a Usenet resolver".into(),
            )),
            StreamTarget::Rar(_)
            | StreamTarget::Zip(_)
            | StreamTarget::SevenZip(_)
            | StreamTarget::Tgz(_)
            | StreamTarget::Tar(_) => Err(Error::Stremio(
                "archive stream requires an archive resolver".into(),
            )),
            StreamTarget::ExternalUrl(_) => Err(Error::Stremio(
                "external stream must be opened outside the in-app player".into(),
            )),
        }
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

/// One fetched catalog page plus the addon's own optional cache hints.
///
/// Stremio's addon protocol lets a catalog response carry `cacheMaxAge`,
/// `staleRevalidate` and `staleError` fields (seconds) beside `metas`,
/// mirroring HTTP's `Cache-Control: max-age`/`stale-while-revalidate`/
/// `stale-if-error`. ANKAI reads these JSON-level hints rather than raw HTTP
/// response headers: the addon SDK documents and tests against this form,
/// and every resource already flows through this crate's single bounded
/// `get<T>` helper, which would otherwise need to thread response headers
/// through every caller for a case only the catalog cache uses.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogPage {
    pub metas: Vec<MetaPreview>,
    pub cache_max_age: Option<u64>,
    pub stale_revalidate: Option<u64>,
    pub stale_error: Option<u64>,
}

#[derive(Deserialize)]
struct CatalogResponse {
    #[serde(default)]
    metas: Vec<MetaPreview>,
    #[serde(default, rename = "cacheMaxAge")]
    cache_max_age: Option<u64>,
    #[serde(default, rename = "staleRevalidate")]
    stale_revalidate: Option<u64>,
    #[serde(default, rename = "staleError")]
    stale_error: Option<u64>,
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
#[derive(Deserialize)]
struct SubtitlesResponse {
    #[serde(default)]
    subtitles: Vec<Subtitle>,
}
#[derive(Deserialize)]
struct AddonCatalogResponse {
    #[serde(default)]
    addons: Vec<AddonCatalogEntry>,
}

impl Manifest {
    /// Returns whether this manifest should receive a resource request.
    ///
    /// String resources inherit the manifest-level `types` and `idPrefixes`.
    /// Descriptor resources use their own filters; an omitted descriptor
    /// `idPrefixes` matches every ID. Catalog routing is intentionally exposed
    /// separately through [`Manifest::catalog_for_request`] because catalog IDs
    /// are matched against manifest catalog declarations, not content-ID
    /// prefixes.
    pub fn supports_resource(&self, resource: &str, media_type: &str, id: &str) -> bool {
        if resource == "catalog" {
            return self
                .catalogs
                .iter()
                .any(|catalog| catalog.media_type == media_type && catalog.id == id);
        }
        if resource == "addon_catalog" {
            return self
                .addon_catalogs
                .iter()
                .any(|catalog| catalog.media_type == media_type && catalog.id == id);
        }
        self.resources.iter().any(|candidate| match candidate {
            Resource::Name(name) if name == resource => {
                matches_type(&self.types, media_type) && matches_prefix(&self.id_prefixes, id)
            }
            Resource::Descriptor {
                name,
                types,
                id_prefixes,
            } if name == resource => {
                matches_type(types, media_type) && matches_prefix(id_prefixes, id)
            }
            _ => false,
        })
    }

    /// Resolves and validates a content-catalog request, including required
    /// extras, declared option values and `optionsLimit`.
    pub fn catalog_for_request<'a>(
        &'a self,
        media_type: &str,
        catalog_id: &str,
        extra: &[(&str, &str)],
    ) -> Result<Option<&'a Catalog>, Error> {
        let Some(catalog) = self
            .catalogs
            .iter()
            .find(|catalog| catalog.media_type == media_type && catalog.id == catalog_id)
        else {
            return Ok(None);
        };
        catalog.validate_request_extra(extra)?;
        Ok(Some(catalog))
    }

    pub fn addon_catalog_for_request<'a>(
        &'a self,
        media_type: &str,
        catalog_id: &str,
        extra: &[(&str, &str)],
    ) -> Result<Option<&'a Catalog>, Error> {
        let Some(catalog) = self
            .addon_catalogs
            .iter()
            .find(|catalog| catalog.media_type == media_type && catalog.id == catalog_id)
        else {
            return Ok(None);
        };
        catalog.validate_request_extra(extra)?;
        Ok(Some(catalog))
    }

    fn validate(&self) -> Result<(), Error> {
        validate_string("manifest id", &self.id, MAX_SHORT_STRING, true)?;
        validate_string("manifest name", &self.name, MAX_SHORT_STRING, true)?;
        validate_string("manifest version", &self.version, 128, true)?;
        validate_optional_string(
            "manifest description",
            self.description.as_deref(),
            MAX_LONG_STRING,
        )?;
        validate_optional_string("manifest logo", self.logo.as_deref(), MAX_URL_LENGTH)?;
        validate_optional_string(
            "manifest background",
            self.background.as_deref(),
            MAX_URL_LENGTH,
        )?;
        ensure_collection_limit("manifest resources", self.resources.len(), 64)?;
        ensure_collection_limit("manifest types", self.types.len(), 64)?;
        ensure_collection_limit(
            "manifest id prefixes",
            self.id_prefixes.len(),
            MAX_COLLECTION_ITEMS,
        )?;
        ensure_collection_limit("manifest catalogs", self.catalogs.len(), 128)?;
        ensure_collection_limit("manifest addon catalogs", self.addon_catalogs.len(), 128)?;
        ensure_collection_limit("manifest config", self.config.len(), 128)?;
        validate_string_collection("manifest types", &self.types, 64)?;
        validate_string_collection("manifest id prefixes", &self.id_prefixes, MAX_SHORT_STRING)?;
        validate_optional_string(
            "manifest contact email",
            self.contact_email.as_deref(),
            MAX_SHORT_STRING,
        )?;
        for resource in &self.resources {
            match resource {
                Resource::Name(name) => {
                    validate_string("manifest resource", name, 128, true)?;
                }
                Resource::Descriptor {
                    name,
                    types,
                    id_prefixes,
                } => {
                    validate_string("manifest resource name", name, 128, true)?;
                    ensure_collection_limit("manifest resource types", types.len(), 64)?;
                    ensure_collection_limit(
                        "manifest resource id prefixes",
                        id_prefixes.len(),
                        128,
                    )?;
                    validate_string_collection("manifest resource types", types, 64)?;
                    validate_string_collection(
                        "manifest resource id prefixes",
                        id_prefixes,
                        MAX_SHORT_STRING,
                    )?;
                }
            }
        }
        for catalog in &self.catalogs {
            catalog.validate()?;
        }
        for catalog in &self.addon_catalogs {
            catalog.validate()?;
        }
        let mut config_keys = HashSet::new();
        for config in &self.config {
            config.validate()?;
            if !config_keys.insert(config.key.as_str()) {
                return Err(Error::Stremio(format!(
                    "manifest config key {:?} is duplicated",
                    config.key
                )));
            }
        }
        Ok(())
    }
}

impl Catalog {
    pub fn validate_request_extra(&self, extra: &[(&str, &str)]) -> Result<(), Error> {
        let mut supplied = HashMap::<&str, Vec<&str>>::new();
        for (name, value) in extra {
            let Some(declaration) = self.extra.iter().find(|item| item.name == *name) else {
                return Err(Error::Stremio(format!(
                    "catalog {:?} does not support extra parameter {name:?}",
                    self.id
                )));
            };
            if !declaration.options.is_empty()
                && !declaration.options.iter().any(|option| option == value)
            {
                return Err(Error::Stremio(format!(
                    "catalog extra {name:?} does not allow value {value:?}"
                )));
            }
            supplied.entry(name).or_default().push(value);
        }
        for declaration in &self.extra {
            let count = supplied
                .get(declaration.name.as_str())
                .map_or(0, |values| values.len());
            if declaration.is_required && count == 0 {
                return Err(Error::Stremio(format!(
                    "catalog extra {:?} is required",
                    declaration.name
                )));
            }
            let options_limit = declaration.options_limit.unwrap_or(1);
            if count > options_limit {
                return Err(Error::Stremio(format!(
                    "catalog extra {:?} accepts at most {options_limit} value(s)",
                    declaration.name
                )));
            }
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), Error> {
        validate_string("catalog type", &self.media_type, 64, true)?;
        validate_string("catalog id", &self.id, MAX_SHORT_STRING, true)?;
        validate_optional_string("catalog name", self.name.as_deref(), MAX_SHORT_STRING)?;
        ensure_collection_limit("catalog genres", self.genres.len(), MAX_COLLECTION_ITEMS)?;
        ensure_collection_limit("catalog extras", self.extra.len(), 32)?;
        validate_string_collection("catalog genres", &self.genres, MAX_SHORT_STRING)?;
        let mut extra_names = HashSet::new();
        for extra in &self.extra {
            validate_string("catalog extra name", &extra.name, 64, true)?;
            ensure_collection_limit("catalog extra options", extra.options.len(), 256)?;
            validate_string_collection("catalog extra options", &extra.options, 512)?;
            if extra.options_limit == Some(0) {
                return Err(Error::Stremio(
                    "catalog extra optionsLimit must be greater than zero".into(),
                ));
            }
            if !extra_names.insert(extra.name.as_str()) {
                return Err(Error::Stremio(format!(
                    "catalog extra {:?} is duplicated",
                    extra.name
                )));
            }
        }
        Ok(())
    }
}

impl ManifestConfig {
    fn validate(&self) -> Result<(), Error> {
        validate_string("manifest config key", &self.key, 128, true)?;
        validate_optional_string(
            "manifest config title",
            self.title.as_deref(),
            MAX_SHORT_STRING,
        )?;
        ensure_collection_limit("manifest config options", self.options.len(), 256)?;
        validate_string_collection("manifest config options", &self.options, 512)?;
        if let Some(default) = &self.default {
            let mut nodes = 0;
            validate_extra_value(default, 0, &mut nodes)?;
        }
        Ok(())
    }
}

fn matches_type(types: &[String], media_type: &str) -> bool {
    types.is_empty() || types.iter().any(|candidate| candidate == media_type)
}

fn matches_prefix(prefixes: &[String], id: &str) -> bool {
    prefixes.is_empty() || prefixes.iter().any(|prefix| id.starts_with(prefix))
}

impl MetaPreview {
    fn validate(&self) -> Result<(), Error> {
        validate_string("catalog item id", &self.id, MAX_SHORT_STRING, true)?;
        validate_string("catalog item type", &self.media_type, 64, true)?;
        validate_string("catalog item name", &self.name, MAX_SHORT_STRING, true)?;
        validate_optional_string(
            "catalog item poster",
            self.poster.as_deref(),
            MAX_URL_LENGTH,
        )?;
        validate_optional_string(
            "catalog item background",
            self.background.as_deref(),
            MAX_URL_LENGTH,
        )?;
        validate_optional_string("catalog item logo", self.logo.as_deref(), MAX_URL_LENGTH)?;
        validate_optional_string(
            "catalog item description",
            self.description.as_deref(),
            MAX_LONG_STRING,
        )?;
        validate_optional_string(
            "catalog item release info",
            self.release_info.as_deref(),
            MAX_SHORT_STRING,
        )?;
        ensure_collection_limit(
            "catalog item genres",
            self.genres.len(),
            MAX_COLLECTION_ITEMS,
        )?;
        ensure_collection_limit(
            "catalog item legacy genres",
            self.genre.len(),
            MAX_COLLECTION_ITEMS,
        )?;
        ensure_collection_limit(
            "catalog item directors",
            self.director.len(),
            MAX_COLLECTION_ITEMS,
        )?;
        ensure_collection_limit(
            "catalog item writers",
            self.writer.len(),
            MAX_COLLECTION_ITEMS,
        )?;
        ensure_collection_limit("catalog item cast", self.cast.len(), MAX_COLLECTION_ITEMS)?;
        ensure_collection_limit("catalog item links", self.links.len(), MAX_COLLECTION_ITEMS)?;
        ensure_collection_limit("catalog item trailers", self.trailers.len(), 64)?;
        ensure_collection_limit(
            "catalog item trailer streams",
            self.trailer_streams.len(),
            64,
        )?;
        validate_string_collection("catalog item genres", &self.genres, MAX_SHORT_STRING)?;
        validate_string_collection("catalog item legacy genres", &self.genre, MAX_SHORT_STRING)?;
        validate_string_collection("catalog item directors", &self.director, MAX_SHORT_STRING)?;
        validate_string_collection("catalog item writers", &self.writer, MAX_SHORT_STRING)?;
        validate_string_collection("catalog item cast", &self.cast, MAX_SHORT_STRING)?;
        validate_optional_string("catalog item IMDb rating", self.imdb_rating.as_deref(), 32)?;
        for link in &self.links {
            link.validate()?;
        }
        for trailer in &self.trailers {
            trailer.validate()?;
        }
        for stream in &self.trailer_streams {
            stream.validate()?;
        }
        validate_optional_string("catalog item release date", self.released.as_deref(), 128)?;
        validate_optional_string("catalog item year", self.year.as_deref(), 64)?;
        validate_optional_string("catalog item runtime", self.runtime.as_deref(), 128)?;
        validate_optional_string("catalog item country", self.country.as_deref(), 128)?;
        validate_optional_string(
            "catalog item awards",
            self.awards.as_deref(),
            MAX_LONG_STRING,
        )?;
        validate_optional_string(
            "catalog item default video id",
            self.behavior_hints.default_video_id.as_deref(),
            MAX_SHORT_STRING,
        )?;
        validate_extra_map(
            "catalog item behavior hint extras",
            &self.behavior_hints.extra,
        )?;
        validate_extra_map("catalog item extras", &self.extra)
    }
}

impl Meta {
    fn validate(&self) -> Result<(), Error> {
        validate_string("meta id", &self.id, MAX_SHORT_STRING, true)?;
        validate_string("meta type", &self.media_type, 64, true)?;
        validate_string("meta name", &self.name, MAX_SHORT_STRING, true)?;
        validate_optional_string("meta poster", self.poster.as_deref(), MAX_URL_LENGTH)?;
        validate_optional_string(
            "meta background",
            self.background.as_deref(),
            MAX_URL_LENGTH,
        )?;
        validate_optional_string("meta logo", self.logo.as_deref(), MAX_URL_LENGTH)?;
        validate_optional_string(
            "meta description",
            self.description.as_deref(),
            MAX_LONG_STRING,
        )?;
        ensure_collection_limit("meta cast", self.cast.len(), MAX_COLLECTION_ITEMS)?;
        ensure_collection_limit("meta directors", self.director.len(), MAX_COLLECTION_ITEMS)?;
        ensure_collection_limit("meta writers", self.writer.len(), MAX_COLLECTION_ITEMS)?;
        ensure_collection_limit("meta genres", self.genres.len(), MAX_COLLECTION_ITEMS)?;
        ensure_collection_limit("meta legacy genres", self.genre.len(), MAX_COLLECTION_ITEMS)?;
        ensure_collection_limit("meta links", self.links.len(), MAX_COLLECTION_ITEMS)?;
        ensure_collection_limit("meta trailers", self.trailers.len(), 64)?;
        ensure_collection_limit("meta trailer streams", self.trailer_streams.len(), 64)?;
        ensure_collection_limit("meta videos", self.videos.len(), MAX_VIDEO_ITEMS)?;
        validate_string_collection("meta cast", &self.cast, MAX_SHORT_STRING)?;
        validate_string_collection("meta directors", &self.director, MAX_SHORT_STRING)?;
        validate_string_collection("meta writers", &self.writer, MAX_SHORT_STRING)?;
        validate_string_collection("meta genres", &self.genres, MAX_SHORT_STRING)?;
        validate_string_collection("meta legacy genres", &self.genre, MAX_SHORT_STRING)?;
        validate_optional_string(
            "meta release info",
            self.release_info.as_deref(),
            MAX_SHORT_STRING,
        )?;
        validate_optional_string("meta IMDb rating", self.imdb_rating.as_deref(), 32)?;
        validate_optional_string("meta release date", self.released.as_deref(), 128)?;
        validate_optional_string("meta year", self.year.as_deref(), 64)?;
        validate_optional_string("meta DVD release date", self.dvd_release.as_deref(), 128)?;
        validate_optional_string("meta runtime", self.runtime.as_deref(), 128)?;
        validate_optional_string("meta language", self.language.as_deref(), 128)?;
        validate_optional_string("meta country", self.country.as_deref(), 128)?;
        validate_optional_string("meta awards", self.awards.as_deref(), MAX_LONG_STRING)?;
        validate_optional_string("meta website", self.website.as_deref(), MAX_URL_LENGTH)?;
        validate_optional_string(
            "meta default video id",
            self.behavior_hints.default_video_id.as_deref(),
            MAX_SHORT_STRING,
        )?;
        validate_extra_map("meta behavior hint extras", &self.behavior_hints.extra)?;
        for link in &self.links {
            link.validate()?;
        }
        for trailer in &self.trailers {
            trailer.validate()?;
        }
        for stream in &self.trailer_streams {
            stream.validate()?;
        }
        for video in &self.videos {
            video.validate()?;
        }
        validate_extra_map("meta extras", &self.extra)
    }
}

impl Video {
    fn validate(&self) -> Result<(), Error> {
        validate_string("video id", &self.id, MAX_SHORT_STRING, true)?;
        validate_optional_string("video title", self.title.as_deref(), MAX_SHORT_STRING)?;
        validate_optional_string("video release date", self.released.as_deref(), 128)?;
        validate_optional_string("video thumbnail", self.thumbnail.as_deref(), MAX_URL_LENGTH)?;
        validate_optional_string("video overview", self.overview.as_deref(), MAX_LONG_STRING)?;
        ensure_collection_limit("video streams", self.streams.len(), MAX_STREAM_ITEMS)?;
        ensure_collection_limit("video trailers", self.trailers.len(), 64)?;
        for stream in self.streams.iter().chain(&self.trailers) {
            stream.validate()?;
        }
        validate_extra_map("video extras", &self.extra)
    }
}

impl Stream {
    fn validate(&self) -> Result<(), Error> {
        self.target()?;
        validate_optional_required_string("stream URL", self.url.as_deref(), MAX_URL_LENGTH)?;
        validate_optional_required_string("stream infoHash", self.info_hash.as_deref(), 128)?;
        validate_optional_required_string(
            "stream file matching expression",
            self.file_must_include.as_deref(),
            MAX_SHORT_STRING,
        )?;
        validate_optional_required_string("stream YouTube id", self.yt_id.as_deref(), 256)?;
        validate_optional_required_string(
            "stream NZB URL",
            self.nzb_url.as_deref(),
            MAX_URL_LENGTH,
        )?;
        validate_optional_required_string(
            "stream external URL",
            self.external_url.as_deref(),
            MAX_URL_LENGTH,
        )?;
        ensure_collection_limit("stream sources", self.sources.len(), 64)?;
        validate_string_collection("stream sources", &self.sources, MAX_URL_LENGTH)?;
        ensure_collection_limit("stream NNTP servers", self.servers.len(), 32)?;
        validate_string_collection("stream NNTP servers", &self.servers, MAX_URL_LENGTH)?;
        for (label, sources) in [
            ("stream RAR sources", &self.rar_urls),
            ("stream ZIP sources", &self.zip_urls),
            ("stream 7zip sources", &self.seven_zip_urls),
            ("stream TGZ sources", &self.tgz_urls),
            ("stream TAR sources", &self.tar_urls),
        ] {
            ensure_collection_limit(label, sources.len(), 64)?;
            for source in sources {
                source.validate(label)?;
            }
        }
        validate_optional_string("stream name", self.name.as_deref(), MAX_SHORT_STRING)?;
        validate_optional_string("stream title", self.title.as_deref(), MAX_LONG_STRING)?;
        validate_optional_string(
            "stream description",
            self.description.as_deref(),
            MAX_LONG_STRING,
        )?;
        ensure_collection_limit("stream subtitles", self.subtitles.len(), MAX_SUBTITLE_ITEMS)?;
        for subtitle in &self.subtitles {
            subtitle.validate()?;
        }
        self.behavior_hints.validate()?;
        validate_extra_map("stream extras", &self.extra)
    }
}

impl MetaLink {
    fn validate(&self) -> Result<(), Error> {
        validate_string("meta link name", &self.name, MAX_SHORT_STRING, true)?;
        validate_string("meta link category", &self.category, 128, true)?;
        validate_string("meta link URL", &self.url, MAX_URL_LENGTH, true)
    }
}

impl LegacyTrailer {
    fn validate(&self) -> Result<(), Error> {
        validate_string("trailer source", &self.source, 256, true)?;
        validate_string("trailer type", &self.trailer_type, 64, true)
    }
}

impl MetaTrailer {
    fn validate(&self) -> Result<(), Error> {
        match self {
            Self::Legacy(trailer) => trailer.validate(),
            Self::Stream(stream) => stream.validate(),
        }
    }
}

impl ArchiveSource {
    fn validate(&self, label: &str) -> Result<(), Error> {
        validate_string(label, &self.url, MAX_URL_LENGTH, true)
    }
}

impl StreamBehaviorHints {
    fn validate(&self) -> Result<(), Error> {
        ensure_collection_limit(
            "stream country whitelist",
            self.country_whitelist.len(),
            256,
        )?;
        validate_string_collection("stream country whitelist", &self.country_whitelist, 16)?;
        validate_optional_string(
            "stream binge group",
            self.binge_group.as_deref(),
            MAX_SHORT_STRING,
        )?;
        validate_optional_string(
            "stream legacy group",
            self.group.as_deref(),
            MAX_SHORT_STRING,
        )?;
        validate_optional_string(
            "stream filename",
            self.filename.as_deref(),
            MAX_SHORT_STRING,
        )?;
        validate_optional_string("stream video hash", self.video_hash.as_deref(), 256)?;
        if let Some(headers) = &self.proxy_headers {
            if !self.not_web_ready {
                return Err(Error::Stremio(
                    "stream proxyHeaders requires behaviorHints.notWebReady=true".into(),
                ));
            }
            headers.validate()?;
        }
        validate_extra_map("stream behavior hint extras", &self.extra)
    }
}

impl StreamProxyHeaders {
    fn validate(&self) -> Result<(), Error> {
        for (label, headers) in [
            ("stream proxy request headers", &self.request),
            ("stream proxy response headers", &self.response),
        ] {
            ensure_collection_limit(label, headers.len(), 64)?;
            for (name, value) in headers {
                validate_string("stream proxy header name", name, 256, true)?;
                validate_string("stream proxy header value", value, MAX_SHORT_STRING, false)?;
            }
        }
        Ok(())
    }
}

impl Subtitle {
    fn validate(&self) -> Result<(), Error> {
        validate_string("subtitle id", &self.id, MAX_SHORT_STRING, true)?;
        validate_string("subtitle URL", &self.url, MAX_URL_LENGTH, true)?;
        validate_string("subtitle language", &self.lang, 128, true)?;
        validate_extra_map("subtitle extras", &self.extra)
    }

    /// Applies ANKAI's remote media policy before handing a subtitle URL to a
    /// player. Legacy local-streaming-server URLs remain representable in the
    /// model but are not trusted as player input.
    pub fn playback_url(&self) -> Result<&str, Error> {
        let parsed = reqwest::Url::parse(&self.url)
            .map_err(|error| Error::Stremio(format!("invalid subtitle URL: {error}")))?;
        validate_public_https_url(&parsed, "subtitle URL")?;
        Ok(&self.url)
    }
}

impl AddonCatalogEntry {
    fn validate(&self) -> Result<(), Error> {
        validate_string(
            "addon catalog transport name",
            &self.transport_name,
            64,
            true,
        )?;
        if self.transport_name != "http" {
            return Err(Error::Stremio(format!(
                "unsupported addon catalog transport name {:?}",
                self.transport_name
            )));
        }
        validate_string(
            "addon catalog transport URL",
            &self.transport_url,
            MAX_URL_LENGTH,
            true,
        )?;
        self.manifest.validate()
    }

    /// Returns the installability outcome without following or rewriting the
    /// repository-provided transport URL.
    pub fn transport_outcome(&self) -> Result<AddonTransportOutcome, Error> {
        AddonClient::transport_outcome(&self.transport_url)
    }
}

fn validate_path_value(label: &str, value: &str) -> Result<(), Error> {
    validate_string(label, value, MAX_SHORT_STRING, true)
}

fn validate_optional_string(
    label: &str,
    value: Option<&str>,
    max_bytes: usize,
) -> Result<(), Error> {
    if let Some(value) = value {
        validate_string(label, value, max_bytes, false)?;
    }
    Ok(())
}

fn validate_optional_required_string(
    label: &str,
    value: Option<&str>,
    max_bytes: usize,
) -> Result<(), Error> {
    if let Some(value) = value {
        validate_string(label, value, max_bytes, true)?;
    }
    Ok(())
}

fn validate_string(
    label: &str,
    value: &str,
    max_bytes: usize,
    required: bool,
) -> Result<(), Error> {
    if required && value.trim().is_empty() {
        return Err(Error::Stremio(format!("{label} must not be empty")));
    }
    if value.len() > max_bytes {
        return Err(Error::Stremio(format!(
            "{label} is longer than {max_bytes} bytes"
        )));
    }
    Ok(())
}

fn ensure_collection_limit(label: &str, count: usize, max: usize) -> Result<(), Error> {
    if count > max {
        return Err(Error::Stremio(format!(
            "{label} contains too many items (maximum {max})"
        )));
    }
    Ok(())
}

fn validate_string_collection(
    label: &str,
    values: &[String],
    max_string_bytes: usize,
) -> Result<(), Error> {
    for value in values {
        validate_string(label, value, max_string_bytes, false)?;
    }
    Ok(())
}

fn validate_extra_map(label: &str, extras: &HashMap<String, Value>) -> Result<(), Error> {
    ensure_collection_limit(label, extras.len(), MAX_EXTRA_FIELDS)?;
    let mut nodes = 0;
    for (key, value) in extras {
        validate_string("extra field name", key, 256, true)?;
        validate_extra_value(value, 0, &mut nodes)?;
    }
    Ok(())
}

fn validate_extra_value(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), Error> {
    if depth > MAX_EXTRA_DEPTH {
        return Err(Error::Stremio(
            "addon extra data is nested too deeply".into(),
        ));
    }
    *nodes = nodes.saturating_add(1);
    if *nodes > MAX_EXTRA_NODES {
        return Err(Error::Stremio(
            "addon extra data contains too many values".into(),
        ));
    }
    match value {
        Value::String(value) => {
            validate_string("addon extra string", value, MAX_LONG_STRING, false)?;
        }
        Value::Array(values) => {
            ensure_collection_limit("addon extra array", values.len(), MAX_COLLECTION_ITEMS)?;
            for value in values {
                validate_extra_value(value, depth + 1, nodes)?;
            }
        }
        Value::Object(values) => {
            ensure_collection_limit("addon extra object", values.len(), MAX_EXTRA_FIELDS)?;
            for (key, value) in values {
                validate_string("addon extra field name", key, 256, true)?;
                validate_extra_value(value, depth + 1, nodes)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
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
        assert_eq!(
            client.manifest_url().unwrap(),
            "https://example.com/c29tZS1jb25maWc=/manifest.json"
        );
    }

    #[test]
    fn addon_urls_must_be_public_credential_free_https() {
        for rejected in [
            "http://addons.example.com/manifest.json",
            "https://user:secret@addons.example.com/manifest.json",
            "https://localhost/manifest.json",
            "https://router/manifest.json",
            "https://media.internal/manifest.json",
            "https://127.0.0.1/manifest.json",
            "https://10.2.3.4/manifest.json",
            "https://169.254.169.254/manifest.json",
            "https://[::1]/manifest.json",
            "https://[fe80::1]/manifest.json",
        ] {
            assert!(AddonClient::new(rejected).is_err(), "accepted {rejected}");
        }
        assert!(AddonClient::new("https://addons.example.com/manifest.json").is_ok());
    }

    #[test]
    fn redirect_origin_comparison_includes_scheme_host_and_port() {
        let original = reqwest::Url::parse("https://addons.example.com/a").unwrap();
        assert!(same_origin(
            &original,
            &reqwest::Url::parse("https://addons.example.com/b").unwrap()
        ));
        assert!(!same_origin(
            &original,
            &reqwest::Url::parse("https://cdn.example.com/b").unwrap()
        ));
        assert!(!same_origin(
            &original,
            &reqwest::Url::parse("https://addons.example.com:444/b").unwrap()
        ));
        assert!(!same_origin(
            &original,
            &reqwest::Url::parse("http://addons.example.com/b").unwrap()
        ));
    }

    #[test]
    fn only_the_exact_official_cinemeta_cross_origin_redirect_is_allowed() {
        let official =
            reqwest::Url::parse("https://v3-cinemeta.strem.io/catalog/movie/top.json").unwrap();
        let catalog =
            reqwest::Url::parse("https://cinemeta-catalogs.strem.io/top/catalog/movie/top.json")
                .unwrap();
        assert!(is_official_cinemeta_redirect(&official, &catalog));
        assert!(!is_official_cinemeta_redirect(
            &reqwest::Url::parse("https://untrusted.example/catalog/movie/top.json").unwrap(),
            &catalog
        ));
        assert!(!is_official_cinemeta_redirect(
            &official,
            &reqwest::Url::parse("https://cinemeta-catalogs.strem.io.evil.example/top").unwrap()
        ));
        assert!(!is_official_cinemeta_redirect(
            &official,
            &reqwest::Url::parse("http://cinemeta-catalogs.strem.io/top").unwrap()
        ));
    }

    #[test]
    fn dns_address_filter_rejects_non_public_ranges() {
        for rejected in [
            "0.0.0.0",
            "10.0.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.1.1",
            "172.16.0.1",
            "192.168.0.1",
            "224.0.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
        ] {
            assert!(
                is_non_public_ip(rejected.parse().unwrap()),
                "accepted {rejected}"
            );
        }
        assert!(!is_non_public_ip("1.1.1.1".parse().unwrap()));
        assert!(!is_non_public_ip("2606:4700:4700::1111".parse().unwrap()));
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

    #[test]
    fn direct_stream_policy_rejects_native_and_local_sources() {
        for rejected in [
            "file:///etc/passwd",
            "data:text/plain,hello",
            "concat:https://a|https://b",
            "/Users/example/movie.mp4",
            "https://user:secret@video.example/movie.mp4",
            "https://127.0.0.1/movie.mp4",
            "http://video.example/movie.mp4",
            "ftp://video.example/movie.mp4",
        ] {
            let stream = Stream {
                url: Some(rejected.into()),
                ..Stream::default()
            };
            assert!(stream.source().is_err(), "accepted {rejected}");
        }
    }

    #[test]
    fn deserialized_collection_and_extra_limits_are_enforced() {
        let mut preview: MetaPreview = serde_json::from_value(serde_json::json!({
            "id": "tt1",
            "type": "movie",
            "name": "Example"
        }))
        .unwrap();
        preview.genres =
            std::iter::repeat_n("genre".to_owned(), MAX_COLLECTION_ITEMS + 1).collect();
        assert!(preview.validate().is_err());

        preview.genres.clear();
        preview.extra.insert(
            "oversized".into(),
            Value::String("x".repeat(MAX_LONG_STRING + 1)),
        );
        assert!(preview.validate().is_err());
    }

    #[test]
    fn transport_classification_explains_legacy_and_distributed_urls() {
        assert_eq!(
            AddonClient::transport_outcome("https://addons.example/manifest.json").unwrap(),
            AddonTransportOutcome::SupportedHttps
        );
        assert_eq!(
            AddonClient::transport_outcome("http://addons.example/manifest.json").unwrap(),
            AddonTransportOutcome::UnsupportedInsecureHttp
        );
        assert_eq!(
            AddonClient::transport_outcome("https://legacy.example/stremio/v1/").unwrap(),
            AddonTransportOutcome::UnsupportedLegacyV1
        );
        assert_eq!(
            AddonClient::transport_outcome("https://legacy.example/stremio/v2").unwrap(),
            AddonTransportOutcome::UnsupportedLegacyV2
        );
        assert_eq!(
            AddonClient::transport_outcome("ipfs://bafy/example/manifest.json").unwrap(),
            AddonTransportOutcome::UnsupportedIpfs
        );
        assert_eq!(
            AddonClient::transport_outcome("ipns://addons.example/manifest.json").unwrap(),
            AddonTransportOutcome::UnsupportedIpns
        );
        assert!(AddonClient::new("ipfs://bafy/example/manifest.json")
            .unwrap_err()
            .to_string()
            .contains("trusted gateway"));
    }

    #[test]
    fn resource_filters_use_global_and_descriptor_capabilities() {
        let manifest: Manifest = serde_json::from_value(serde_json::json!({
            "id": "org.example.capabilities",
            "name": "Capabilities",
            "version": "1.0.0",
            "resources": [
                "meta",
                {"name": "stream", "types": ["series"], "idPrefixes": ["kitsu:"]},
                {"name": "subtitles", "types": ["movie"]},
                "catalog",
                "addon_catalog"
            ],
            "types": ["movie", "series"],
            "idPrefixes": ["tt"],
            "catalogs": [{"type": "movie", "id": "popular", "name": "Popular"}],
            "addonCatalogs": [{"type": "addon", "id": "community", "name": "Community"}]
        }))
        .unwrap();

        assert!(manifest.supports_resource("meta", "movie", "tt123"));
        assert!(!manifest.supports_resource("meta", "movie", "kitsu:1"));
        assert!(manifest.supports_resource("stream", "series", "kitsu:1"));
        assert!(!manifest.supports_resource("stream", "movie", "kitsu:1"));
        assert!(manifest.supports_resource("subtitles", "movie", "any-id"));
        assert!(manifest.supports_resource("catalog", "movie", "popular"));
        assert!(manifest.supports_resource("addon_catalog", "addon", "community"));
    }

    #[test]
    fn catalog_extras_enforce_required_options_and_options_limit() {
        let catalog: Catalog = serde_json::from_value(serde_json::json!({
            "type": "series",
            "id": "anime",
            "name": "Anime",
            "extra": [
                {"name": "genre", "isRequired": true, "options": ["Action", "Drama"], "optionsLimit": 2},
                {"name": "skip", "options": ["0", "100"]}
            ]
        }))
        .unwrap();
        catalog.validate().unwrap();
        catalog
            .validate_request_extra(&[("genre", "Action"), ("genre", "Drama")])
            .unwrap();
        assert!(catalog.validate_request_extra(&[]).is_err());
        assert!(catalog
            .validate_request_extra(&[("genre", "Comedy")])
            .is_err());
        assert!(catalog
            .validate_request_extra(&[("genre", "Action"), ("genre", "Drama"), ("genre", "Action")])
            .is_err());
        assert!(catalog
            .validate_request_extra(&[("genre", "Action"), ("unknown", "x")])
            .is_err());
    }

    #[test]
    fn manifest_models_configuration_behavior_and_full_meta_fields() {
        let manifest: Manifest = serde_json::from_value(serde_json::json!({
            "id": "org.example.full",
            "name": "Full",
            "version": "1.0.0",
            "description": "Full protocol",
            "resources": ["catalog", "meta", "stream", "subtitles", "addon_catalog"],
            "types": ["movie"],
            "idPrefixes": ["tt"],
            "catalogs": [],
            "addonCatalogs": [{"type": "addon", "id": "official", "name": "Official"}],
            "contactEmail": "support@example.com",
            "behaviorHints": {
                "adult": true,
                "p2p": true,
                "configurable": true,
                "configurationRequired": true
            },
            "config": [{
                "key": "token",
                "type": "password",
                "title": "Token",
                "required": true
            }]
        }))
        .unwrap();
        manifest.validate().unwrap();
        assert!(manifest.behavior_hints.p2p);
        assert_eq!(manifest.config[0].config_type, ManifestConfigType::Password);

        let meta: Meta = serde_json::from_value(serde_json::json!({
            "id": "tt123",
            "type": "movie",
            "name": "Protocol Film",
            "poster": "https://images.example/poster.jpg",
            "posterShape": "landscape",
            "background": "https://images.example/background.jpg",
            "logo": "https://images.example/logo.png",
            "releaseInfo": "2026",
            "imdbRating": "8.1",
            "released": "2026-01-01T00:00:00.000Z",
            "director": ["A. Director"],
            "cast": ["A. Performer"],
            "trailers": [{"source": "youtube-id", "type": "Trailer"}],
            "links": [{"name": "A. Director", "category": "director", "url": "stremio:///search?search=A"}],
            "runtime": "120m",
            "language": "English",
            "country": "GB",
            "awards": "One award",
            "website": "https://film.example",
            "behaviorHints": {"defaultVideoId": "tt123"}
        }))
        .unwrap();
        meta.validate().unwrap();
        assert_eq!(meta.poster_shape, Some(PosterShape::Landscape));
        assert_eq!(
            meta.behavior_hints.default_video_id.as_deref(),
            Some("tt123")
        );
    }

    #[test]
    fn every_documented_stream_target_is_typed() {
        let cases = [
            (
                serde_json::json!({"url": "https://video.example/movie.mp4"}),
                "direct",
            ),
            (serde_json::json!({"ytId": "abc123"}), "youtube"),
            (
                serde_json::json!({"infoHash": "abc123", "fileIdx": 1}),
                "torrent",
            ),
            (
                serde_json::json!({"nzbUrl": "https://usenet.example/movie.nzb", "servers": ["nntps://user:pass@news.example:563/4"]}),
                "nzb",
            ),
            (
                serde_json::json!({"rarUrls": [{"url": "https://archive.example/movie.rar", "bytes": 100}]}),
                "rar",
            ),
            (
                serde_json::json!({"zipUrls": [{"url": "https://archive.example/movie.zip"}]}),
                "zip",
            ),
            (
                serde_json::json!({"7zipUrls": [{"url": "https://archive.example/movie.7z"}]}),
                "7zip",
            ),
            (
                serde_json::json!({"tgzUrls": [{"url": "https://archive.example/movie.tgz"}]}),
                "tgz",
            ),
            (
                serde_json::json!({"tarUrls": [{"url": "https://archive.example/movie.tar"}]}),
                "tar",
            ),
            (
                serde_json::json!({"externalUrl": "https://provider.example/watch/1"}),
                "external",
            ),
        ];
        for (value, expected) in cases {
            let stream: Stream = serde_json::from_value(value).unwrap();
            stream.validate().unwrap();
            let actual = match stream.target().unwrap() {
                StreamTarget::DirectUrl(_) => "direct",
                StreamTarget::YouTube(_) => "youtube",
                StreamTarget::BitTorrent { .. } => "torrent",
                StreamTarget::Nzb { .. } => "nzb",
                StreamTarget::Rar(_) => "rar",
                StreamTarget::Zip(_) => "zip",
                StreamTarget::SevenZip(_) => "7zip",
                StreamTarget::Tgz(_) => "tgz",
                StreamTarget::Tar(_) => "tar",
                StreamTarget::ExternalUrl(_) => "external",
            };
            assert_eq!(actual, expected);
        }

        for empty_target in [
            serde_json::json!({"url": ""}),
            serde_json::json!({"ytId": ""}),
            serde_json::json!({"infoHash": ""}),
            serde_json::json!({"nzbUrl": ""}),
            serde_json::json!({"externalUrl": ""}),
        ] {
            let stream: Stream = serde_json::from_value(empty_target).unwrap();
            assert!(stream.validate().is_err());
        }
    }

    #[test]
    fn stream_behavior_hints_subtitles_and_addon_catalogs_are_bounded() {
        let stream: Stream = serde_json::from_value(serde_json::json!({
            "url": "https://video.example/movie.mkv",
            "description": "4K HDR",
            "subtitles": [{"id": "english", "url": "https://subs.example/en.vtt", "lang": "eng"}],
            "behaviorHints": {
                "countryWhitelist": ["gbr"],
                "notWebReady": true,
                "bingeGroup": "provider-4k",
                "proxyHeaders": {"request": {"User-Agent": "Ankai"}},
                "filename": "movie.mkv",
                "videoHash": "abcdef",
                "videoSize": 123456
            }
        }))
        .unwrap();
        stream.validate().unwrap();
        assert_eq!(
            stream.subtitles[0].playback_url().unwrap(),
            "https://subs.example/en.vtt"
        );

        let response: AddonCatalogResponse = serde_json::from_value(serde_json::json!({
            "addons": [{
                "transportName": "http",
                "transportUrl": "https://addon.example/manifest.json",
                "manifest": {
                    "id": "org.example.child",
                    "name": "Child",
                    "version": "1.0.0",
                    "resources": [],
                    "types": [],
                    "catalogs": []
                }
            }]
        }))
        .unwrap();
        response.addons[0].validate().unwrap();
        assert_eq!(
            response.addons[0].transport_outcome().unwrap(),
            AddonTransportOutcome::SupportedHttps
        );
    }
}
