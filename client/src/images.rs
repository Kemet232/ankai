//! Remote image fetch + decode for real anime cover art, entirely in
//! memory — no file ever written to disk.
//!
//! `core::anime`'s `AnimeSummary`/`WatchlistEntry` already carry a real
//! HTTPS `cover_image_url` pointing at AniList's own CDN, but Slint 1.17's
//! `Image` element can only load from a local file path or compile-time
//! embedded data — it has no builtin for "load this network URL." This
//! module is the missing piece: given a URL, fetch the real bytes (via
//! `reqwest`, same HTTP client `core::anime`/`core::mal_forums` already
//! use), decode them (via the `image` crate — already a real dependency of
//! this exact `slint` version, since `slint::Image::load_from_path` uses it
//! internally; see `client/Cargo.toml`'s doc comment), and hand Slint a
//! `slint::Image` built straight from the decoded RGBA pixel buffer.
//!
//! **Deliberately no disk cache.** An earlier draft of this module wrote
//! decoded covers to a file under the app's data directory (mirroring how
//! `main.rs::open_local_db` locates its SQLCipher file) so repeat views
//! wouldn't re-fetch. The human explicitly vetoed that mid-task: no
//! persistent local files for this, full stop. What's here instead is a
//! process-lifetime in-memory cache ([`cached`]/[`cache_insert`]) — a
//! `HashMap<String, slint::Image>` behind a thread-local, so a cover
//! fetched once during this run is reused for the rest of the run without
//! re-hitting AniList's CDN, but nothing survives past the process exiting.
//!
//! **Why a thread-local instead of a plain `Rc<RefCell<..>>` passed
//! around:** `slint::Image` (like `slint::Weak`'s target types generally)
//! is not `Send`, so it can't be captured into the `tokio`-spawned async
//! blocks [`load_cover_image`] hands to the P2P runtime — the same
//! constraint `main.rs`'s `MessagingHandles`/`MESSAGING_HANDLES` doc
//! comment already explains for `Rc<Db>`/`Rc<AnkaiMlsProvider>`. This
//! module follows the same established pattern: the cache is only ever
//! touched from inside closures that `slint::invoke_from_event_loop`
//! guarantees run on the UI thread, never from the spawned future itself.
//!
//! **Error handling.** A bad URL, a network failure, a non-2xx HTTP
//! status, or bytes that don't decode as a real image all surface as a
//! plain `Err(String)` — logged by the caller and otherwise ignored. Never
//! panics, never blocks the calling thread (the actual fetch is `async`;
//! [`load_cover_image`] is the sync entry point real UI code calls, and it
//! only ever spawns the work rather than running it inline).

use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// Process-lifetime (not persisted anywhere) cache of already-decoded
    /// cover images, keyed by their source URL. See this module's doc
    /// comment for why this is a thread-local `HashMap` rather than a
    /// shared `Rc<RefCell<..>>` passed into async tasks.
    static COVER_CACHE: RefCell<HashMap<String, slint::Image>> = RefCell::new(HashMap::new());
}

/// Returns a previously-fetched-and-decoded image for `url`, if this
/// process has already loaded it since startup. Must only be called from
/// the UI thread (same rule as every other thread-local in this crate).
fn cached(url: &str) -> Option<slint::Image> {
    COVER_CACHE.with(|cache| cache.borrow().get(url).cloned())
}

/// Records a successfully decoded image under `url` for future [`cached`]
/// lookups. Must only be called from the UI thread.
fn cache_insert(url: String, image: slint::Image) {
    COVER_CACHE.with(|cache| {
        cache.borrow_mut().insert(url, image);
    });
}

/// Decoded RGBA pixel data for a fetched cover image. Plain `Send` data
/// (unlike `slint::Image`, which wraps a non-`Send` render-backend handle —
/// see this module's doc comment) so it can cross the tokio-task-to-UI-thread
/// boundary in [`load_cover_image`]; the actual `slint::Image` /
/// `SharedPixelBuffer` gets built from this only once back on the UI thread,
/// inside the `slint::invoke_from_event_loop` closure.
struct DecodedImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl DecodedImage {
    fn into_slint_image(self) -> slint::Image {
        let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
            &self.rgba,
            self.width,
            self.height,
        );
        slint::Image::from_rgba8(buffer)
    }
}

/// Fetches `url`'s real bytes over HTTP and decodes them into raw RGBA
/// pixels, entirely in memory (no temp file, no disk cache — see this
/// module's doc comment). Returns [`DecodedImage`] rather than a
/// `slint::Image` directly since this runs on a background tokio task and
/// `slint::Image` isn't `Send` — see [`load_cover_image`] for where the
/// actual `slint::Image` gets built, back on the UI thread.
///
/// Real async I/O (this function does not block); the caller is expected
/// to run it on a spawned task, not synchronously on the UI thread — see
/// [`load_cover_image`] for the actual integration point real UI code
/// should call instead of this directly.
async fn fetch_image(url: &str) -> Result<DecodedImage, String> {
    let response = reqwest::get(url)
        .await
        .map_err(|e| format!("failed to fetch cover image {url}: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("fetching cover image {url} returned HTTP {status}"));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("failed to read cover image bytes from {url}: {e}"))?;

    let decoded = image::load_from_memory(&bytes)
        .map_err(|e| format!("failed to decode cover image from {url}: {e}"))?;

    let rgba = decoded.to_rgba8();
    Ok(DecodedImage {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

/// The real integration point UI code in `main.rs` should call to display a
/// remote cover image: returns immediately (synchronously calling `apply`)
/// if `url` is already in the in-memory cache, otherwise spawns a real
/// fetch+decode on `handle` and calls `apply` once it completes — handed
/// back to the UI thread via `slint::invoke_from_event_loop`, the same
/// cross-thread-handoff shape every other async-to-UI update in `main.rs`
/// already uses (e.g. `spawn_friend_presence_checks`).
///
/// `apply` is only ever invoked on success; a failed fetch/decode is logged
/// to stderr and otherwise left alone — whatever fallback the caller
/// already rendered (see `app.slint`'s `has-cover` fields) simply stays as
/// it was. Never panics, never blocks the calling thread.
pub fn load_cover_image<F>(
    url: String,
    handle: tokio::runtime::Handle,
    app_weak: slint::Weak<crate::AppWindow>,
    apply: F,
) where
    F: FnOnce(&crate::AppWindow, slint::Image) + Send + 'static,
{
    if let Some(image) = cached(&url) {
        if let Some(app) = app_weak.upgrade() {
            apply(&app, image);
        }
        return;
    }

    handle.spawn(async move {
        let result = fetch_image(&url).await;
        let _ = slint::invoke_from_event_loop(move || match result {
            Ok(decoded) => {
                let image = decoded.into_slint_image();
                cache_insert(url, image.clone());
                if let Some(app) = app_weak.upgrade() {
                    apply(&app, image);
                }
            }
            Err(err) => {
                eprintln!("ankai-client: {err}");
            }
        });
    });
}
