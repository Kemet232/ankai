//! Bounded remote image fetch and decode for real cover art.
//!
//! Slint 1.17 does not fetch remote `Image` sources itself, so this module
//! performs HTTPS I/O on the Tokio runtime, decodes on a blocking worker, and
//! creates the final `slint::Image` on the UI thread. Untrusted addon artwork
//! is constrained at every stage: public credential-free HTTPS URLs only,
//! same-origin redirects, request/body/decode budgets, downsampling, an
//! in-memory byte-bounded LRU, and one shared in-flight fetch per URL. Nothing
//! is written to disk.
//!
//! DNS names are resolved inside reqwest's connector through the core public-
//! address resolver, which filters every returned address before connection;
//! environment proxies are disabled so they cannot resolve the target again.
//! A transparent system/VPN route that maps an otherwise public address into a
//! private network remains outside what an application URL policy can detect.

use std::cell::RefCell;
use std::collections::{hash_map::Entry, HashMap, VecDeque};
use std::io::Cursor;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::OnceLock;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_REDIRECTS: usize = 3;
const MAX_URL_LENGTH: usize = 8 * 1024;
const MAX_IMAGE_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_SOURCE_DIMENSION: u32 = 8_192;
const MAX_SOURCE_PIXELS: u64 = 32_000_000;
const MAX_DECODE_ALLOC_BYTES: u64 = 160 * 1024 * 1024;
const MAX_RENDER_EDGE: u32 = 1_600;
const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_CACHE_ITEMS: usize = 128;
const MAX_PENDING_URLS: usize = 512;
const MAX_CALLBACKS_PER_URL: usize = 512;

type ApplyImage = Box<dyn FnOnce(&crate::AppWindow, slint::Image)>;

struct PendingApply {
    app_weak: slint::Weak<crate::AppWindow>,
    apply: ApplyImage,
}

struct CachedImage {
    image: slint::Image,
    decoded_bytes: usize,
}

struct ImageLru {
    entries: HashMap<String, CachedImage>,
    order: VecDeque<String>,
    decoded_bytes: usize,
    max_bytes: usize,
    max_items: usize,
}

impl ImageLru {
    fn new(max_bytes: usize, max_items: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            decoded_bytes: 0,
            max_bytes,
            max_items,
        }
    }

    fn get(&mut self, url: &str) -> Option<slint::Image> {
        let image = self.entries.get(url)?.image.clone();
        self.touch(url);
        Some(image)
    }

    fn insert(&mut self, url: String, image: slint::Image, decoded_bytes: usize) {
        if decoded_bytes == 0 || decoded_bytes > self.max_bytes || self.max_items == 0 {
            return;
        }
        if let Some(previous) = self.entries.remove(&url) {
            self.decoded_bytes = self.decoded_bytes.saturating_sub(previous.decoded_bytes);
            self.order.retain(|key| key != &url);
        }
        while self.decoded_bytes.saturating_add(decoded_bytes) > self.max_bytes
            || self.entries.len() >= self.max_items
        {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.decoded_bytes = self.decoded_bytes.saturating_sub(removed.decoded_bytes);
            }
        }
        self.decoded_bytes = self.decoded_bytes.saturating_add(decoded_bytes);
        self.order.push_back(url.clone());
        self.entries.insert(
            url,
            CachedImage {
                image,
                decoded_bytes,
            },
        );
    }

    fn touch(&mut self, url: &str) {
        self.order.retain(|key| key != url);
        self.order.push_back(url.to_owned());
    }
}

struct ImageServiceState {
    cache: ImageLru,
    pending: HashMap<String, Vec<PendingApply>>,
}

impl Default for ImageServiceState {
    fn default() -> Self {
        Self {
            cache: ImageLru::new(MAX_CACHE_BYTES, MAX_CACHE_ITEMS),
            pending: HashMap::new(),
        }
    }
}

thread_local! {
    /// Slint images and window handles are UI-thread-only. Network/decode work
    /// crosses threads as plain RGBA bytes; cache and callbacks stay here.
    static IMAGE_SERVICE: RefCell<ImageServiceState> = RefCell::new(ImageServiceState::default());
}

/// Plain `Send` pixel data passed from a worker to Slint's UI thread.
struct DecodedImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl DecodedImage {
    fn decoded_bytes(&self) -> usize {
        self.rgba.len()
    }

    fn into_slint_image(self) -> slint::Image {
        let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
            &self.rgba,
            self.width,
            self.height,
        );
        slint::Image::from_rgba8(buffer)
    }
}

fn image_client() -> Result<&'static reqwest::Client, String> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(REQUEST_TIMEOUT)
                .no_proxy()
                .dns_resolver(ankai_core::stremio::PublicDnsResolver)
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    let Some(initial) = attempt.previous().first() else {
                        return attempt.error("redirect has no initial request URL");
                    };
                    if attempt.previous().len() > MAX_REDIRECTS {
                        return attempt.error("too many image redirects");
                    }
                    if !same_origin(initial, attempt.url()) {
                        return attempt.error("image redirect changed origin");
                    }
                    if validate_image_url(attempt.url()).is_err() {
                        return attempt.error("image redirect URL is not public HTTPS");
                    }
                    attempt.follow()
                }))
                .user_agent(concat!("ankai/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|error| format!("failed to build image HTTP client: {error}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn parse_image_url(value: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(value).map_err(|_| "image URL is invalid".to_owned())?;
    validate_image_url(&url)?;
    url.set_fragment(None);
    Ok(url)
}

fn validate_image_url(url: &reqwest::Url) -> Result<(), String> {
    if url.as_str().len() > MAX_URL_LENGTH {
        return Err("image URL is too long".into());
    }
    if url.scheme() != "https" {
        return Err("image URL must use HTTPS".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("image URL must not contain credentials".into());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "image URL must have a host".to_owned())?;
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_non_public_ip(ip) {
            return Err("image URL must not target a local or non-public address".into());
        }
    } else if is_obviously_local_name(host) {
        return Err("image URL must not target a local hostname".into());
    }
    Ok(())
}

fn same_origin(left: &reqwest::Url, right: &reqwest::Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
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

async fn fetch_image(url: reqwest::Url) -> Result<DecodedImage, String> {
    validate_image_url(&url)?;
    let mut response = image_client()?
        .get(url)
        .send()
        .await
        .map_err(|error| format!("cover image request failed: {}", error.without_url()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("cover image returned HTTP {status}"));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_IMAGE_BODY_BYTES as u64)
    {
        return Err(format!(
            "cover image is larger than {MAX_IMAGE_BODY_BYTES} bytes"
        ));
    }

    let mut bytes = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or_default()
            .min(MAX_IMAGE_BODY_BYTES as u64) as usize,
    );
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        format!(
            "failed to read cover image response: {}",
            error.without_url()
        )
    })? {
        append_bounded(&mut bytes, &chunk, MAX_IMAGE_BODY_BYTES)?;
    }

    tokio::task::spawn_blocking(move || decode_image(&bytes))
        .await
        .map_err(|error| format!("cover image decode worker failed: {error}"))?
}

fn append_bounded(target: &mut Vec<u8>, chunk: &[u8], max_bytes: usize) -> Result<(), String> {
    if target.len().saturating_add(chunk.len()) > max_bytes {
        return Err(format!("cover image is larger than {max_bytes} bytes"));
    }
    target.extend_from_slice(chunk);
    Ok(())
}

fn decode_image(bytes: &[u8]) -> Result<DecodedImage, String> {
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| format!("failed to identify cover image: {error}"))?;
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| format!("failed to read cover image dimensions: {error}"))?;
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if width == 0 || height == 0 {
        return Err("cover image has zero dimensions".into());
    }
    if width > MAX_SOURCE_DIMENSION || height > MAX_SOURCE_DIMENSION || pixels > MAX_SOURCE_PIXELS {
        return Err(format!(
            "cover image dimensions exceed {MAX_SOURCE_DIMENSION}px/{MAX_SOURCE_PIXELS} pixels"
        ));
    }

    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_DIMENSION);
    limits.max_image_height = Some(MAX_SOURCE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_ALLOC_BYTES);
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| format!("failed to identify cover image: {error}"))?;
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|error| format!("failed to decode cover image: {error}"))?;
    let decoded = if width > MAX_RENDER_EDGE || height > MAX_RENDER_EDGE {
        decoded.thumbnail(MAX_RENDER_EDGE, MAX_RENDER_EDGE)
    } else {
        decoded
    };
    let rgba = decoded.to_rgba8();
    Ok(DecodedImage {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

/// Loads a cover without blocking the UI thread.
///
/// Cached images are applied synchronously. Concurrent requests for the same
/// canonical URL share one network/decode operation and each queued callback
/// is invoked once on success. Failures leave the caller's existing fallback
/// untouched and are logged without echoing potentially sensitive URL paths or
/// query strings.
pub fn load_cover_image<F>(
    url: String,
    handle: tokio::runtime::Handle,
    app_weak: slint::Weak<crate::AppWindow>,
    apply: F,
) where
    F: FnOnce(&crate::AppWindow, slint::Image) + Send + 'static,
{
    let url = match parse_image_url(&url) {
        Ok(url) => url,
        Err(error) => {
            eprintln!("ankai-client: {error}");
            return;
        }
    };
    let cache_key = url.as_str().to_owned();
    if let Some(image) = IMAGE_SERVICE.with(|service| service.borrow_mut().cache.get(&cache_key)) {
        if let Some(app) = app_weak.upgrade() {
            apply(&app, image);
        }
        return;
    }

    let pending = PendingApply {
        app_weak,
        apply: Box::new(apply),
    };
    let should_fetch = IMAGE_SERVICE.with(|service| {
        let mut service = service.borrow_mut();
        if !service.pending.contains_key(&cache_key) && service.pending.len() >= MAX_PENDING_URLS {
            return None;
        }
        match service.pending.entry(cache_key.clone()) {
            Entry::Occupied(mut request) => {
                if request.get().len() >= MAX_CALLBACKS_PER_URL {
                    return None;
                }
                request.get_mut().push(pending);
                Some(false)
            }
            Entry::Vacant(request) => {
                request.insert(vec![pending]);
                Some(true)
            }
        }
    });
    match should_fetch {
        Some(true) => {}
        Some(false) => return,
        None => {
            eprintln!("ankai-client: too many pending cover image requests");
            return;
        }
    }

    handle.spawn(async move {
        let result = fetch_image(url).await;
        let _ = slint::invoke_from_event_loop(move || match result {
            Ok(decoded) => {
                let decoded_bytes = decoded.decoded_bytes();
                let image = decoded.into_slint_image();
                let callbacks = IMAGE_SERVICE.with(|service| {
                    let mut service = service.borrow_mut();
                    service
                        .cache
                        .insert(cache_key.clone(), image.clone(), decoded_bytes);
                    service.pending.remove(&cache_key).unwrap_or_default()
                });
                for callback in callbacks {
                    if let Some(app) = callback.app_weak.upgrade() {
                        (callback.apply)(&app, image.clone());
                    }
                }
            }
            Err(error) => {
                IMAGE_SERVICE.with(|service| {
                    service.borrow_mut().pending.remove(&cache_key);
                });
                eprintln!("ankai-client: {error}");
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use image::ImageEncoder;

    use super::*;

    #[test]
    fn image_urls_must_be_public_credential_free_https() {
        for rejected in [
            "http://images.example.com/cover.jpg",
            "file:///tmp/cover.jpg",
            "data:image/png;base64,abc",
            "https://user:secret@images.example.com/cover.jpg",
            "https://localhost/cover.jpg",
            "https://nas.local/cover.jpg",
            "https://127.0.0.1/cover.jpg",
            "https://10.0.0.1/cover.jpg",
            "https://169.254.169.254/cover.jpg",
            "https://[::1]/cover.jpg",
        ] {
            assert!(parse_image_url(rejected).is_err(), "accepted {rejected}");
        }
        assert!(parse_image_url("https://images.example.com/cover.jpg").is_ok());
        assert_eq!(
            parse_image_url("https://images.example.com/cover.jpg#ignored")
                .unwrap()
                .as_str(),
            "https://images.example.com/cover.jpg"
        );
    }

    #[test]
    fn body_append_enforces_streamed_cap() {
        let mut bytes = vec![1, 2];
        assert!(append_bounded(&mut bytes, &[3, 4], 4).is_ok());
        assert!(append_bounded(&mut bytes, &[5], 4).is_err());
        assert_eq!(bytes, vec![1, 2, 3, 4]);
    }

    #[test]
    fn large_valid_images_are_downsampled() {
        let pixels = vec![128; 2_000 * 1_000 * 4];
        let mut encoded = Vec::new();
        image::codecs::png::PngEncoder::new(&mut encoded)
            .write_image(&pixels, 2_000, 1_000, image::ExtendedColorType::Rgba8)
            .unwrap();
        let decoded = decode_image(&encoded).unwrap();
        assert_eq!((decoded.width, decoded.height), (1_600, 800));
        assert_eq!(decoded.rgba.len(), 1_600 * 800 * 4);
    }

    #[test]
    fn source_dimension_limit_is_checked_before_full_decode() {
        let pixels = vec![0; (MAX_SOURCE_DIMENSION as usize + 1) * 4];
        let mut encoded = Vec::new();
        image::codecs::png::PngEncoder::new(&mut encoded)
            .write_image(
                &pixels,
                MAX_SOURCE_DIMENSION + 1,
                1,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        assert!(decode_image(&encoded).is_err());
    }

    fn test_image() -> slint::Image {
        let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(1, 1);
        slint::Image::from_rgba8(buffer)
    }

    #[test]
    fn cache_is_byte_bounded_and_recent_reads_refresh_lru_order() {
        let mut cache = ImageLru::new(8, 8);
        cache.insert("a".into(), test_image(), 4);
        cache.insert("b".into(), test_image(), 4);
        assert!(cache.get("a").is_some());
        cache.insert("c".into(), test_image(), 4);
        assert!(cache.get("a").is_some());
        assert!(cache.get("b").is_none());
        assert!(cache.get("c").is_some());
        assert!(cache.decoded_bytes <= 8);
    }
}
