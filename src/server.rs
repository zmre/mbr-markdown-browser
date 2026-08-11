use axum::{
    Router,
    body::Body,
    extract::{self, ConnectInfo, DefaultBodyLimit, OriginalUri, State, ws::WebSocketUpgrade},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use std::collections::HashSet;
use std::{net::SocketAddr, path::Path, path::PathBuf, sync::Arc};
use tokio::sync::broadcast;

use crate::config::{RelationType, SortField, TagSource};
use crate::embedded_katex;
use crate::embedded_pico;
use crate::errors::{MbrError, ServerError, TaskPatchError};
use crate::link_grep::InboundLinkCache;
use crate::link_index::{InboundIndex, LinkCache, resolve_outbound_links};
use crate::link_transform::LinkTransformConfig;
use crate::oembed_cache::OembedCache;
use crate::page_context::{self, ModeFlags, PageChrome, UrlMode};
use crate::path_resolver::{PathResolverConfig, ResolvedPath, resolve_request_path};
use crate::repo::MarkdownInfo;
use crate::search::{SearchEngine, SearchQuery, search_other_files};
use crate::sorting::sort_files;
use crate::templates;
#[cfg(feature = "media-metadata")]
use crate::video_metadata_cache::VideoMetadataCache;
#[cfg(feature = "media-metadata")]
use crate::video_transcode_cache::HlsCache;
use crate::{markdown, repo::Repo};
use std::time::Instant;
use tower::ServiceExt;
use tower_http::{compression::CompressionLayer, services::ServeFile, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Default HLS cache size: 200 MB.
#[cfg(feature = "media-metadata")]
const DEFAULT_HLS_CACHE_SIZE: usize = 200 * 1024 * 1024;

/// Maximum time a request waits for an in-progress video-metadata extraction
/// before giving up (degrades a lost wakeup into a retryable `None`).
#[cfg(feature = "media-metadata")]
const METADATA_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Maximum playback-compatibility probes run concurrently for one page. Each
/// probe occupies a blocking thread while ffmpeg parses container headers, so
/// a gallery page with dozens of clips must not saturate the blocking pool.
#[cfg(feature = "media-metadata")]
const MEDIA_COMPAT_PROBE_CONCURRENCY: usize = 4;

/// Outcome of trying to claim an in-flight single-flight slot for a cache key.
enum InflightClaim {
    /// This caller won the slot and must produce the result, then release it.
    Produce(Arc<tokio::sync::Notify>),
    /// Another caller is already producing; await this notify then re-read the
    /// result cache.
    Wait(Arc<tokio::sync::Notify>),
}

/// Claims (or joins) the single-flight slot for `key`.
///
/// Returns [`InflightClaim::Produce`] to exactly one caller (which inserts a
/// fresh `Notify`) and [`InflightClaim::Wait`] to any caller that finds an
/// existing in-progress entry. Modeled on `HlsCache::start_generation`, so N
/// concurrent requests for the same media only trigger one decode. Also used
/// to single-flight inbound-link greps for links.json requests.
///
/// The claim is one atomic `try_insert`: `pin()` is a seize epoch guard
/// (reclamation only, not mutual exclusion), so a `get`-then-`insert` pair is
/// two independent lock-free operations and two callers can both observe a
/// vacant slot and both produce.
///
/// The winner must hold the returned slot in an [`InflightSlot`] guard so
/// cancellation or a panic cannot leave the key claimed forever.
fn claim_inflight(
    inflight: &papaya::HashMap<String, Arc<tokio::sync::Notify>>,
    key: &str,
) -> InflightClaim {
    let notify = Arc::new(tokio::sync::Notify::new());
    match inflight.pin().try_insert(key.to_string(), notify.clone()) {
        Ok(_) => InflightClaim::Produce(notify),
        Err(papaya::OccupiedError { current, .. }) => InflightClaim::Wait(current.clone()),
    }
}

/// Releases a single-flight slot claimed by [`claim_inflight`] when dropped.
///
/// The producer holds this for the whole produce step so *every* exit path
/// frees the slot and wakes waiters: normal return, `?`, panic, and — the case
/// that motivated it — the handler future being dropped at an `await` because
/// the client disconnected. Releasing only on the success path leaves a
/// permanent entry that no TTL or eviction can clear, so every later request
/// for that key waits the full timeout and then 404s until the process
/// restarts.
struct InflightSlot {
    inflight: Arc<papaya::HashMap<String, Arc<tokio::sync::Notify>>>,
    key: String,
    notify: Arc<tokio::sync::Notify>,
}

impl InflightSlot {
    fn new(
        inflight: Arc<papaya::HashMap<String, Arc<tokio::sync::Notify>>>,
        key: String,
        notify: Arc<tokio::sync::Notify>,
    ) -> Self {
        Self {
            inflight,
            key,
            notify,
        }
    }
}

impl Drop for InflightSlot {
    fn drop(&mut self) {
        self.inflight.pin().remove(&self.key);
        // Waiters re-read the result cache after waking; a miss degrades to a
        // retryable `None` instead of a hang.
        self.notify.notify_waiters();
    }
}

/// Releases an `HlsCache` generation slot claimed by `start_generation` when
/// dropped, unless the producer already published a terminal state.
///
/// Same hazard as [`InflightSlot`], with a worse failure mode: an `InProgress`
/// entry has no TTL and `evict_until_freed` refuses to evict non-`Complete`
/// entries, so a slot abandoned by a cancelled request (the handler future is
/// dropped at the `spawn_blocking` await when the client disconnects) or by a
/// panicking transcode task blocks every later request for that playlist or
/// segment for the full `HLS_WAIT_TIMEOUT` and then 404s, for the life of the
/// process. Recording a failure instead lets waiters wake immediately and
/// lets the cache's `FAILED_ENTRY_TTL` re-open the key for a retry.
#[cfg(feature = "media-metadata")]
struct HlsGenerationSlot<'a> {
    cache: &'a HlsCache,
    /// `None` once the producer settled the entry itself (complete or failed).
    key: Option<crate::video_transcode_cache::HlsCacheKey>,
    notify: Arc<tokio::sync::Notify>,
}

#[cfg(feature = "media-metadata")]
impl<'a> HlsGenerationSlot<'a> {
    fn new(
        cache: &'a HlsCache,
        key: crate::video_transcode_cache::HlsCacheKey,
        notify: Arc<tokio::sync::Notify>,
    ) -> Self {
        Self {
            cache,
            key: Some(key),
            notify,
        }
    }

    /// Disarms the guard after the producer stored a terminal state, so the
    /// drop does not overwrite a completed entry with a failure.
    fn settled(&mut self) {
        self.key = None;
    }
}

#[cfg(feature = "media-metadata")]
impl Drop for HlsGenerationSlot<'_> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            tracing::warn!("HLS generation abandoned for {key:?}; releasing the in-flight slot");
            self.cache.fail_generation(
                key,
                &crate::video_transcode::TranscodeError::TranscodeFailed(
                    "generation did not finish (request cancelled or worker panicked)".to_string(),
                ),
            );
            self.notify.notify_waiters();
        }
    }
}

/// Drops every cache derived from markdown file *contents* after a batch of
/// file changes.
///
/// The sibling navigation lists, the directory-listing subdirectories, the
/// serialized site.json body, the outbound `LinkCache` and the inbound link
/// cache are all recomputed from the files themselves, and none of the readers
/// re-checks mtimes, so a change that is not followed by this call is served
/// from the pre-edit snapshot indefinitely (`LinkCache` has no TTL at all).
/// All of them are rebuilt lazily on the next render / listing / site.json /
/// links.json request.
fn invalidate_derived_caches(
    listing_caches: &ListingCaches,
    link_cache: &LinkCache,
    inbound_link_cache: &InboundLinkCache,
) {
    listing_caches.invalidate();
    link_cache.invalidate_all();
    inbound_link_cache.invalidate_all();
}

/// The path-resolution inputs, owned rather than borrowed.
///
/// [`PathResolverConfig`] borrows from [`ServerState`], which is right for a
/// request handler but impossible for the `'static` markdown-page probe the
/// link transform carries into the renderer. Built per page render; the clones
/// are a handful of short strings.
fn owned_resolver_config(config: &ServerState) -> crate::path_resolver::OwnedPathResolverConfig {
    crate::path_resolver::OwnedPathResolverConfig {
        base_dir: config.base_dir.clone(),
        canonical_base_dir: config.canonical_base_dir.clone(),
        static_folder: config.static_folder.clone(),
        markdown_extensions: config.markdown_extensions.clone(),
        index_file: config.index_file.clone(),
        tag_sources: crate::config::tag_sources_to_url_sources(&config.tag_sources),
    }
}

/// Everything [`index_page_links`] needs to reproduce the renderer's link
/// resolution outside a request.
///
/// Cloned once at startup rather than read from `ServerState`, because the
/// index is built and maintained from background tasks that outlive any
/// individual request.
#[derive(Clone)]
struct LinkIndexConfig {
    base_dir: PathBuf,
    index_file: String,
    markdown_extensions: Vec<String>,
    valid_tag_sources: HashSet<String>,
}

/// Parse one markdown file and record its contribution to the backlink index.
///
/// Link resolution must match what a real render would produce, or the info
/// panel would show backlinks the page does not actually contain. That is why
/// this goes through the same `extract_outbound_links_sync` →
/// `resolve_outbound_links` pair the request path uses, with the same
/// `LinkTransformConfig`, rather than re-deriving URLs.
fn index_page_links(
    repo: &Repo,
    index: &InboundIndex,
    cfg: &LinkIndexConfig,
    path: &Path,
    url_path: &str,
) {
    let is_index_file = path
        .file_name()
        .and_then(|f| f.to_str())
        .is_some_and(|f| f == cfg.index_file);

    let link_transform_config = LinkTransformConfig {
        markdown_extensions: cfg.markdown_extensions.clone(),
        index_file: cfg.index_file.clone(),
        is_index_file,
        url_depth: None,
        current_page_url: url_path.to_string(),
        markdown_page_probe: None,
    };

    match markdown::extract_outbound_links_sync(
        path.to_path_buf(),
        &cfg.base_dir,
        link_transform_config,
        true, // server_mode
        cfg.valid_tag_sources.clone(),
        Some(repo.wikilink_index.clone()),
    ) {
        Ok(links) => {
            // `OutboundLink.to` is the *raw* markdown destination
            // (`alpha.md`, `../notes/x.md`) — the renderer records it before
            // rewriting the href. Resolving that directly against the page URL
            // yields `/alpha.md/`, which matches no page, so every
            // extension-style link would silently produce no backlink.
            // Running it through the same `transform_link` the renderer uses
            // for the href first gives `../alpha/`, which resolves to the
            // page's real URL.
            let resolved: Vec<crate::link_index::OutboundLink> = links
                .into_iter()
                .map(|mut link| {
                    if link.internal && !link.to.is_empty() {
                        link.to = crate::link_transform::transform_link(
                            &link.to,
                            &LinkTransformConfig {
                                markdown_extensions: cfg.markdown_extensions.clone(),
                                index_file: cfg.index_file.clone(),
                                is_index_file,
                                url_depth: None,
                                current_page_url: url_path.to_string(),
                                markdown_page_probe: None,
                            },
                        );
                    }
                    link
                })
                .collect();
            // `true`, not `is_index_file`. These hrefs have *already* been
            // through `transform_link`, which added the `../` that compensates
            // for the trailing-slash URL convention. Passing `is_index_file`
            // here makes `resolve_relative_url` strip the page URL's last
            // segment a second time, so `docs/beta.md` linking `alpha.md`
            // recorded a backlink on `/alpha/` instead of `/docs/alpha/`. The
            // bug hid at the repository root, where the (former) silent clamp
            // on an above-root `..` cancelled the extra hop. This is the same
            // reasoning `page_errors::validate_media_references` documents for
            // post-transform srcs.
            let resolved = resolve_outbound_links(url_path, resolved, true);
            index.set_page_links(url_path, &resolved);
        }
        Err(e) => {
            // A file that cannot be parsed contributes no backlinks. Withdraw
            // whatever it contributed before so a file that became unreadable
            // does not leave stale backlinks behind.
            tracing::warn!("Backlink index: failed to read {}: {e}", path.display());
            index.remove_page(url_path);
        }
    }
}

/// Build the whole backlink index from the current repository contents.
///
/// Blocking and rayon-parallel; call from `spawn_blocking`.
fn populate_inbound_index(repo: &Repo, index: &InboundIndex, cfg: &LinkIndexConfig) {
    use rayon::prelude::*;

    let pages: Vec<(PathBuf, String)> = repo
        .markdown_files
        .pin()
        .iter()
        .map(|(path, info)| (path.clone(), info.url_path.clone()))
        .collect();

    let page_count = pages.len();
    let started = Instant::now();

    pages.par_iter().for_each(|(path, url_path)| {
        index_page_links(repo, index, cfg, path, url_path);
    });

    index.mark_ready();
    tracing::info!(
        "Backlink index built in {:?}: {} pages parsed, {} pages have backlinks",
        started.elapsed(),
        page_count,
        index.target_count(),
    );
}

/// The three caches that hold pre-rendered *listings* of the repository: the
/// per-directory file lists, the per-directory subdirectory lists, and the
/// serialized `/.mbr/site.json` body.
///
/// Grouped so every invalidation site clears all three. They are derived from
/// the same data, and there are several invalidation sites (the watcher task,
/// the create handler, the move handler); clearing only some of them serves a
/// page whose navigation disagrees with its content.
#[derive(Clone)]
struct ListingCaches {
    sibling_nav_cache: Arc<papaya::HashMap<PathBuf, Arc<Vec<serde_json::Value>>>>,
    subdir_cache: Arc<papaya::HashMap<PathBuf, Arc<Vec<serde_json::Value>>>>,
    site_json_cache: Arc<parking_lot::RwLock<SiteJsonCache>>,
}

impl ListingCaches {
    fn invalidate(&self) {
        self.sibling_nav_cache.pin().clear();
        self.subdir_cache.pin().clear();
        self.site_json_cache.write().invalidate();
    }
}

/// The memoized `/.mbr/site.json` body plus a generation counter.
///
/// The counter closes the fill/invalidate race: a rebuild that started before a
/// file change would otherwise publish its pre-change snapshot *after* the
/// invalidation cleared the slot, and that stale body would then be served
/// until the next change. A builder captures the generation up front and its
/// store is rejected if the generation moved meanwhile.
#[derive(Default)]
pub struct SiteJsonCache {
    generation: u64,
    body: Option<axum::body::Bytes>,
}

impl SiteJsonCache {
    /// The current generation and body, read together under one lock.
    fn snapshot(&self) -> (u64, Option<axum::body::Bytes>) {
        (self.generation, self.body.clone())
    }

    /// Publishes `body` only if no invalidation happened since `generation` was
    /// observed.
    fn store(&mut self, generation: u64, body: axum::body::Bytes) {
        if self.generation == generation {
            self.body = Some(body);
        }
    }

    fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.body = None;
    }
}

impl From<&ServerState> for ListingCaches {
    fn from(state: &ServerState) -> Self {
        Self {
            sibling_nav_cache: Arc::clone(&state.sibling_nav_cache),
            subdir_cache: Arc::clone(&state.subdir_cache),
            site_json_cache: Arc::clone(&state.site_json_cache),
        }
    }
}

/// What the live-reload WebSocket loop should do with a broadcast receive
/// result.
#[derive(Debug)]
enum LiveReloadAction {
    /// Serialize and forward this event to the client.
    Forward(crate::watcher::FileChangeEvent),
    /// This client fell behind and the channel dropped events; keep listening.
    Skip,
    /// The sender is gone (server shutting down); close the socket.
    Close,
}

/// Maps a file-change broadcast result to the live-reload loop's next action.
///
/// Extracted so the lag path is unit-testable: the loop must keep forwarding
/// after a `Lagged` error rather than treating it as terminal. Dropped events
/// only cost this client a missed reload, and the client re-fetches the page
/// on the next event it does receive.
fn live_reload_action(
    result: Result<crate::watcher::FileChangeEvent, broadcast::error::RecvError>,
) -> LiveReloadAction {
    match result {
        Ok(event) => LiveReloadAction::Forward(event),
        Err(broadcast::error::RecvError::Lagged(skipped)) => {
            tracing::warn!("Live reload client lagged; {skipped} file change event(s) dropped");
            LiveReloadAction::Skip
        }
        Err(broadcast::error::RecvError::Closed) => {
            tracing::debug!("File change channel closed; ending live reload stream");
            LiveReloadAction::Close
        }
    }
}

/// Default outbound link cache size: 2 MB.
const DEFAULT_LINK_CACHE_SIZE: usize = 2 * 1024 * 1024;

/// Default inbound link cache size: 4 MB. Sized for the sidebar mini graph,
/// which fans out links.json requests for many pages at once.
const DEFAULT_INBOUND_LINK_CACHE_SIZE: usize = 4 * 1024 * 1024;

/// TTL for inbound link cache entries in seconds. The editing endpoints
/// (`/.mbr/create`, `/.mbr/move`, `/.mbr/mkdir`) invalidate this cache when
/// they mutate the repo, and the file watcher drops it wholesale (together with
/// the outbound `LinkCache`) after each debounced batch of changes, but a
/// bounded TTL still guards against any missed invalidation and keeps
/// mini-graph bursts from re-grepping the repo.
const INBOUND_LINK_CACHE_TTL_SECS: u64 = 300;

/// Maximum inbound-link greps allowed to run concurrently. Each grep walks
/// the whole repository, so a burst of links.json requests (the sidebar mini
/// graph fetches 10-80 pages) must not stampede the filesystem.
const INBOUND_GREP_MAX_CONCURRENCY: usize = 2;

/// Maximum time a request waits for an in-progress inbound-link grep before
/// giving up (degrades a lost wakeup into a retryable `None`).
const INBOUND_GREP_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// `content-type` prefixes whose payloads must never be gzipped.
///
/// Two independent reasons, either of which is disqualifying:
///
/// 1. **Correctness.** tower-http drops `content-length` and `accept-ranges`
///    and switches to `transfer-encoding: chunked` on any response it
///    compresses. For `video/*` and `audio/*` that destroys seeking and
///    duration detection in every client that negotiates gzip — verified
///    against WebKit, which is the engine behind mbr's own GUI window. (Range
///    requests are unaffected because tower-http skips compression when
///    `content-range` is present, so the bug only bites the initial plain
///    `GET`.)
/// 2. **Waste.** These formats are already entropy-coded, so gzip burns CPU on
///    every byte of a potentially multi-gigabyte file to save nothing.
///
/// Matched as a prefix against the response's `content-type`, mirroring
/// tower-http's own [`NotForContentType`] semantics.
///
/// [`NotForContentType`]: tower_http::compression::predicate::NotForContentType
const INCOMPRESSIBLE_CONTENT_TYPE_PREFIXES: &[&str] = &[
    "video/",
    "audio/",
    "application/pdf",
    "application/zip",
    "application/gzip",
    "application/x-gzip",
    "application/octet-stream",
];

/// Returns `true` when the response's `content-type` names an already-compressed
/// or range-critical payload that must bypass the compression layer.
///
/// Absent or non-ASCII `content-type` headers compare against the empty string
/// and therefore match nothing, preserving tower-http's compress-by-default
/// behaviour for everything not explicitly listed.
fn is_incompressible_content_type(headers: &HeaderMap) -> bool {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    INCOMPRESSIBLE_CONTENT_TYPE_PREFIXES
        .iter()
        .any(|prefix| content_type.starts_with(prefix))
}

/// tower-http compression predicate: compress unless the payload is one of the
/// [`INCOMPRESSIBLE_CONTENT_TYPE_PREFIXES`].
///
/// Written as a free function rather than a closure so it coerces cleanly into
/// tower-http's blanket `Predicate` impl for
/// `Fn(StatusCode, Version, &HeaderMap, &Extensions) -> bool`.
fn compress_by_content_type(
    _status: StatusCode,
    _version: axum::http::Version,
    headers: &HeaderMap,
    _extensions: &axum::http::Extensions,
) -> bool {
    !is_incompressible_content_type(headers)
}

/// Builds the compression predicate for the router.
///
/// Keeps every [`DefaultPredicate`] exclusion (gRPC, images, SSE, tiny bodies)
/// and adds [`INCOMPRESSIBLE_CONTENT_TYPE_PREFIXES`] on top.
///
/// [`DefaultPredicate`]: tower_http::compression::predicate::DefaultPredicate
fn compression_predicate() -> impl tower_http::compression::Predicate {
    use tower_http::compression::predicate::{DefaultPredicate, Predicate};

    DefaultPredicate::new().and(compress_by_content_type)
}

/// Type of media for the viewer page.
///
/// Used to route requests to the appropriate media viewer template
/// at `/.mbr/videos/`, `/.mbr/pdfs/`, `/.mbr/audio/`, or `/.mbr/images/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaViewerType {
    Video,
    Pdf,
    Audio,
    Image,
}

impl MediaViewerType {
    /// Parse from route path.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// assert_eq!(MediaViewerType::from_route("/.mbr/videos/"), Some(MediaViewerType::Video));
    /// assert_eq!(MediaViewerType::from_route("/.mbr/pdfs/"), Some(MediaViewerType::Pdf));
    /// assert_eq!(MediaViewerType::from_route("/.mbr/audio/"), Some(MediaViewerType::Audio));
    /// assert_eq!(MediaViewerType::from_route("/.mbr/images/"), Some(MediaViewerType::Image));
    /// assert_eq!(MediaViewerType::from_route("/some/other/path"), None);
    /// ```
    #[must_use]
    pub fn from_route(path: &str) -> Option<Self> {
        match path {
            "/.mbr/videos/" => Some(Self::Video),
            "/.mbr/pdfs/" => Some(Self::Pdf),
            "/.mbr/audio/" => Some(Self::Audio),
            "/.mbr/images/" => Some(Self::Image),
            _ => None,
        }
    }

    /// Template name for this media type.
    #[must_use]
    pub const fn template_name(&self) -> &'static str {
        "media_viewer.html"
    }

    /// Human-readable label for this media type.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Video => "Video",
            Self::Pdf => "PDF",
            Self::Audio => "Audio",
            Self::Image => "Image",
        }
    }

    /// Lowercase string representation for template context.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Pdf => "pdf",
            Self::Audio => "audio",
            Self::Image => "image",
        }
    }

    /// Determine media type from a file extension (case-insensitive).
    ///
    /// Returns `None` for unrecognized extensions.
    #[must_use]
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            // Video
            "mp4" | "m4v" | "mov" | "webm" | "flv" | "mpg" | "mpeg" | "avi" | "3gp" | "wmv"
            | "mkv" | "ts" | "mts" | "m2ts" | "vob" | "divx" | "xvid" | "asf" | "rm" | "rmvb"
            | "f4v" | "ogv" => Some(Self::Video),
            // Audio
            "mp3" | "wav" | "ogg" | "flac" | "aac" | "m4a" | "aiff" | "aif" | "oga" | "opus"
            | "wma" => Some(Self::Audio),
            // Image
            "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "svg" => {
                Some(Self::Image)
            }
            // PDF
            "pdf" => Some(Self::Pdf),
            _ => None,
        }
    }

    /// Determine media type from a file path by inspecting its extension.
    ///
    /// Returns `None` if the path has no extension or the extension is unrecognized.
    #[must_use]
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(Self::from_extension)
    }

    /// Returns the server route path for this media viewer type.
    #[must_use]
    pub const fn route_path(&self) -> &'static str {
        match self {
            Self::Video => "/.mbr/videos/",
            Self::Pdf => "/.mbr/pdfs/",
            Self::Audio => "/.mbr/audio/",
            Self::Image => "/.mbr/images/",
        }
    }
}

/// Query parameters for media viewer routes.
#[derive(Debug, serde::Deserialize)]
pub struct MediaViewerQuery {
    /// Path to the media file (URL-encoded)
    pub path: Option<String>,
}

/// Validates a media path from a query parameter.
///
/// - URL-decodes the path
/// - Rejects paths containing ".." (directory traversal)
/// - Validates the path resolves within the repository root OR the static folder
///
/// # Arguments
///
/// * `path` - The URL-encoded path from the query parameter
/// * `repo_root` - The repository root directory
/// * `static_folder` - The static folder path (may be relative to repo_root, e.g., "../static")
///
/// # Returns
///
/// * `Ok(PathBuf)` - The validated, canonical path to the media file
/// * `Err(MbrError)` - If the path is invalid or attempts directory traversal
///
/// # Security
///
/// When `static_folder` points outside `repo_root` (e.g., `../static`), paths are validated
/// against BOTH directories. This allows serving assets from external static folders while
/// maintaining path traversal protection. Content root takes precedence if the file exists
/// in both locations.
pub fn validate_media_path(
    path: &str,
    repo_root: &Path,
    static_folder: &str,
) -> Result<PathBuf, MbrError> {
    // URL-decode the path
    let decoded = percent_decode_str(path)
        .decode_utf8()
        .map_err(|_| MbrError::InvalidMediaPath("Invalid UTF-8 in path".to_string()))?;

    // Reject paths containing ".." to prevent directory traversal
    if decoded.contains("..") {
        return Err(MbrError::DirectoryTraversal);
    }

    // Remove leading slash if present for path joining
    let clean_path = decoded.trim_start_matches('/');

    // Try repo_root first
    let full_path = repo_root.join(clean_path);

    // Canonicalize repo root for validation
    let canonical_root = repo_root
        .canonicalize()
        .map_err(|_| MbrError::InvalidMediaPath("Repository root not found".to_string()))?;

    // Try to resolve within repo_root
    if let Ok(canonical_path) = full_path.canonicalize()
        && canonical_path.starts_with(&canonical_root)
    {
        return Ok(canonical_path);
    }

    // If static_folder is non-empty, try resolving against it as a fallback
    if !static_folder.is_empty() {
        let static_root = repo_root.join(static_folder);

        // The static folder must exist
        if let Ok(canonical_static_root) = static_root.canonicalize() {
            let static_full_path = static_root.join(clean_path);

            if let Ok(canonical_path) = static_full_path.canonicalize()
                && canonical_path.starts_with(&canonical_static_root)
            {
                return Ok(canonical_path);
            }
        }
    }

    // Neither repo_root nor static_folder contained a valid path
    Err(MbrError::InvalidMediaPath(format!(
        "Path does not exist: {}",
        decoded
    )))
}

/// Safely join a base directory with a relative path for serving MBR assets.
///
/// Returns `Some(PathBuf)` if the path is safe (within base_dir and exists as a file),
/// `None` otherwise. This prevents path traversal attacks.
///
/// # Security
///
/// This function guards against path traversal attacks by:
/// 1. Rejecting paths containing ".." before any filesystem operations
/// 2. Canonicalizing both the base directory and the joined path
/// 3. Verifying the resolved path starts with the base directory
/// 4. Ensuring the path is a file (not a directory)
fn safe_join_asset(base_dir: &Path, relative_path: &str) -> Option<PathBuf> {
    // Early rejection of obvious path traversal attempts
    if relative_path.contains("..") {
        tracing::warn!(
            "Path traversal attempt blocked in MBR assets: {}",
            relative_path
        );
        return None;
    }

    let clean_path = relative_path.trim_start_matches('/');
    let candidate = base_dir.join(clean_path);

    // Canonicalize base_dir first to handle any symlinks in the base
    let canonical_base = base_dir.canonicalize().ok()?;

    // Canonicalize the candidate path - this resolves symlinks and ".."
    let canonical = candidate.canonicalize().ok()?;

    // Verify containment and that it's a file
    if canonical.starts_with(&canonical_base) && canonical.is_file() {
        Some(canonical)
    } else {
        None
    }
}

/// Verify that a file path is safely contained within a base directory.
///
/// Returns `Some(PathBuf)` with the canonical path if valid, `None` if the path
/// escapes the base directory (path traversal) or doesn't exist as a file.
///
/// This is used for defense-in-depth validation of paths that have already
/// been constructed from URL paths.
///
/// # Security
///
/// Guards against path traversal by canonicalizing both paths and verifying containment.
/// Note: We intentionally do NOT reject paths containing ".." before canonicalization
/// because the base_dir itself may be constructed with ".." (e.g., when static_folder
/// is "../static"). The canonicalization resolves all ".." components, and the
/// starts_with check ensures the resolved path is within bounds.
#[cfg(feature = "media-metadata")]
fn validate_path_containment(file_path: &Path, base_dir: &Path) -> Option<PathBuf> {
    let canonical_base = base_dir.canonicalize().ok()?;
    let canonical_file = file_path.canonicalize().ok()?;

    if canonical_file.starts_with(&canonical_base) && canonical_file.is_file() {
        Some(canonical_file)
    } else {
        None
    }
}

/// Resolve a media source file from a URL path with path traversal protection.
///
/// Tries `base_dir/url_path` first, then falls back to
/// `base_dir/static_folder/url_path`. Each candidate is validated with
/// [`validate_path_containment`], so paths that escape their containing
/// directory (e.g. via `..`) return `None`.
///
/// Returns the canonical path of the resolved file, or `None` if the file
/// doesn't exist in either location or the path escapes containment.
#[cfg(feature = "media-metadata")]
fn resolve_media_source_file(
    url_path: &str,
    base_dir: &Path,
    static_folder: &str,
) -> Option<PathBuf> {
    let direct = base_dir.join(url_path);
    // Validate path stays within base_dir (defense in depth)
    validate_path_containment(&direct, base_dir).or_else(|| {
        // Validate path stays within static folder
        let static_dir = base_dir.join(static_folder);
        validate_path_containment(&static_dir.join(url_path), &static_dir)
    })
}

/// Name of the per-repository template/configuration folder.
const MBR_TEMPLATE_DIR: &str = ".mbr";

/// File extensions the `/.mbr/*` route is allowed to serve.
///
/// `.mbr/` is a *configuration* directory as much as an asset directory: it
/// holds `config.toml` with the Argon2 `edit_token_hash`. Serving it required
/// no credential, so the route is restricted to the asset types the compiled-in
/// defaults and shipped templates actually reference (stylesheets, scripts,
/// source maps, templates, images, fonts) and everything else 404s.
const MBR_ASSET_EXTENSIONS: &[&str] = &[
    // Documents, styles, scripts and their source maps
    "css",
    "js",
    "mjs",
    "map",
    "json",
    "html",
    "txt", // Images and fonts
    "png",
    "jpg",
    "jpeg",
    "gif",
    "webp",
    "svg",
    "ico",
    "woff",
    "woff2",
    "ttf",
    "otf",
    "eot",
    // Pagefind index shipped inside `.mbr/pagefind/` by a static build
    "wasm",
    "pf_meta",
    "pf_fragment",
    "pf_index",
];

/// Whether `asset_path` (a `/`-prefixed path relative to the template folder)
/// may be served by [`Server::serve_mbr_assets`].
///
/// Requires an allowlisted extension ([`MBR_ASSET_EXTENSIONS`]) and rejects any
/// dot-prefixed component, so neither `config.toml` nor a dotfile such as
/// `.env` can be read out of the template folder.
fn is_servable_mbr_asset(asset_path: &str) -> bool {
    let path = Path::new(asset_path);
    let has_hidden_component = path.components().any(|component| match component {
        std::path::Component::Normal(name) => {
            name.to_str().is_some_and(|name| name.starts_with('.'))
        }
        std::path::Component::CurDir | std::path::Component::ParentDir => true,
        _ => false,
    });
    if has_hidden_component {
        return false;
    }
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| MBR_ASSET_EXTENSIONS.contains(&ext.as_str()))
}

/// File extensions the `/.mbr/upload` endpoint accepts.
///
/// Mirrors the image/audio/video/PDF classification in
/// `repo::StaticFileMetadata::empty` (plus the `vtt`/`srt` caption sidecars the
/// video pipeline understands). The uploader must never be able to create a
/// file the browser or the template engine will *execute* — `.html`, `.js`,
/// `.css` and `.toml` are deliberately absent, and so is `.svg`, which executes
/// script when navigated to directly.
const UPLOAD_ALLOWED_EXTENSIONS: &[&str] = &[
    // Images
    "jpg", "jpeg", "png", "webp", "gif", "bmp", "tif", "tiff", // Audio
    "aiff", "aif", "mp3", "aac", "m4a", "ogg", "oga", "opus", "wma", "flac", "wav",
    // Video
    "mp4", "m4v", "mov", "webm", "flv", "mpg", "mpeg", "avi", "3gp", "wmv",
    // Documents and caption sidecars
    "pdf", "vtt", "srt",
];

/// Canonicalizes the deepest existing ancestor of `path` (the path itself when
/// it already exists), so containment checks also work for files that have not
/// been created yet.
fn canonical_existing_ancestor(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find_map(|ancestor| ancestor.canonicalize().ok())
}

/// Whether `target` lands inside mbr's template folder — the repository's
/// `.mbr/` directory or an explicitly configured `--template-folder`.
///
/// Checked both lexically (the caller builds `target` by joining onto the
/// canonical root, so `dir=.mbr` is caught before the file exists) and against
/// the canonicalized deepest existing ancestor (so a symlinked `.mbr` or a
/// template folder given by a non-canonical path is caught too).
fn is_template_folder_path(
    target: &Path,
    base_dir: &Path,
    canonical_base_dir: Option<&Path>,
    template_folder: Option<&Path>,
) -> bool {
    let canonical_target = canonical_existing_ancestor(target);
    std::iter::once(base_dir.join(MBR_TEMPLATE_DIR))
        .chain(canonical_base_dir.map(|base| base.join(MBR_TEMPLATE_DIR)))
        .chain(template_folder.map(Path::to_path_buf))
        .any(|root| {
            target.starts_with(&root)
                || match (root.canonicalize(), canonical_target.as_deref()) {
                    (Ok(root), Some(canonical_target)) => canonical_target.starts_with(root),
                    _ => false,
                }
        })
}

/// Extracts the hostname from a `Host` header value, dropping the port and any
/// IPv6 brackets: `127.0.0.1:5200` → `127.0.0.1`, `[::1]:5200` → `::1`,
/// `localhost` → `localhost`.
fn host_header_hostname(host: &str) -> &str {
    let host = host.trim();
    if let Some(rest) = host.strip_prefix('[') {
        // Bracketed IPv6 literal, with or without a port.
        rest.split(']').next().unwrap_or(rest)
    } else if host.matches(':').count() > 1 {
        // Bare IPv6 literal (not RFC-conformant in `Host`, but be lenient).
        host
    } else {
        host.split(':').next().unwrap_or(host)
    }
}

/// Whether a `Host` header names an address this server could have been reached
/// at directly.
///
/// Accepts `localhost`, any loopback IP literal (`127.0.0.1`, `127.0.0.53`,
/// `::1`, `[::1]`) and the configured bind address. Everything else — notably
/// an attacker-controlled name that DNS-rebinds to 127.0.0.1 — is rejected, as
/// is a missing `Host` (HTTP/1.1 requires one and every browser sends it).
///
/// The port is deliberately ignored: `start_with_port_retry` may bind a
/// different port than the configured one, and a rebinding attacker controls
/// the *name*, never the port. A wildcard bind (`0.0.0.0`) is only reachable
/// under a name this rejects, but `Config::validate` already refuses to enable
/// editing on a non-loopback bind without a token, and a configured token skips
/// this check entirely.
fn host_header_is_allowed(headers: &HeaderMap, bind_ip: [u8; 4]) -> bool {
    let Some(raw) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let hostname = host_header_hostname(raw);
    if hostname.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match hostname.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.is_loopback() || ip == std::net::IpAddr::from(bind_ip),
        Err(_) => false,
    }
}

/// Defense-in-depth containment check for a path the router is about to serve.
///
/// Returns `true` only when `path` **canonicalizes** inside the repository root
/// or inside the `static_folder` overlay (which may legitimately resolve
/// outside the root, e.g. `static_folder = "../static"`).
///
/// `Path::starts_with` on an un-canonicalized path is not enough: a symlink
/// whose target lives outside the root is lexically inside it, and both
/// `is_file()` and `ServeFile` follow symlinks. Every resolver in this crate is
/// supposed to reject that, so this is a second, independent gate rather than
/// the primary one.
fn is_within_served_roots(
    path: &Path,
    base_dir: &Path,
    canonical_base_dir: Option<&Path>,
    static_folder: &str,
) -> bool {
    let Ok(canonical) = path.canonicalize() else {
        return false;
    };
    let owned_base;
    let base = match canonical_base_dir {
        Some(cached) => Some(cached),
        None => {
            owned_base = base_dir.canonicalize().ok();
            owned_base.as_deref()
        }
    };
    if base.is_some_and(|base| canonical.starts_with(base)) {
        return true;
    }
    // Only pay for this canonicalize when the cheap check already failed.
    base_dir
        .join(static_folder)
        .canonicalize()
        .is_ok_and(|static_root| canonical.starts_with(static_root))
}

pub struct Server {
    pub router: Router,
    pub port: u16,
    pub ip: [u8; 4],
    /// True when a native GUI window fronts this server. Only affects how the
    /// startup banner is announced; see [`Server::announce_listening`].
    pub gui_mode: bool,
    /// File watcher handle - kept alive for the lifetime of the server.
    /// When Server is dropped, this is dropped, stopping the watcher.
    _watcher_handle: Arc<std::sync::Mutex<Option<crate::watcher::FileWatcher>>>,
}

/// Configuration for initializing a Server instance.
///
/// This struct consolidates all parameters needed by `Server::init`,
/// making it easier to construct and pass around configuration.
///
/// # Example
///
/// ```ignore
/// use mbr::server::ServerConfig;
/// use mbr::config::Config;
///
/// let config = Config::default();
/// let server_config = ServerConfig::from(&config)
///     .with_gui_mode(false)
///     .with_log_filter(Some("mbr=debug"));
/// let server = Server::init(server_config)?;
/// ```
#[derive(Clone)]
pub struct ServerConfig {
    pub ip: [u8; 4],
    pub port: u16,
    pub base_dir: std::path::PathBuf,
    pub static_folder: String,
    pub markdown_extensions: Vec<String>,
    pub ignore_dirs: Vec<String>,
    pub ignore_globs: Vec<String>,
    pub watcher_ignore_dirs: Vec<String>,
    pub index_file: String,
    pub oembed_timeout_ms: u64,
    pub oembed_cache_size: usize,
    /// Budget in bytes for the video/PDF metadata cache (cover images,
    /// chapters, captions). Separate from `oembed_cache_size`, which sizes a
    /// text-metadata cache.
    #[cfg(feature = "media-metadata")]
    pub media_cache_size: usize,
    pub template_folder: Option<std::path::PathBuf>,
    pub sort: Vec<SortField>,
    pub gui_mode: bool,
    pub theme: String,
    pub log_filter: Option<String>,
    pub link_tracking: bool,
    /// Enable typed relationship tracking.
    pub relationship_tracking: bool,
    /// Configured relation types (for the relationship index + site.json).
    pub relationship_types: Vec<RelationType>,
    pub tag_sources: Vec<TagSource>,
    pub sidebar_style: String,
    pub sidebar_max_items: usize,
    /// Depth (hops) of the sidebar mini graph neighborhood (1-5).
    pub graph_depth: usize,
    pub title_prefix: String,
    pub title_suffix: String,
    /// Highlight blocks beginning with an incomplete marker (TK/TODO/FIXME/XXX).
    pub mark_incomplete: bool,
    /// Marker strings used by the incomplete-block highlighter.
    pub incomplete_markers: Vec<String>,
    /// Enable the task browser (`POST /.mbr/tasks`).
    pub tasks_enabled: bool,
    /// Maintain the `@done(...)` annotation when `POST /.mbr/task` toggles a
    /// task's status.
    pub tasks_stamp_done: bool,
    /// Globs matched against repo-relative paths whose files are kept out of the
    /// task index (e.g. `templates/**`). Empty by default.
    pub tasks_ignore_globs: Vec<String>,
    /// Enable the in-browser markdown editing endpoints.
    pub edit_enabled: bool,
    /// Require the editing token even for loopback callers.
    pub edit_require_token_on_loopback: bool,
    /// Argon2 PHC hash of the shared editing token (server-side only).
    pub edit_token_hash: Option<String>,
    /// Maximum size in bytes of a single asset uploaded via `/.mbr/upload`.
    pub upload_max_bytes: usize,
    #[cfg(feature = "media-metadata")]
    pub transcode_enabled: bool,
}

impl ServerConfig {
    /// Set whether the server is running in GUI mode (native window).
    #[must_use]
    pub fn with_gui_mode(mut self, gui_mode: bool) -> Self {
        self.gui_mode = gui_mode;
        self
    }

    /// Set the log filter for tracing (e.g., "mbr=debug,tower_http=warn").
    #[must_use]
    pub fn with_log_filter(mut self, filter: Option<&str>) -> Self {
        self.log_filter = filter.map(|s| s.to_string());
        self
    }
}

impl From<&crate::config::Config> for ServerConfig {
    fn from(config: &crate::config::Config) -> Self {
        Self {
            ip: config.host.0,
            port: config.port,
            base_dir: config.root_dir.clone(),
            static_folder: config.static_folder.clone(),
            markdown_extensions: config.markdown_extensions.clone(),
            ignore_dirs: config.ignore_dirs.clone(),
            ignore_globs: config.ignore_globs.clone(),
            watcher_ignore_dirs: config.watcher_ignore_dirs.clone(),
            index_file: config.index_file.clone(),
            oembed_timeout_ms: config.oembed_timeout_ms,
            oembed_cache_size: config.oembed_cache_size,
            #[cfg(feature = "media-metadata")]
            media_cache_size: config.media_cache_size,
            template_folder: config.template_folder.clone(),
            sort: config.sort.clone(),
            gui_mode: false, // Default to server mode
            theme: config.theme.clone(),
            log_filter: None, // Set via with_log_filter()
            link_tracking: config.link_tracking,
            relationship_tracking: config.relationship_tracking,
            relationship_types: config.relationship_types.clone(),
            tag_sources: config.tag_sources.clone(),
            sidebar_style: config.sidebar_style.clone(),
            sidebar_max_items: config.sidebar_max_items,
            graph_depth: config.graph_depth,
            title_prefix: config.title_prefix.clone(),
            title_suffix: config.title_suffix.clone(),
            // Server/GUI default: on unless config overrides.
            mark_incomplete: config.mark_incomplete.unwrap_or(true),
            incomplete_markers: config.incomplete_markers.clone(),
            tasks_enabled: config.tasks_enabled,
            tasks_stamp_done: config.tasks_stamp_done,
            tasks_ignore_globs: config.tasks_ignore_globs.clone(),
            edit_enabled: config.edit_enabled,
            edit_require_token_on_loopback: config.edit_require_token_on_loopback,
            edit_token_hash: config.edit_token_hash.clone(),
            upload_max_bytes: config.upload_max_bytes,
            #[cfg(feature = "media-metadata")]
            transcode_enabled: config.transcode,
        }
    }
}

#[derive(Clone)]
pub struct ServerState {
    /// IPv4 address the server was configured to bind. Used to validate the
    /// `Host` header on editing requests (anti DNS-rebinding).
    pub bind_ip: [u8; 4],
    pub base_dir: std::path::PathBuf,
    /// Pre-computed canonical base directory for path resolution (avoids per-request canonicalize)
    pub canonical_base_dir: Option<std::path::PathBuf>,
    pub static_folder: String,
    pub markdown_extensions: Vec<String>,
    pub ignore_dirs: Vec<String>,
    pub ignore_globs: Vec<String>,
    pub index_file: String,
    pub templates: crate::templates::Templates,
    pub repo: Arc<Repo>,
    pub oembed_timeout_ms: u64,
    pub file_change_tx: Option<broadcast::Sender<crate::watcher::FileChangeEvent>>,
    /// Optional template folder that overrides default .mbr/ and compiled defaults
    pub template_folder: Option<std::path::PathBuf>,
    /// Sort configuration for file listings
    pub sort: Vec<SortField>,
    /// Whether the server is running in GUI mode (native window) vs browser mode
    pub gui_mode: bool,
    /// Theme for Pico CSS selection (e.g., "default", "amber", "fluid", "fluid.jade")
    pub theme: String,
    /// Cache for OEmbed page metadata to avoid redundant network requests
    pub oembed_cache: Arc<OembedCache>,
    /// Cache for dynamically generated video metadata (covers, chapters, captions)
    #[cfg(feature = "media-metadata")]
    pub video_metadata_cache: Arc<VideoMetadataCache>,
    /// Whether video transcoding is enabled
    #[cfg(feature = "media-metadata")]
    pub transcode_enabled: bool,
    /// Cache for HLS playlists and transcoded segments
    #[cfg(feature = "media-metadata")]
    pub hls_cache: Arc<HlsCache>,
    /// Single-flight guard for video metadata extraction: maps a metadata cache
    /// key to a `Notify` while an extraction is in progress, so a gallery with N
    /// references to the same clip only spawns one ffmpeg decode (others await).
    #[cfg(feature = "media-metadata")]
    pub metadata_inflight: Arc<papaya::HashMap<String, Arc<tokio::sync::Notify>>>,
    /// Cache of probed video resolutions keyed by path+mtime, so repeated HLS
    /// requests for an unchanged file never re-run a blocking ffmpeg demux.
    #[cfg(feature = "media-metadata")]
    pub video_resolution_cache:
        Arc<papaya::HashMap<String, crate::video_transcode::VideoResolution>>,
    /// Cache of playback-compatibility probes keyed by path+mtime. Both
    /// outcomes are cached, so a page reload never re-opens an unchanged file,
    /// and an edited file is transparently re-probed.
    #[cfg(feature = "media-metadata")]
    pub media_compat_cache:
        Arc<papaya::HashMap<String, crate::video_metadata::PlaybackCompatibility>>,
    /// Per-directory memoized sibling navigation lists (prev/next). Avoids an
    /// O(repo) scan on every markdown render; invalidated when files change.
    ///
    /// Doubles as the *file* half of a directory listing: the listing for a
    /// directory is exactly the sorted sibling list of the files it contains.
    pub sibling_nav_cache: Arc<papaya::HashMap<PathBuf, Arc<Vec<serde_json::Value>>>>,
    /// Per-directory memoized immediate-subdirectory lists, the other half of a
    /// directory listing. Invalidated alongside `sibling_nav_cache`.
    pub subdir_cache: Arc<papaya::HashMap<PathBuf, Arc<Vec<serde_json::Value>>>>,
    /// Fully rendered `/.mbr/site.json` body.
    ///
    /// The payload is a pure function of the repository index, but mbr is a
    /// multi-page app: `shared.ts` fetches it on *every* navigation, and
    /// rebuilding it re-serializes every markdown file's frontmatter each time.
    /// Cleared whenever the repo changes (alongside `sibling_nav_cache`) and
    /// rebuilt lazily on the next request.
    pub site_json_cache: Arc<parking_lot::RwLock<SiteJsonCache>>,
    /// Whether bidirectional link tracking is enabled
    pub link_tracking: bool,
    /// Whether typed relationship tracking is enabled
    pub relationship_tracking: bool,
    /// Configured relation types (for directory-scan temp repos + site.json)
    pub relationship_types: Vec<RelationType>,
    /// Cache for outbound links extracted during page renders
    pub link_cache: Arc<LinkCache>,
    /// Cache for inbound links discovered via grep
    pub inbound_link_cache: Arc<InboundLinkCache>,
    /// Repository-wide backlink index, built once in the background after the
    /// initial scan and maintained incrementally by the watcher. Until it
    /// reports ready, `links.json` falls back to the per-page grep below.
    pub inbound_index: Arc<InboundIndex>,
    /// Bounds concurrent inbound-link greps: each cache miss walks the whole
    /// repository, and the sidebar mini graph fans out many links.json
    /// requests at once, so at most two full-repo walks run at a time.
    pub inbound_grep_semaphore: Arc<tokio::sync::Semaphore>,
    /// Single-flight guard for inbound-link greps: maps a page URL path to a
    /// `Notify` while a grep is in progress, so N concurrent links.json
    /// requests for the same page trigger one repo walk (others await).
    pub inbound_grep_inflight: Arc<papaya::HashMap<String, Arc<tokio::sync::Notify>>>,
    /// Tag sources for frontmatter extraction
    pub tag_sources: Vec<TagSource>,
    /// Sidebar navigation style ("panel" for mbr-browse, "single" for mbr-browse-single)
    pub sidebar_style: String,
    /// Maximum items per section in sidebar navigation
    pub sidebar_max_items: usize,
    /// Depth (hops) of the sidebar mini graph neighborhood (1-5)
    pub graph_depth: usize,
    /// Text to prepend to all page titles
    pub title_prefix: String,
    /// Text to append to all page titles
    pub title_suffix: String,
    /// Highlight blocks beginning with TK/TODO/FIXME/XXX (default on in server/GUI).
    pub mark_incomplete: bool,
    /// Marker strings used by the incomplete-block highlighter.
    pub incomplete_markers: Vec<String>,
    /// Whether the task browser (`POST /.mbr/tasks`) is enabled.
    pub tasks_enabled: bool,
    /// Whether `POST /.mbr/task` maintains the `@done(...)` annotation when it
    /// toggles a task's status.
    pub tasks_stamp_done: bool,
    /// Lazy index of the repository's markdown tasks.
    ///
    /// Deliberately *not* built at startup: it is filled on the first task
    /// query and then kept fresh by the watcher, so a server whose user never
    /// opens the task panel never pays for a full-repo read pass.
    pub task_index: Arc<crate::task_index::TaskIndex>,
    /// Whether the in-browser markdown editing endpoints are enabled.
    pub edit_enabled: bool,
    /// Require the editing token even for loopback callers.
    pub edit_require_token_on_loopback: bool,
    /// Argon2 PHC hash of the shared editing token (never sent to the frontend).
    pub edit_token_hash: Option<String>,
    /// Maximum size in bytes of a single asset uploaded via `/.mbr/upload`.
    pub upload_max_bytes: usize,
}

/// JSON body for `POST /.mbr/edit/{*path}`.
#[derive(serde::Deserialize)]
pub struct EditRequest {
    /// Full new file contents (frontmatter + body, recombined by the client).
    pub content: String,
    /// SHA-256 hex of the content the client loaded, for optimistic concurrency.
    pub base_hash: String,
}

/// JSON body for `POST /.mbr/task`.
///
/// `expected` is the per-line analogue of [`EditRequest::base_hash`]: it is what
/// makes it safe to patch one line of a file without holding the whole file.
#[derive(serde::Deserialize)]
pub struct TaskToggleRequest {
    /// Repo-relative **filesystem** path of the file to patch (with extension),
    /// the same convention as `/.mbr/raw` and `/.mbr/edit`.
    pub path: String,
    /// 1-based source line of the task.
    pub line: u32,
    /// The exact current text of that line, verbatim. The line terminator is
    /// ignored, everything else must match byte for byte.
    pub expected: String,
    /// Target status: `open`, `done` or `canceled`.
    pub to: crate::tasks::TaskStatus,
}

/// Response for a successful `POST /.mbr/task`.
#[derive(serde::Serialize)]
pub struct TaskToggleResponse {
    /// The line that was patched, echoed back.
    pub line: u32,
    /// Its new text, without the terminator — including any `@done(...)` the
    /// server added or removed, which the client cannot predict.
    pub text: String,
}

/// JSON body for `POST /.mbr/create/{*path}`.
#[derive(serde::Deserialize)]
pub struct CreateRequest {
    /// Full file contents (frontmatter + body) for the new markdown file.
    pub content: String,
    /// Create any missing parent directories.
    #[serde(default)]
    pub create_dirs: bool,
}

/// JSON body for `POST /.mbr/move/{*path}`.
#[derive(serde::Deserialize)]
pub struct MoveRequest {
    /// Destination repo-relative filesystem path (with markdown extension).
    pub to: String,
    /// Create any missing parent directories at the destination.
    #[serde(default)]
    pub create_dirs: bool,
}

/// Response for a successful `POST /.mbr/create`.
#[derive(serde::Serialize)]
pub struct CreateResponse {
    /// The canonical site URL of the new page.
    pub url_path: String,
    /// The repo-relative filesystem path of the new file.
    pub path: String,
}

/// Response for a successful `POST /.mbr/mkdir`.
#[derive(serde::Serialize)]
pub struct MkdirResponse {
    /// The repo-relative filesystem path of the folder.
    pub path: String,
}

/// Response for a successful `POST /.mbr/move`.
#[derive(serde::Serialize)]
pub struct MoveResponse {
    /// The canonical site URL the page moved *from* (now gone).
    pub from_url: String,
    /// The canonical site URL the page moved *to*.
    pub url_path: String,
    /// The repo-relative filesystem path of the destination file.
    pub path: String,
    /// Site URLs of pages whose inbound links were rewritten (A4-A).
    pub rewritten: Vec<String>,
    /// Site URLs of pages whose bare `[[Name]]` links were rewritten (A4-C).
    pub wikilinks_rewritten: Vec<String>,
    /// Whether any missing destination parent directories were created.
    pub created_dirs: bool,
}

/// Query parameters for `POST /.mbr/upload`.
#[derive(serde::Deserialize)]
pub struct UploadParams {
    /// Repo-relative destination folder (the note's own folder). May be empty
    /// (`""`) for a root-level note.
    #[serde(default)]
    pub dir: String,
    /// Desired filename (basename with extension).
    pub name: String,
}

/// Response for a successful `POST /.mbr/upload`.
#[derive(serde::Serialize)]
pub struct UploadResponse {
    /// Root-absolute, percent-encoded URL of the saved file (matches how mbr
    /// serves it), e.g. `/notes/image.png`.
    pub url: String,
    /// Repo-relative filesystem path of the saved file, e.g. `notes/image.png`.
    pub path: String,
    /// Final filename after collision de-duplication, e.g. `image-1.png`.
    pub name: String,
}

/// Error type for the file-management editing endpoints
/// (`/.mbr/create`, `/.mbr/move`, `/.mbr/mkdir`, `/.mbr/upload`). Mirrors the
/// inline `(StatusCode, &str)` convention used by the other edit handlers rather
/// than the crate-wide `MbrError`.
#[derive(Debug)]
enum FileOpError {
    /// The target path already exists (create/move collision) → `409`.
    AlreadyExists,
    /// The destination parent directory does not exist (and `create_dirs`
    /// was not set) → `400`.
    ParentMissing,
    /// The destination lacks a configured markdown extension → `400`.
    NotMarkdown,
    /// An uploaded filename was empty, contained path separators / `..`, lacked
    /// an extension, carried a markdown extension (markdown goes through
    /// `/.mbr/create`), or was not an allowed media type → `400`.
    InvalidUploadName,
    /// The upload destination is inside the template folder (`.mbr/` or
    /// `--template-folder`), where a file would be executed as a template or
    /// served as site JavaScript → `400`.
    ForbiddenUploadDir,
    /// The move source is not an existing markdown file → `404`.
    SourceNotFound,
    /// The path escaped the repository root (traversal/symlink) → `400`.
    Traversal,
    /// A filesystem error occurred → `500`.
    Io(std::io::Error),
}

impl IntoResponse for FileOpError {
    fn into_response(self) -> Response {
        let (status, msg): (StatusCode, &'static str) = match self {
            FileOpError::AlreadyExists => (StatusCode::CONFLICT, "Target already exists"),
            FileOpError::ParentMissing => (
                StatusCode::BAD_REQUEST,
                "Destination parent directory does not exist (set create_dirs to create it)",
            ),
            FileOpError::NotMarkdown => (
                StatusCode::BAD_REQUEST,
                "Path must end in a markdown extension",
            ),
            FileOpError::InvalidUploadName => (
                StatusCode::BAD_REQUEST,
                "Invalid upload filename (must be a basename with an allowed media extension)",
            ),
            FileOpError::ForbiddenUploadDir => (
                StatusCode::BAD_REQUEST,
                "Uploads into the template folder are not allowed",
            ),
            FileOpError::SourceNotFound => {
                (StatusCode::NOT_FOUND, "Source markdown file not found")
            }
            FileOpError::Traversal => (StatusCode::BAD_REQUEST, "Invalid path"),
            FileOpError::Io(e) => {
                tracing::error!("file operation I/O error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "I/O error")
            }
        };
        (status, msg).into_response()
    }
}

/// Resolves a repo-relative path (that may not yet exist) against an
/// already-canonical repository root, guarding against traversal/symlink
/// escape. Shared core of [`Server::resolve_new_target`], factored out for
/// unit testing.
///
/// Rejects any `..` or absolute component, joins onto `canonical_base`, then
/// canonicalizes the deepest **existing** ancestor and asserts it stays within
/// the root (so a symlink in the existing portion cannot escape).
fn resolve_new_target_path(canonical_base: &Path, rel: &str) -> Result<PathBuf, FileOpError> {
    let clean = rel.trim_start_matches('/');
    if clean.is_empty() {
        return Err(FileOpError::Traversal);
    }
    // Reject `..`/absolute components before any filesystem access.
    for component in Path::new(clean).components() {
        match component {
            std::path::Component::Normal(_) | std::path::Component::CurDir => {}
            _ => return Err(FileOpError::Traversal),
        }
    }

    let candidate = canonical_base.join(clean);

    // Canonicalize the deepest existing ancestor to defeat symlink escape
    // through the already-existing portion of the path.
    let mut ancestor = candidate.as_path();
    let existing = loop {
        if ancestor.exists() {
            break ancestor;
        }
        match ancestor.parent() {
            Some(p) => ancestor = p,
            None => break ancestor,
        }
    };
    let canonical_existing = existing.canonicalize().map_err(FileOpError::Io)?;
    if !canonical_existing.starts_with(canonical_base) {
        return Err(FileOpError::Traversal);
    }

    Ok(candidate)
}

/// Percent-encode set for upload-response URLs. Encodes everything except the
/// RFC 3986 unreserved characters (`A-Z a-z 0-9 - . _ ~`) and the `/` path
/// separator, so the returned URL round-trips through mbr's request-path
/// percent-decoding and the browser can load the saved file (e.g. a space in a
/// filename becomes `%20`).
const UPLOAD_URL_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'/')
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Sanitizes a desired upload filename down to a safe basename.
///
/// Returns `None` (which callers map to `400`) when `name`:
/// - is empty or whitespace-only,
/// - contains a path separator (`/` or `\`) or a `..` sequence,
/// - is not a pure basename (has directory components),
/// - lacks a non-empty stem or a non-empty extension,
/// - carries a configured markdown extension (those go through `/.mbr/create`),
///   or
/// - carries an extension outside [`UPLOAD_ALLOWED_EXTENSIONS`].
///
/// The allowlist is the important half: rejecting only separators and markdown
/// left the media uploader able to write `index.html` or
/// `components/mbr-components.min.js`, which the watcher hot-reloads into every
/// rendered page.
///
/// On success returns the trimmed, validated basename unchanged.
fn sanitize_upload_name(name: &str, markdown_extensions: &[String]) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    // Reject path separators and parent-traversal outright: only a basename is
    // accepted. `\\` is rejected too so a Windows-style path can't sneak through.
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return None;
    }
    // The input must be exactly its own file name (no directory components,
    // and not `.`/`..` which have no file name).
    let path = Path::new(name);
    if path.file_name().and_then(|n| n.to_str()) != Some(name) {
        return None;
    }
    // Require a non-empty stem AND a non-empty extension.
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) if !e.is_empty() => e,
        _ => return None,
    };
    if stem.is_empty() {
        return None;
    }
    let ext_lower = ext.to_ascii_lowercase();
    // Markdown files must be created via `/.mbr/create`, not uploaded.
    if crate::repo::is_markdown_extension(&ext_lower, markdown_extensions) {
        return None;
    }
    // Only media types the rest of mbr already understands may be uploaded.
    if !UPLOAD_ALLOWED_EXTENSIONS.contains(&ext_lower.as_str()) {
        return None;
    }
    Some(name.to_string())
}

/// Finds a non-colliding destination path for `stem`.`ext` inside `dir`.
///
/// Returns `dir/stem.ext` when free, otherwise the first free
/// `dir/stem-N.ext` (N = 1, 2, 3, …). `exists` reports whether a candidate is
/// taken, so the collision loop is pure and unit-testable without touching the
/// filesystem.
fn dedupe_name(dir: &Path, stem: &str, ext: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    std::iter::once(dir.join(format!("{stem}.{ext}")))
        .chain((1u64..).map(|n| dir.join(format!("{stem}-{n}.{ext}"))))
        .find(|candidate| !exists(candidate))
        .expect("candidate sequence is infinite, so a free path always exists")
}

/// Serializable view of the repository for `/.mbr/site.json`.
///
/// Deliberately *not* `serde_json::to_value(&*repo)`: that materializes the
/// whole `other_files` subtree — one `Value` per asset, tens of thousands on a
/// media-heavy repo — only for the handler to delete the key again one
/// statement later. Naming the fields that actually ship means the media
/// catalog is never built here; it is served by `/.mbr/media.json`.
#[derive(serde::Serialize)]
struct SiteJson<'a> {
    index_file: &'a str,
    markdown_files: &'a crate::repo::MarkdownFiles,
    sort: &'a [SortField],
    sidebar_style: &'a str,
    sidebar_max_items: usize,
}

/// The non-repo values that appear in `/.mbr/site.json`.
///
/// Owned (not borrowed from `ServerState`) so the body can be rendered on the
/// blocking pool.
struct SiteJsonParams {
    sort: Vec<SortField>,
    sidebar_style: String,
    sidebar_max_items: usize,
    relationship_tracking: bool,
}

/// Renders the `/.mbr/site.json` body for the current repository snapshot.
///
/// Pure with respect to the caches: the caller decides whether to memoize.
/// Relationship injection still walks a `serde_json::Value`, because it
/// decorates each markdown entry, but that DOM now contains only what ships.
fn render_site_json(
    repo: &Repo,
    params: &SiteJsonParams,
) -> Result<axum::body::Bytes, serde_json::Error> {
    let mut value = serde_json::to_value(SiteJson {
        index_file: &repo.index_file,
        markdown_files: &repo.markdown_files,
        sort: &params.sort,
        sidebar_style: &params.sidebar_style,
        sidebar_max_items: params.sidebar_max_items,
    })?;

    // Add relationship_types + per-note resolved relationships (if enabled).
    if params.relationship_tracking {
        repo.relationship_index.inject_into_site_json(&mut value);
    }

    Ok(axum::body::Bytes::from(serde_json::to_vec(&value)?))
}

impl Server {
    /// Initialize a new server instance with the given configuration.
    pub fn init(config: ServerConfig) -> Result<Self, ServerError> {
        let ServerConfig {
            ip,
            port,
            base_dir,
            static_folder,
            markdown_extensions,
            ignore_dirs,
            ignore_globs,
            watcher_ignore_dirs,
            index_file,
            oembed_timeout_ms,
            oembed_cache_size,
            #[cfg(feature = "media-metadata")]
            media_cache_size,
            template_folder,
            sort,
            gui_mode,
            theme,
            log_filter,
            link_tracking,
            relationship_tracking,
            relationship_types,
            tag_sources,
            sidebar_style,
            sidebar_max_items,
            graph_depth,
            title_prefix,
            title_suffix,
            mark_incomplete,
            incomplete_markers,
            tasks_enabled,
            tasks_stamp_done,
            tasks_ignore_globs,
            edit_enabled,
            edit_require_token_on_loopback,
            edit_token_hash,
            upload_max_bytes,
            #[cfg(feature = "media-metadata")]
            transcode_enabled,
        } = config;

        let oembed_cache = Arc::new(OembedCache::new(oembed_cache_size));

        // Media metadata (cover JPEGs, chapters, captions) gets its own budget:
        // it used to borrow the oembed *text* cache size, so a couple of dozen
        // covers filled it and `--oembed-cache-size 0` silently disabled media
        // caching too.
        #[cfg(feature = "media-metadata")]
        let video_metadata_cache = Arc::new(VideoMetadataCache::new(media_cache_size));

        #[cfg(feature = "media-metadata")]
        let hls_cache = Arc::new(HlsCache::new(DEFAULT_HLS_CACHE_SIZE));

        // Single-flight guard + probed-resolution cache for media metadata/HLS.
        #[cfg(feature = "media-metadata")]
        let metadata_inflight: Arc<papaya::HashMap<String, Arc<tokio::sync::Notify>>> =
            Arc::new(papaya::HashMap::new());
        #[cfg(feature = "media-metadata")]
        let video_resolution_cache: Arc<
            papaya::HashMap<String, crate::video_transcode::VideoResolution>,
        > = Arc::new(papaya::HashMap::new());
        #[cfg(feature = "media-metadata")]
        let media_compat_cache: Arc<
            papaya::HashMap<String, crate::video_metadata::PlaybackCompatibility>,
        > = Arc::new(papaya::HashMap::new());

        // Listing caches (per-directory files, per-directory subdirectories and
        // the serialized site.json body). Created before the file-change
        // invalidation task so that task can clear them when files change.
        let listing_caches = ListingCaches {
            sibling_nav_cache: Arc::new(papaya::HashMap::new()),
            subdir_cache: Arc::new(papaya::HashMap::new()),
            site_json_cache: Arc::new(parking_lot::RwLock::new(SiteJsonCache::default())),
        };

        // Use try_init to allow multiple server instances in tests
        // RUST_LOG env var takes precedence, then CLI flag, then default (warn)
        let default_filter = log_filter.as_deref().unwrap_or("mbr=warn,tower_http=warn");
        let _ = tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| default_filter.into()),
            )
            .with(tracing_subscriber::fmt::layer())
            .try_init();

        let templates = templates::Templates::new(base_dir.as_path(), template_folder.as_deref())
            .map_err(ServerError::TemplateInit)?;

        let repo = Arc::new(Repo::init(
            &base_dir,
            &static_folder,
            &markdown_extensions,
            &ignore_dirs,
            &ignore_globs,
            &index_file,
            &tag_sources,
            &relationship_types,
        ));

        // Spawn background repo scan so site.json is ready before first request.
        // Phase 1: basic scan (file listing + frontmatter). Phase 2: media metadata (ffmpeg/lopdf).
        let repo_for_scan = Arc::clone(&repo);
        tokio::task::spawn_blocking(move || {
            if let Err(e) = repo_for_scan.scan_all() {
                tracing::error!("Background scan failed: {e}");
            }
            // Build the relationship index once all note titles are known.
            repo_for_scan.build_relationship_index();
            // Build the global wikilink name index (always on) so body
            // `[[Name]]` links resolve globally on first render.
            repo_for_scan.build_wikilink_index();
            repo_for_scan.mark_scan_complete();

            // Phase 1.5: scan static folder (deferred from scan_all for faster search)
            if let Err(e) = repo_for_scan.scan_static_folder() {
                tracing::error!("Background static scan failed: {e}");
            }

            // Phase 2: populate basic file metadata (stat calls for size/timestamps)
            repo_for_scan.populate_basic_metadata();

            // Phase 3: populate media metadata in background (non-blocking for site.json)
            repo_for_scan.populate_media_metadata();
            repo_for_scan.notify_media_populated();

            // Phase 4: extract text from PDFs/text files for search
            repo_for_scan.ensure_text_extracted();
        });

        // Create a broadcast channel for file changes - watcher will be initialized in background
        let (file_change_tx, _rx) = tokio::sync::broadcast::channel::<
            crate::watcher::FileChangeEvent,
        >(crate::watcher::BROADCAST_CAPACITY);
        let tx_for_watcher = file_change_tx.clone();

        // Initialize file watcher in background to avoid blocking server startup
        let base_dir_for_watcher = base_dir.clone();
        let template_folder_for_watcher = template_folder.clone();
        let watcher_ignore_dirs_for_watcher = watcher_ignore_dirs.clone();
        let ignore_globs_for_watcher = ignore_globs.clone();

        // Create a handle to store the watcher once it's initialized.
        // This ensures proper cleanup when Server is dropped.
        let watcher_handle: Arc<std::sync::Mutex<Option<crate::watcher::FileWatcher>>> =
            Arc::new(std::sync::Mutex::new(None));
        let watcher_handle_for_thread = Arc::clone(&watcher_handle);

        std::thread::spawn(move || {
            match crate::watcher::FileWatcher::new_with_sender(
                &base_dir_for_watcher,
                template_folder_for_watcher.as_deref(),
                &watcher_ignore_dirs_for_watcher,
                &ignore_globs_for_watcher,
                tx_for_watcher,
            ) {
                Ok(watcher) => {
                    tracing::info!("File watcher initialized successfully (background)");
                    // Store the watcher in the shared handle so it stays alive
                    // and can be properly dropped when Server is dropped
                    if let Ok(mut guard) = watcher_handle_for_thread.lock() {
                        *guard = Some(watcher);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to initialize file watcher: {}. Live reload disabled.",
                        e
                    );
                }
            }
        });

        // Spawn background task to reload templates when .html files change
        let templates_for_reload = templates.clone();
        // Canonicalize once here, not per event: the watcher reports canonical
        // paths while the configured folder is stored as written, and this
        // cannot change for the life of the process. Fall back to the path as
        // given when canonicalization fails (e.g. the folder does not exist).
        let template_folder_for_reload = template_folder
            .clone()
            .map(|tf| tf.canonicalize().unwrap_or(tf));
        let mut template_change_rx = file_change_tx.subscribe();
        tokio::spawn(async move {
            loop {
                // `while let Ok(..)` here treated a `Lagged` error as the end of
                // the stream, so one burst of file changes silently disabled
                // template hot reload for the life of the process — the same
                // defect as the live-reload WebSocket loop.
                let event = match live_reload_action(template_change_rx.recv().await) {
                    LiveReloadAction::Forward(event) => event,
                    LiveReloadAction::Skip => continue,
                    LiveReloadAction::Close => break,
                };

                // Only reload for .html files
                if !event.path.ends_with(".html") {
                    continue;
                }

                // If we have a template folder, only reload for changes in that folder
                // Otherwise, only reload for changes in .mbr folder
                if should_reload_template(&event.path, template_folder_for_reload.as_deref()) {
                    tracing::debug!("Template file changed: {}", event.path);
                    if let Err(e) = templates_for_reload.reload() {
                        tracing::error!("Failed to reload templates: {}", e);
                    }
                }
            }
        });

        // Link caches. Created before the file-change invalidation task so that
        // task can drop them when files change: both are derived from file
        // contents and neither the outbound `LinkCache` nor a served links.json
        // response re-checks mtimes, so a missed invalidation serves pre-edit
        // links until the process restarts.
        let link_cache = Arc::new(LinkCache::new(DEFAULT_LINK_CACHE_SIZE));
        let inbound_link_cache = Arc::new(InboundLinkCache::new(
            DEFAULT_INBOUND_LINK_CACHE_SIZE,
            INBOUND_LINK_CACHE_TTL_SECS,
        ));

        // Repository-wide backlink index. Built once, in the background, after
        // the initial scan (which is what makes the wikilink index — and so
        // link resolution — trustworthy). Until it is ready, links.json falls
        // back to the per-page grep, so startup is not blocked on it.
        let inbound_index = Arc::new(InboundIndex::new());
        // Serializes the initial full build against the watcher's incremental
        // updates. Both mutate the same index, and an update that interleaves
        // with the build can be silently undone by it.
        let index_lock = Arc::new(tokio::sync::Mutex::new(()));
        let link_index_config = LinkIndexConfig {
            base_dir: base_dir.clone(),
            index_file: index_file.clone(),
            markdown_extensions: markdown_extensions.clone(),
            valid_tag_sources: crate::config::tag_sources_to_set(&tag_sources),
        };
        if link_tracking {
            let repo_for_index = Arc::clone(&repo);
            let index_for_build = Arc::clone(&inbound_index);
            let cfg_for_build = link_index_config.clone();
            let index_lock_for_build = Arc::clone(&index_lock);
            tokio::spawn(async move {
                repo_for_index.wait_for_scan().await;
                let _guard = index_lock_for_build.lock().await;
                let repo = Arc::clone(&repo_for_index);
                tokio::task::spawn_blocking(move || {
                    populate_inbound_index(&repo, &index_for_build, &cfg_for_build);
                })
                .await
                .unwrap_or_else(|e| tracing::error!("Backlink index build task failed: {e}"));
            });
        }

        // Lazy task index. Created here so the watcher can keep it fresh, but
        // deliberately left *empty*: it is filled by the first `/.mbr/tasks`
        // request, and `invalidate_file` is a no-op until then, so a server
        // whose user never opens the task panel never reads a file for it.
        // The ignore patterns are compiled once, by the constructor.
        let task_index = Arc::new(crate::task_index::TaskIndex::new(&tasks_ignore_globs));

        // Spawn background task to invalidate repo cache when files change.
        // Uses debouncing: accumulate events for 2 seconds, then apply changes.
        // For small batches (<=50 files): surgical per-file invalidation.
        // For large batches: full clear + rescan.
        let repo_for_invalidation = Arc::clone(&repo);
        let base_dir_for_invalidation = base_dir.clone();
        let markdown_extensions_for_invalidation = markdown_extensions.clone();
        let listing_caches_for_invalidation = listing_caches.clone();
        let link_cache_for_invalidation = Arc::clone(&link_cache);
        let inbound_link_cache_for_invalidation = Arc::clone(&inbound_link_cache);
        let inbound_index_for_invalidation = Arc::clone(&inbound_index);
        let link_index_config_for_invalidation = link_index_config.clone();
        let index_lock_for_invalidation = Arc::clone(&index_lock);
        let task_index_for_invalidation = Arc::clone(&task_index);
        let mut repo_change_rx = file_change_tx.subscribe();
        tokio::spawn(async move {
            const DEBOUNCE_DURATION: std::time::Duration = std::time::Duration::from_secs(2);
            const SURGICAL_THRESHOLD: usize = 50;

            loop {
                // Wait for the first event
                let first_event = match repo_change_rx.recv().await {
                    Ok(e) => e,
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                };

                // Accumulate events during the debounce window
                let mut pending_events = vec![first_event];
                let deadline = tokio::time::Instant::now() + DEBOUNCE_DURATION;

                loop {
                    tokio::select! {
                        result = repo_change_rx.recv() => {
                            match result {
                                Ok(event) => pending_events.push(event),
                                Err(broadcast::error::RecvError::Closed) => break,
                                Err(broadcast::error::RecvError::Lagged(_)) => {
                                    // Too many events queued — force full rescan
                                    pending_events.clear();
                                    pending_events.push(crate::watcher::FileChangeEvent {
                                        path: String::new(),
                                        relative_path: String::new(),
                                        event: crate::watcher::ChangeEventType::Created,
                                    });
                                    // Push over threshold to trigger full rescan
                                    for _ in 0..SURGICAL_THRESHOLD {
                                        pending_events.push(crate::watcher::FileChangeEvent {
                                            path: String::new(),
                                            relative_path: String::new(),
                                            event: crate::watcher::ChangeEventType::Created,
                                        });
                                    }
                                    break;
                                }
                            }
                        }
                        _ = tokio::time::sleep_until(deadline) => break,
                    }
                }

                // Filter to only relevant events
                let relevant_events: Vec<_> = pending_events
                    .into_iter()
                    .filter(|event| match event.event {
                        crate::watcher::ChangeEventType::Created
                        | crate::watcher::ChangeEventType::Deleted => true,
                        crate::watcher::ChangeEventType::Modified => {
                            markdown_extensions_for_invalidation
                                .iter()
                                .any(|ext| event.relative_path.ends_with(&format!(".{}", ext)))
                        }
                    })
                    .collect();

                if relevant_events.is_empty() {
                    continue;
                }

                let repo = Arc::clone(&repo_for_invalidation);
                let base_dir = base_dir_for_invalidation.clone();
                let inbound_index = Arc::clone(&inbound_index_for_invalidation);
                let link_index_cfg = link_index_config_for_invalidation.clone();
                let index_lock = Arc::clone(&index_lock_for_invalidation);
                let task_index = Arc::clone(&task_index_for_invalidation);

                if relevant_events.len() <= SURGICAL_THRESHOLD {
                    // Surgical invalidation: update individual files
                    tracing::debug!(
                        "Surgical invalidation for {} file(s)",
                        relevant_events.len()
                    );
                    let has_tag_changes = relevant_events.iter().any(|e| {
                        matches!(
                            e.event,
                            crate::watcher::ChangeEventType::Deleted
                                | crate::watcher::ChangeEventType::Modified
                        )
                    });

                    // Serialized against the initial index build; see the
                    // `is_ready()` comment below.
                    let _index_guard = index_lock.lock().await;
                    tokio::task::spawn_blocking(move || {
                        let mut index_targets: Vec<(PathBuf, Option<String>, bool)> = Vec::new();
                        for event in &relevant_events {
                            let abs_path = if event.path.is_empty() {
                                continue;
                            } else {
                                PathBuf::from(&event.path)
                            };
                            index_targets.push((
                                abs_path.clone(),
                                // Take the URL from the repository's own map
                                // rather than recomputing it. `base_dir` is the
                                // path as configured, while the scanner keys
                                // everything by the *canonical* root, and on
                                // macOS a temp dir differs (`/var` vs
                                // `/private/var`). A recomputed URL would not
                                // match the key the initial build inserted
                                // under, so the edit would write a second entry
                                // and never withdraw the stale one. Captured
                                // before `invalidate_file` because a deletion
                                // removes the entry it is read from.
                                repo.markdown_files
                                    .pin()
                                    .get(&abs_path)
                                    .map(|info| info.url_path.clone()),
                                matches!(event.event, crate::watcher::ChangeEventType::Deleted),
                            ));
                            repo.invalidate_file(&abs_path, &event.event);
                            // After `repo.invalidate_file`, never before: a
                            // created or modified file's url/title are read
                            // back out of the repository's own map, which the
                            // call above is what populates. A no-op until the
                            // task index has actually been built.
                            task_index.invalidate_file(
                                &abs_path,
                                &event.event,
                                &repo,
                                &link_index_cfg.base_dir,
                            );
                        }
                        // Rebuild tag index if any files were deleted or modified
                        // (created files add tags inline in invalidate_file)
                        if has_tag_changes {
                            repo.rebuild_tag_index();
                        }
                        // Relationships may have changed on any create/modify/delete;
                        // rebuild the index so endpoint resolution stays consistent.
                        repo.build_relationship_index();
                        // The global wikilink index must track the same changes.
                        repo.build_wikilink_index();

                        // Re-index only the pages that changed. This runs after
                        // build_wikilink_index() on purpose: `[[Name]]` links
                        // resolve through that index, so re-extracting first
                        // would record links against the pre-edit name table.
                        //
                        // `is_ready()` gates it because the initial build is
                        // still filling the index otherwise. That is safe only
                        // because the caller holds `index_lock` across this
                        // whole task: without it, an edit arriving mid-build
                        // would be skipped here and then overwritten by the
                        // build's pre-edit contents, leaving links.json stale
                        // until the next full rescan.
                        if inbound_index.is_ready() {
                            for (abs_path, known_url, is_delete) in &index_targets {
                                if !link_index_cfg.markdown_extensions.iter().any(|ext| {
                                    abs_path
                                        .extension()
                                        .and_then(|e| e.to_str())
                                        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
                                }) {
                                    continue;
                                }
                                let url_path = match known_url {
                                    Some(url) => url.clone(),
                                    // Only reachable for a file the scan never
                                    // saw (created inside this batch, or
                                    // deleted before it was ever indexed).
                                    None => crate::repo::build_markdown_url_path(
                                        abs_path,
                                        &link_index_cfg.base_dir,
                                        &link_index_cfg.index_file,
                                    ),
                                };
                                if *is_delete {
                                    inbound_index.remove_page(&url_path);
                                } else {
                                    index_page_links(
                                        &repo,
                                        &inbound_index,
                                        &link_index_cfg,
                                        abs_path,
                                        &url_path,
                                    );
                                }
                            }
                        }
                    })
                    .await
                    .ok();
                } else {
                    // Too many changes — full rescan
                    tracing::info!(
                        "Full rescan triggered: {} file changes exceed threshold",
                        relevant_events.len()
                    );
                    tokio::task::spawn_blocking(move || {
                        repo.full_rescan();
                        // Every page may have changed, so rebuild rather than
                        // patch. `populate_inbound_index` re-marks it ready; the
                        // stale index stays queryable in the meantime, which is
                        // better than falling back to a full-repo grep per page.
                        if inbound_index.is_ready() {
                            populate_inbound_index(&repo, &inbound_index, &link_index_cfg);
                        }
                        // Same reasoning for tasks: too many files moved for a
                        // per-file patch to be cheaper than one read pass. Still
                        // a no-op unless somebody has actually used tasks.
                        task_index.rebuild_if_built(&repo, &link_index_cfg.base_dir);
                        let _ = base_dir; // keep alive for potential future use
                    })
                    .await
                    .ok();
                }

                // The repository changed, so every cache derived from file
                // contents may be stale. Drop them all; they are rebuilt lazily
                // on the next render/links.json from the freshly invalidated
                // repo.
                invalidate_derived_caches(
                    &listing_caches_for_invalidation,
                    &link_cache_for_invalidation,
                    &inbound_link_cache_for_invalidation,
                );
            }
        });

        let inbound_grep_semaphore =
            Arc::new(tokio::sync::Semaphore::new(INBOUND_GREP_MAX_CONCURRENCY));
        let inbound_grep_inflight: Arc<papaya::HashMap<String, Arc<tokio::sync::Notify>>> =
            Arc::new(papaya::HashMap::new());

        let canonical_base_dir = base_dir.canonicalize().ok();
        let state = ServerState {
            bind_ip: ip,
            base_dir,
            canonical_base_dir,
            static_folder,
            markdown_extensions,
            ignore_dirs,
            ignore_globs,
            index_file,
            templates,
            repo,
            oembed_timeout_ms,
            file_change_tx: Some(file_change_tx),
            template_folder,
            sort,
            gui_mode,
            theme,
            oembed_cache,
            #[cfg(feature = "media-metadata")]
            video_metadata_cache,
            #[cfg(feature = "media-metadata")]
            transcode_enabled,
            #[cfg(feature = "media-metadata")]
            hls_cache,
            #[cfg(feature = "media-metadata")]
            metadata_inflight,
            #[cfg(feature = "media-metadata")]
            video_resolution_cache,
            #[cfg(feature = "media-metadata")]
            media_compat_cache,
            sibling_nav_cache: listing_caches.sibling_nav_cache,
            subdir_cache: listing_caches.subdir_cache,
            site_json_cache: listing_caches.site_json_cache,
            link_tracking,
            relationship_tracking,
            relationship_types,
            link_cache,
            inbound_link_cache,
            inbound_index,
            inbound_grep_semaphore,
            inbound_grep_inflight,
            tag_sources,
            sidebar_style,
            sidebar_max_items,
            graph_depth,
            title_prefix,
            title_suffix,
            mark_incomplete,
            incomplete_markers,
            tasks_enabled,
            tasks_stamp_done,
            task_index,
            edit_enabled,
            edit_require_token_on_loopback,
            edit_token_hash,
            upload_max_bytes,
        };

        let router = Router::new()
            .route("/", get(Self::home_page))
            .route("/.mbr/site.json", get(Self::get_site_info))
            .route("/.mbr/media.json", get(Self::get_media_info))
            .route("/.mbr/search", post(Self::search_handler))
            // Task browser query endpoint (gated by tasks_enabled; 404 when off)
            .route("/.mbr/tasks", post(Self::tasks_handler))
            // Editing endpoints: raw source fetch and save (gated by edit_enabled + auth)
            .route("/.mbr/raw/{*path}", get(Self::raw_markdown_handler))
            .route(
                "/.mbr/edit/{*path}",
                post(Self::save_markdown_handler)
                    // Cap edit payloads at 5 MB (axum default is 2 MB).
                    .layer(DefaultBodyLimit::max(5 * 1024 * 1024)),
            )
            // Single-line task toggle (gated by edit_enabled + auth, like the
            // rest of the write endpoints). Singular `/task`, next to the plural
            // `/tasks` query above.
            .route("/.mbr/task", post(Self::task_toggle_handler))
            // File-management endpoints (gated by edit_enabled + auth): create a
            // new file, move/rename with repo-wide link rewrite, create a folder.
            .route(
                "/.mbr/create/{*path}",
                post(Self::create_markdown_handler)
                    // Cap create payloads at 5 MB (axum default is 2 MB).
                    .layer(DefaultBodyLimit::max(5 * 1024 * 1024)),
            )
            .route("/.mbr/move/{*path}", post(Self::move_markdown_handler))
            .route("/.mbr/mkdir/{*path}", post(Self::mkdir_handler))
            // Binary asset upload (the editor's image uploader). Body limit comes
            // from `upload_max_bytes`; oversize bodies get 413 automatically.
            .route(
                "/.mbr/upload",
                post(Self::upload_handler).layer(DefaultBodyLimit::max(upload_max_bytes)),
            )
            .route("/.mbr/ws/changes", get(Self::websocket_handler))
            // Media viewer routes - must be before the catch-all /.mbr/{*path}
            .route("/.mbr/videos/", get(Self::serve_media_viewer))
            .route("/.mbr/pdfs/", get(Self::serve_media_viewer))
            .route("/.mbr/audio/", get(Self::serve_media_viewer))
            .route("/.mbr/images/", get(Self::serve_media_viewer))
            .route("/.mbr/{*path}", get(Self::serve_mbr_assets))
            .route("/{*path}", get(Self::handle))
            .layer(CompressionLayer::new().compress_when(compression_predicate()))
            .layer(TraceLayer::new_for_http())
            .with_state(state);

        Ok(Server {
            router,
            ip,
            port,
            gui_mode,
            _watcher_handle: watcher_handle,
        })
    }

    /// Announces the bound address once the listener is up.
    ///
    /// In server mode (`-s`) the URL is the whole point of the command and the
    /// user is watching that terminal, so it goes to stdout unchanged. In GUI
    /// mode the window is the affordance and the banner is noise: on Windows a
    /// console-subsystem binary launched from Explorer gets a console window,
    /// and the line lands there, behind the webview the user is actually
    /// looking at. It becomes a `tracing::info!` instead, so `-v` still
    /// surfaces it while the default `warn` level keeps it hidden.
    ///
    /// The decision reads `self.gui_mode` rather than relying on which start
    /// method was called. `start_with_port_retry` happens to be GUI-only today,
    /// but that is incidental and would rot silently the first time a
    /// non-GUI caller wanted port retry.
    fn announce_listening(&self, local_addr: SocketAddr) {
        tracing::debug!("listening on {}", local_addr);
        if self.gui_mode {
            tracing::info!("Server running at http://{}/", local_addr);
        } else {
            println!("Server running at http://{}/", local_addr);
        }
    }

    pub async fn start(&self) -> Result<(), ServerError> {
        self.start_with_ready_signal(None).await
    }

    /// Starts the server and optionally signals when ready to accept connections.
    /// If a sender is provided, it will receive `()` once the server is bound and listening.
    pub async fn start_with_ready_signal(
        &self,
        ready_tx: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> Result<(), ServerError> {
        let addr = SocketAddr::from((self.ip, self.port));
        let listener =
            tokio::net::TcpListener::bind(addr)
                .await
                .map_err(|e| ServerError::BindFailed {
                    addr: addr.to_string(),
                    source: e,
                })?;
        let local_addr = listener
            .local_addr()
            .map_err(ServerError::LocalAddrFailed)?;
        self.announce_listening(local_addr);

        // Signal that server is ready before starting to serve
        if let Some(tx) = ready_tx
            && tx.send(()).is_err()
        {
            tracing::debug!("Ready signal receiver dropped (shutdown in progress)");
        }

        axum::serve(
            listener,
            self.router
                .clone()
                .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .map_err(ServerError::StartFailed)?;
        Ok(())
    }

    /// Starts the server with automatic port retry on address-in-use errors.
    ///
    /// If the configured port is already in use, this method will try incrementing
    /// the port (up to `max_retries` times) until it finds an available port.
    /// A warning is printed to stderr when the port is incremented.
    ///
    /// If a sender is provided, it will receive the actual bound port once the
    /// server is listening.
    pub async fn start_with_port_retry(
        &mut self,
        ready_tx: Option<tokio::sync::oneshot::Sender<u16>>,
        max_retries: u16,
    ) -> Result<(), ServerError> {
        let mut attempts = 0;

        loop {
            let addr = SocketAddr::from((self.ip, self.port));
            match tokio::net::TcpListener::bind(addr).await {
                Ok(listener) => {
                    let local_addr = listener
                        .local_addr()
                        .map_err(ServerError::LocalAddrFailed)?;
                    self.announce_listening(local_addr);

                    // Signal that server is ready with the actual port
                    if let Some(tx) = ready_tx
                        && tx.send(self.port).is_err()
                    {
                        tracing::debug!("Port signal receiver dropped (shutdown in progress)");
                    }

                    axum::serve(
                        listener,
                        self.router
                            .clone()
                            .into_make_service_with_connect_info::<SocketAddr>(),
                    )
                    .await
                    .map_err(ServerError::StartFailed)?;
                    return Ok(());
                }
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && attempts < max_retries => {
                    let old_port = self.port;
                    // Fail fast if we've hit the maximum port number
                    if self.port == 65535 {
                        return Err(ServerError::BindFailed {
                            addr: "port range exhausted (reached port 65535)".into(),
                            source: e,
                        });
                    }
                    self.port += 1;
                    attempts += 1;
                    eprintln!(
                        "Warning: Port {} already in use, trying port {}",
                        old_port, self.port
                    );
                    tracing::warn!(
                        "Port {} already in use, trying port {}",
                        old_port,
                        self.port
                    );
                }
                Err(e) => {
                    return Err(ServerError::BindFailed {
                        addr: addr.to_string(),
                        source: e,
                    });
                }
            }
        }
    }

    /// WebSocket handler for live reload file change notifications.
    ///
    /// # Security
    ///
    /// WebSocket handshakes are exempt from the same-origin policy, so without
    /// this check any page the user happens to visit could open
    /// `ws://127.0.0.1:<port>/.mbr/ws/changes` and watch the private
    /// file-change feed in real time. Upgrades must therefore be same-origin,
    /// and a handshake with **no** `Origin` is rejected as well (browsers
    /// always send one on a WebSocket upgrade).
    pub async fn websocket_handler(
        ws: WebSocketUpgrade,
        State(config): State<ServerState>,
        headers: HeaderMap,
    ) -> Response {
        if !headers.contains_key(header::ORIGIN) || !Self::is_same_origin(&headers) {
            tracing::warn!("Blocked cross-origin live-reload WebSocket upgrade");
            return (
                StatusCode::FORBIDDEN,
                "Cross-origin WebSocket upgrade blocked",
            )
                .into_response();
        }
        ws.on_upgrade(|socket| Self::handle_websocket(socket, config))
            .into_response()
    }

    async fn handle_websocket(socket: axum::extract::ws::WebSocket, config: ServerState) {
        let (mut sender, mut receiver) = socket.split();

        // If file watcher is not initialized, close the connection
        let Some(file_change_tx) = config.file_change_tx else {
            tracing::warn!("WebSocket connection attempted but file watcher is disabled");
            if let Err(e) = sender
                .send(axum::extract::ws::Message::Text(
                    r#"{"error":"File watcher not available"}"#.into(),
                ))
                .await
            {
                tracing::debug!("Failed to send error to WebSocket client: {e}");
            }
            return;
        };

        // Subscribe to file change events
        let mut rx = file_change_tx.subscribe();

        tracing::info!("WebSocket client connected for live reload");

        // Send initial connection confirmation
        if sender
            .send(axum::extract::ws::Message::Text(
                r#"{"status":"connected"}"#.to_string().into(),
            ))
            .await
            .is_err()
        {
            return;
        }

        // Handle bidirectional communication
        loop {
            tokio::select! {
                // Forward file change events to the client. The whole result is
                // bound (never matched with a refutable `Ok(..)` pattern): a
                // pattern mismatch permanently disables the branch for that
                // `select!`, so a single `Lagged` would silently kill live
                // reload for this tab with the socket still open.
                result = rx.recv() => {
                    let change_event = match live_reload_action(result) {
                        LiveReloadAction::Forward(event) => event,
                        LiveReloadAction::Skip => continue,
                        LiveReloadAction::Close => break,
                    };

                    let json = match serde_json::to_string(&change_event) {
                        Ok(j) => j,
                        Err(e) => {
                            tracing::error!("Failed to serialize change event: {}", e);
                            continue;
                        }
                    };

                    if sender
                        .send(axum::extract::ws::Message::Text(json.into()))
                        .await
                        .is_err()
                    {
                        tracing::info!("WebSocket client disconnected");
                        break;
                    }
                }

                // Handle incoming messages from client (mostly for connection health)
                msg = receiver.next() => {
                    match msg {
                        Some(Ok(axum::extract::ws::Message::Close(_))) => {
                            tracing::info!("WebSocket client closed connection");
                            break;
                        }
                        #[allow(clippy::collapsible_match)]
                        Some(Ok(axum::extract::ws::Message::Ping(data))) => {
                            if sender
                                .send(axum::extract::ws::Message::Pong(data))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Some(Err(e)) => {
                            tracing::error!("WebSocket error: {}", e);
                            break;
                        }
                        None => {
                            tracing::info!("WebSocket stream ended");
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Serves `/.mbr/site.json` from a cached body.
    ///
    /// The body only changes when the repository does, so it is built once and
    /// reused until `invalidate_derived_caches` drops it. Building it is
    /// blocking CPU work on a large repo, so the (rare) rebuild runs on the
    /// blocking pool rather than stalling an async worker.
    pub async fn get_site_info(
        State(config): State<ServerState>,
    ) -> Result<impl IntoResponse, StatusCode> {
        // Wait for background scan to complete (watcher handles updates)
        if !config.repo.is_scan_complete() {
            tracing::debug!("get_site_info: waiting for background scan...");
            config.repo.wait_for_scan().await;
        }

        let (generation, cached) = config.site_json_cache.read().snapshot();
        let body = match cached {
            Some(body) => body,
            None => {
                let json_start = std::time::Instant::now();
                let repo = Arc::clone(&config.repo);
                let params = SiteJsonParams {
                    sort: config.sort.clone(),
                    sidebar_style: config.sidebar_style.clone(),
                    sidebar_max_items: config.sidebar_max_items,
                    relationship_tracking: config.relationship_tracking,
                };
                let built = tokio::task::spawn_blocking(move || render_site_json(&repo, &params))
                    .await
                    .map_err(|e| {
                        tracing::error!("site.json render task failed: {e}");
                        StatusCode::INTERNAL_SERVER_ERROR
                    })?
                    .inspect_err(|e| tracing::error!("Error serializing site json: {e}"))
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                // Publish only if the repository did not change while we built
                // (a concurrent request storing an equivalent body is harmless:
                // both were built from the same generation). This response
                // still carries the snapshot the request was answered from.
                config
                    .site_json_cache
                    .write()
                    .store(generation, built.clone());
                tracing::debug!(
                    "get_site_info JSON serialization: {:?}",
                    json_start.elapsed()
                );
                built
            }
        };

        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .inspect_err(|e| tracing::error!("Error rendering site file: {e}"))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
            .map(IntoResponse::into_response)
    }

    /// Returns media metadata (other_files) as JSON.
    ///
    /// Waits for both the initial scan and media metadata population to complete
    /// before returning, ensuring rich metadata (dimensions, duration, etc.) is available.
    pub async fn get_media_info(
        State(config): State<ServerState>,
    ) -> Result<impl IntoResponse, StatusCode> {
        let start = std::time::Instant::now();

        // Wait for background scan + media metadata population (watcher handles updates)
        if !config.repo.is_scan_complete() {
            tracing::debug!("get_media_info: waiting for background scan...");
            config.repo.wait_for_scan().await;
        }
        if !config.repo.is_media_populated() {
            tracing::debug!("get_media_info: waiting for media metadata...");
            config.repo.wait_for_media().await;
        }

        // Build response with only other_files
        let json_start = std::time::Instant::now();
        let media_data = serde_json::json!({
            "other_files": &config.repo.other_files,
        });

        let resp = Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(
                serde_json::to_string(&media_data)
                    .inspect_err(|e| tracing::error!("Error serializing media json: {e}"))
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
            )
            .inspect_err(|e| tracing::error!("Error rendering media file: {e}"))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        tracing::debug!(
            "get_media_info completed in {:?} (JSON: {:?})",
            start.elapsed(),
            json_start.elapsed()
        );
        Ok(resp.into_response())
    }

    /// Search endpoint for finding files by metadata and content.
    ///
    /// POST /.mbr/search
    ///
    /// Request body (JSON):
    /// ```json
    /// {
    ///   "q": "search query",
    ///   "limit": 50,           // optional, default 50
    ///   "scope": "all",        // "metadata", "content", or "all"
    ///   "filetype": "markdown",// optional filter
    ///   "folder": "/docs"      // optional folder scope
    /// }
    /// ```
    ///
    /// Response (JSON):
    /// ```json
    /// {
    ///   "query": "search query",
    ///   "total_matches": 42,
    ///   "results": [...],
    ///   "duration_ms": 15
    /// }
    /// ```
    pub async fn search_handler(
        State(config): State<ServerState>,
        Json(query): Json<SearchQuery>,
    ) -> impl IntoResponse {
        tracing::debug!("Search request: q={:?}, scope={:?}", query.q, query.scope);

        // Don't wait for scan — search with whatever files are available now.
        let scan_in_progress = !config.repo.is_scan_complete();

        // Only extract text for non-markdown searches (gated on scan completion)
        if config.repo.is_scan_complete()
            && (query.filetype.as_deref() == Some("all")
                || (query.filetype.is_some() && query.filetype.as_deref() != Some("markdown")))
        {
            config.repo.ensure_text_extracted();
        }

        let repo = config.repo.clone();
        let base_dir = config.base_dir.clone();

        // Clone query string for error handling (query is moved into closure)
        let query_str = query.q.clone();

        // Run search on blocking thread pool (grep does synchronous I/O)
        let search_result = tokio::task::spawn_blocking(move || {
            let engine = SearchEngine::new(repo.clone(), base_dir);
            let mut response = engine.search(&query)?;

            // If searching all filetypes or non-markdown, also search other files
            if query.filetype.as_deref() == Some("all")
                || (query.filetype.is_some() && query.filetype.as_deref() != Some("markdown"))
            {
                let other_results = search_other_files(
                    &repo,
                    &query.q,
                    query.folder.as_deref(),
                    query.filetype.as_deref(),
                    query.limit,
                );

                // Merge and re-sort
                response.results.extend(other_results);
                response.results.sort_by_key(|r| std::cmp::Reverse(r.score));
                response.results.truncate(query.limit);
                response.total_matches = response.results.len();
            }

            Ok::<_, crate::errors::SearchError>(response)
        })
        .await;

        let mut response = match search_result {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                tracing::error!("Search error: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": format!("Search failed: {}", e),
                        "query": query_str,
                        "total_matches": 0,
                        "results": [],
                        "duration_ms": 0
                    })),
                );
            }
            Err(e) => {
                tracing::error!("Search task panicked: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": "Search task failed",
                        "query": query_str,
                        "total_matches": 0,
                        "results": [],
                        "duration_ms": 0
                    })),
                );
            }
        };

        response.scan_in_progress = scan_in_progress;

        tracing::debug!(
            "Search completed: {} results in {}ms",
            response.total_matches,
            response.duration_ms
        );

        match serde_json::to_value(&response) {
            Ok(value) => (StatusCode::OK, Json(value)),
            Err(e) => {
                tracing::error!("Failed to serialize search response: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": "Failed to serialize search response",
                        "query": query_str,
                        "total_matches": 0,
                        "results": [],
                        "duration_ms": 0
                    })),
                )
            }
        }
    }

    /// Task query endpoint for the task browser.
    ///
    /// `POST /.mbr/tasks`
    ///
    /// Every request field is optional, so `{}` means "all incomplete tasks in
    /// the repository, grouped by file":
    ///
    /// ```json
    /// {
    ///   "q": "report #work",
    ///   "folder": "/docs/",
    ///   "statuses": ["open"],
    ///   "priorities": [],
    ///   "due": "any",
    ///   "mode": "category",
    ///   "limit": 500
    /// }
    /// ```
    ///
    /// Returns `404` when `tasks_enabled` is off. Server/GUI only — static
    /// builds have no task endpoint at all.
    ///
    /// The first request builds the index (one sequential read pass over the
    /// repository's markdown); later ones reuse it, and the watcher keeps it
    /// fresh. Like search, this does not wait for the repository scan: partial
    /// results with `scan_in_progress: true` beat a hung panel.
    pub async fn tasks_handler(
        State(config): State<ServerState>,
        Json(query): Json<crate::task_query::TaskQuery>,
    ) -> Response<Body> {
        if !config.tasks_enabled {
            return Self::tasks_error(StatusCode::NOT_FOUND, "Task browsing is disabled");
        }

        let scan_in_progress = !config.repo.is_scan_complete();

        if let Err(e) = config
            .task_index
            .ensure_built(&config.repo, &config.base_dir)
            .await
        {
            tracing::error!("Task index build failed: {e}");
            return Self::tasks_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to build the task index",
            );
        }

        // Grouping is pure CPU work over an in-memory snapshot, but a
        // repository with a hundred thousand tasks makes it long enough to be
        // worth keeping off the async runtime.
        let files = config.task_index.snapshot();
        let today = chrono::Local::now().date_naive();
        let result = tokio::task::spawn_blocking(move || {
            crate::task_query::run_query(&files, &query, today)
        })
        .await;

        let mut response = match result {
            Ok(response) => response,
            Err(e) => {
                tracing::error!("Task query task panicked: {e}");
                return Self::tasks_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Task query task failed",
                );
            }
        };
        response.scan_in_progress = scan_in_progress;

        tracing::debug!(
            "Task query completed: {} matches in {} group(s), {}ms",
            response.total_matches,
            response.groups.len(),
            response.duration_ms
        );

        match serde_json::to_vec(&response) {
            Ok(body) => build_response_or_500(
                Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "application/json")
                    .body(Body::from(body)),
            ),
            Err(e) => {
                tracing::error!("Failed to serialize task response: {e}");
                Self::tasks_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to serialize task response",
                )
            }
        }
    }

    /// A JSON error body shaped like an empty task response, so the frontend
    /// can render a failure without a second parse path.
    fn tasks_error(status: StatusCode, message: &str) -> Response<Body> {
        let body = serde_json::json!({
            "error": message,
            "groups": [],
            "folders": [],
            "total_matches": 0,
            "duration_ms": 0,
            "scan_in_progress": false,
        });
        build_response_or_500(
            Response::builder()
                .status(status)
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string())),
        )
    }

    /// Enforces the access policy shared by the raw-fetch and save editing
    /// endpoints. Returns `Ok(())` when the request may proceed, or `Err(resp)`
    /// with the appropriate error response.
    ///
    /// Policy:
    /// 1. Editing must be enabled (`403` otherwise).
    /// 2. CSRF: the request must carry `X-MBR-Edit: 1` and be same-origin
    ///    (`403` otherwise).
    /// 3. Host: when **no** token is configured, the `Host` header must name a
    ///    loopback address or the bound address (`403` otherwise). Same-origin
    ///    alone does not stop DNS rebinding — a rebound attacker page *is*
    ///    genuinely same-origin — so the `Host` name is the only thing that
    ///    distinguishes it. This check is skipped once a token is configured,
    ///    because then the token is the authority and the server may
    ///    legitimately sit behind a reverse proxy presenting any `Host`.
    /// 4. Token: required whenever a token is configured, for any non-loopback
    ///    caller, or when `edit_require_token_on_loopback` is set. Verified
    ///    against `edit_token_hash` as a bearer token (`401` otherwise).
    ///
    /// Deriving (4) from the peer IP alone was a bypass: behind the
    /// TLS-terminating reverse proxy that `docs/modes/editing.md` recommends,
    /// every request arrives from 127.0.0.1, so a configured `edit_token_hash`
    /// was never checked. A configured token is now *always* enforced.
    fn check_edit_access(
        config: &ServerState,
        headers: &HeaderMap,
        peer_ip: std::net::IpAddr,
    ) -> Result<(), (StatusCode, &'static str)> {
        if !config.edit_enabled {
            return Err((StatusCode::FORBIDDEN, "Editing is not enabled"));
        }

        // CSRF: custom header that browsers won't send cross-origin without a
        // CORS preflight (which we never grant).
        let csrf_ok = headers
            .get("x-mbr-edit")
            .and_then(|v| v.to_str().ok())
            .map(|v| v == "1")
            .unwrap_or(false);
        if !csrf_ok {
            return Err((
                StatusCode::FORBIDDEN,
                "Missing or invalid X-MBR-Edit header",
            ));
        }

        if !Self::is_same_origin(headers) {
            return Err((StatusCode::FORBIDDEN, "Cross-origin edit request blocked"));
        }

        // Without a token, the only thing separating a DNS-rebound attacker
        // page from the real local UI is the name it used to reach us.
        let token_configured = config.edit_token_hash.is_some();
        if !token_configured && !host_header_is_allowed(headers, config.bind_ip) {
            return Err((
                StatusCode::FORBIDDEN,
                "Host header does not name this server",
            ));
        }

        let caller_is_loopback = peer_ip.is_loopback();
        let require_token =
            token_configured || !caller_is_loopback || config.edit_require_token_on_loopback;
        if require_token {
            let provided = headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .map(str::trim);
            let ok = match (&config.edit_token_hash, provided) {
                (Some(hash), Some(token)) => crate::edit_auth::verify_token(hash, token),
                _ => false,
            };
            if !ok {
                return Err((StatusCode::UNAUTHORIZED, "A valid edit token is required"));
            }
        }

        Ok(())
    }

    /// Best-effort same-origin check for CSRF protection.
    ///
    /// Prefers the `Sec-Fetch-Site` metadata header (sent by all modern
    /// browsers); falls back to comparing the `Origin` host to the `Host`
    /// header. A request with no `Origin` at all (e.g. a non-browser client) is
    /// allowed — CSRF requires an attacker-controlled document, which always
    /// yields an `Origin`/`Sec-Fetch-Site`.
    fn is_same_origin(headers: &HeaderMap) -> bool {
        if let Some(sfs) = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
            return sfs == "same-origin" || sfs == "none";
        }
        match (
            headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()),
            headers.get(header::HOST).and_then(|v| v.to_str().ok()),
        ) {
            (Some(origin), Some(host)) => origin
                .split_once("://")
                .map(|(_, origin_host)| origin_host == host)
                .unwrap_or(false),
            // No Origin header: not a cross-origin browser request.
            (None, _) => true,
            _ => false,
        }
    }

    /// Resolves a URL path to an existing, editable markdown file on disk.
    ///
    /// Only `ResolvedPath::MarkdownFile` targets are accepted (no file
    /// creation), and the canonical path must stay within the repository root
    /// (defense-in-depth against traversal/symlink escape).
    fn resolve_editable_markdown(
        config: &ServerState,
        path: &str,
    ) -> Result<PathBuf, (StatusCode, &'static str)> {
        let tag_url_sources = crate::config::tag_sources_to_url_sources(&config.tag_sources);
        let resolver_config = PathResolverConfig {
            base_dir: config.base_dir.as_path(),
            canonical_base_dir: config.canonical_base_dir.as_deref(),
            static_folder: &config.static_folder,
            markdown_extensions: &config.markdown_extensions,
            index_file: &config.index_file,
            tag_sources: &tag_url_sources,
        };

        match resolve_request_path(&resolver_config, path) {
            ResolvedPath::MarkdownFile(md_path) => {
                let canonical = md_path.canonicalize().ok();
                let base = config
                    .canonical_base_dir
                    .clone()
                    .or_else(|| config.base_dir.canonicalize().ok());
                match (canonical, base) {
                    (Some(c), Some(b)) if c.starts_with(&b) && c.is_file() => Ok(c),
                    _ => Err((StatusCode::BAD_REQUEST, "Invalid path")),
                }
            }
            _ => Err((StatusCode::NOT_FOUND, "Not an editable markdown file")),
        }
    }

    /// GET /.mbr/raw/{*path} — returns the raw markdown source of an existing
    /// file plus an `X-MBR-Content-Hash` header for optimistic concurrency.
    pub async fn raw_markdown_handler(
        extract::Path(path): extract::Path<String>,
        State(config): State<ServerState>,
        ConnectInfo(peer): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
    ) -> Response {
        if let Err(err) = Self::check_edit_access(&config, &headers, peer.ip()) {
            return err.into_response();
        }
        let md_path = match Self::resolve_editable_markdown(&config, &path) {
            Ok(p) => p,
            Err(err) => return err.into_response(),
        };

        match tokio::fs::read(&md_path).await {
            Ok(bytes) => {
                let hash = crate::edit_auth::content_hash(&bytes);
                let mut resp = (StatusCode::OK, bytes).into_response();
                resp.headers_mut().insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/markdown; charset=utf-8"),
                );
                if let Ok(hv) = HeaderValue::from_str(&hash) {
                    resp.headers_mut().insert("x-mbr-content-hash", hv);
                }
                resp
            }
            Err(e) => {
                tracing::error!("Failed to read markdown for editing: {e}");
                (StatusCode::NOT_FOUND, "File not found").into_response()
            }
        }
    }

    /// POST /.mbr/edit/{*path} — overwrites an existing markdown file with the
    /// provided content, guarded by an optimistic-concurrency hash check and an
    /// atomic temp-file-then-rename write.
    pub async fn save_markdown_handler(
        extract::Path(path): extract::Path<String>,
        State(config): State<ServerState>,
        ConnectInfo(peer): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
        Json(req): Json<EditRequest>,
    ) -> Response {
        if let Err(err) = Self::check_edit_access(&config, &headers, peer.ip()) {
            return err.into_response();
        }
        let md_path = match Self::resolve_editable_markdown(&config, &path) {
            Ok(p) => p,
            Err(err) => return err.into_response(),
        };

        // Optimistic concurrency: reject if the file changed since it was loaded.
        let current = match tokio::fs::read(&md_path).await {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::error!("Failed to read markdown before save: {e}");
                return (StatusCode::NOT_FOUND, "File not found").into_response();
            }
        };
        if crate::edit_auth::content_hash(&current) != req.base_hash {
            return (
                StatusCode::CONFLICT,
                "File changed on disk since it was loaded",
            )
                .into_response();
        }

        let new_bytes = req.content.into_bytes();
        let new_hash = crate::edit_auth::content_hash(&new_bytes);

        // Atomic write: write a temp file in the same directory, then rename.
        let parent = md_path.parent().unwrap_or_else(|| Path::new("."));
        let file_name = md_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file.md");
        let tmp_path = parent.join(format!(".{file_name}.mbr-tmp"));
        if let Err(e) = tokio::fs::write(&tmp_path, &new_bytes).await {
            tracing::error!("Failed to write temp file: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Write failed").into_response();
        }
        if let Err(e) = tokio::fs::rename(&tmp_path, &md_path).await {
            tracing::error!("Failed to rename temp file into place: {e}");
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return (StatusCode::INTERNAL_SERVER_ERROR, "Write failed").into_response();
        }

        // Trigger live-reload for connected clients.
        if let Some(tx) = &config.file_change_tx {
            let relative =
                pathdiff::diff_paths(&md_path, &config.base_dir).unwrap_or_else(|| md_path.clone());
            let _ = tx.send(crate::watcher::FileChangeEvent {
                path: md_path.to_string_lossy().to_string(),
                relative_path: relative.to_string_lossy().to_string(),
                event: crate::watcher::ChangeEventType::Modified,
            });
        }

        let mut resp = (StatusCode::OK, "Saved").into_response();
        if let Ok(hv) = HeaderValue::from_str(&new_hash) {
            resp.headers_mut().insert("x-mbr-content-hash", hv);
        }
        resp
    }

    /// POST /.mbr/task — flips the status of a single task line.
    ///
    /// ```json
    /// { "path": "docs/guide.md", "line": 42,
    ///   "expected": "- [ ] write the report !!", "to": "done" }
    /// ```
    ///
    /// ```json
    /// { "line": 42, "text": "- [x] write the report !! @done(2026-08-04 14:32)" }
    /// ```
    ///
    /// Gated on `edit_enabled` and the same [`Self::check_edit_access`] policy
    /// as every other write endpoint — this writes to the user's files, so it
    /// answers to the editing switch, not to `tasks_enabled` (in-document
    /// checkboxes exist whether or not the task browser does).
    ///
    /// # Why `expected` rather than a whole-file hash
    ///
    /// The editor holds a whole file open and can afford
    /// [`EditRequest::base_hash`]. A checkbox cannot: the page has been open for
    /// an hour and the user has no idea what else changed in the file. Matching
    /// just the one line means an unrelated edit elsewhere in the file does not
    /// spuriously fail the toggle, while an edit *to that line* — the only case
    /// where flipping its marker could corrupt something — still does, with a
    /// `409`.
    ///
    /// | Status | Cause |
    /// |--------|-------|
    /// | `403` / `401` | [`Self::check_edit_access`] (disabled, CSRF, cross-origin, `Host`, token) |
    /// | `404` | The path is not an editable markdown file |
    /// | `400` | Path outside the root, unreadable/not-UTF-8 file, no such line, or the line is not a task |
    /// | `409` | The line no longer matches `expected` |
    /// | `422` | The body is not a well-formed request (axum's `Json` rejection, before this handler runs) |
    /// | `500` | The write failed |
    pub async fn task_toggle_handler(
        State(config): State<ServerState>,
        ConnectInfo(peer): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
        Json(req): Json<TaskToggleRequest>,
    ) -> Response {
        if let Err(err) = Self::check_edit_access(&config, &headers, peer.ip()) {
            return err.into_response();
        }
        let md_path = match Self::resolve_editable_markdown(&config, &req.path) {
            Ok(p) => p,
            Err(err) => return err.into_response(),
        };

        let source = match tokio::fs::read(&md_path).await {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(_) => {
                    return (StatusCode::BAD_REQUEST, "File is not valid UTF-8").into_response();
                }
            },
            Err(e) => {
                tracing::error!("Failed to read markdown before task toggle: {e}");
                return (StatusCode::NOT_FOUND, "File not found").into_response();
            }
        };

        // Local wall clock, matching the naive/local dates `tasks.rs` parses.
        let stamp = config
            .tasks_stamp_done
            .then(|| chrono::Local::now().naive_local());
        let patched =
            match crate::tasks::patch_task_line(&source, req.line, &req.expected, req.to, stamp) {
                Ok(patched) => patched,
                Err(e) => {
                    let status = match e {
                        TaskPatchError::Mismatch { .. } => StatusCode::CONFLICT,
                        TaskPatchError::LineOutOfRange { .. } | TaskPatchError::NotATask { .. } => {
                            StatusCode::BAD_REQUEST
                        }
                    };
                    return (status, e.to_string()).into_response();
                }
            };

        if let Err(e) = Self::atomic_write_file(&md_path, patched.source.as_bytes()) {
            tracing::error!("Failed to write task toggle: {e:?}");
            return e.into_response();
        }

        // Live-reload for connected clients, then the task index, which the
        // watcher would also refresh — but only after its debounce, and the
        // panel that sent this expects its own next query to see the change.
        Self::broadcast_change(&config, &md_path, crate::watcher::ChangeEventType::Modified);
        config.task_index.invalidate_file(
            &md_path,
            &crate::watcher::ChangeEventType::Modified,
            &config.repo,
            &config.base_dir,
        );

        Json(TaskToggleResponse {
            line: req.line,
            text: patched.text,
        })
        .into_response()
    }

    /// Resolves a repo-relative path that may not yet exist to an absolute
    /// path, guarding against traversal and symlink escape.
    ///
    /// Rejects any `..` (or absolute) component, joins onto the canonical repo
    /// root, then canonicalizes the deepest **existing** ancestor and asserts it
    /// stays within the root. Does not require the target to exist or to be
    /// markdown — callers enforce extensions where relevant (create/move do;
    /// mkdir does not).
    fn resolve_new_target(config: &ServerState, rel: &str) -> Result<PathBuf, FileOpError> {
        let canonical_base = config
            .canonical_base_dir
            .clone()
            .or_else(|| config.base_dir.canonicalize().ok())
            .ok_or_else(|| {
                FileOpError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "repository root not found",
                ))
            })?;
        resolve_new_target_path(&canonical_base, rel)
    }

    /// Whether `path` ends in a configured markdown extension (case-insensitive).
    fn path_has_markdown_extension(path: &Path, exts: &[String]) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| crate::repo::is_markdown_extension(&e.to_lowercase(), exts))
            .unwrap_or(false)
    }

    /// Whether `path`'s file name is the configured index file.
    fn path_is_index(path: &Path, index_file: &str) -> bool {
        path.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n == index_file)
    }

    /// The repo-relative path of `path`, always `/`-separated.
    ///
    /// This feeds the `path` field of the file-operation responses, which is
    /// documented as e.g. `notes/image.png`, so it must not become
    /// `notes\image.png` on Windows.
    fn rel_path_string(path: &Path, base_dir: &Path) -> String {
        let relative = pathdiff::diff_paths(path, base_dir).unwrap_or_else(|| path.to_path_buf());
        crate::url_path::path_to_url(&relative)
    }

    /// Atomically writes `bytes` to `path` (temp file in the same dir + rename).
    fn atomic_write_file(path: &Path, bytes: &[u8]) -> Result<(), FileOpError> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file.md");
        let tmp = parent.join(format!(".{file_name}.mbr-tmp"));
        std::fs::write(&tmp, bytes).map_err(FileOpError::Io)?;
        if let Err(e) = std::fs::rename(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(FileOpError::Io(e));
        }
        Ok(())
    }

    /// Broadcasts a `FileChangeEvent` for live-reload + watcher reconciliation.
    fn broadcast_change(
        config: &ServerState,
        abs_path: &Path,
        event: crate::watcher::ChangeEventType,
    ) {
        if let Some(tx) = &config.file_change_tx {
            let relative = pathdiff::diff_paths(abs_path, &config.base_dir)
                .unwrap_or_else(|| abs_path.to_path_buf());
            let _ = tx.send(crate::watcher::FileChangeEvent {
                path: abs_path.to_string_lossy().to_string(),
                relative_path: relative.to_string_lossy().to_string(),
                event,
            });
        }
    }

    /// POST /.mbr/create/{*path} — creates a new markdown file with the given
    /// contents. `{*path}` is a repo-relative filesystem path with extension.
    pub async fn create_markdown_handler(
        extract::Path(path): extract::Path<String>,
        State(config): State<ServerState>,
        ConnectInfo(peer): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
        Json(req): Json<CreateRequest>,
    ) -> Response {
        if let Err(err) = Self::check_edit_access(&config, &headers, peer.ip()) {
            return err.into_response();
        }
        let result =
            tokio::task::spawn_blocking(move || Self::do_create(&config, &path, req)).await;
        match result {
            Ok(Ok(resp)) => (StatusCode::OK, Json(resp)).into_response(),
            Ok(Err(e)) => e.into_response(),
            Err(e) => {
                tracing::error!("create task panicked: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Create failed").into_response()
            }
        }
    }

    /// Blocking body of [`Self::create_markdown_handler`].
    fn do_create(
        config: &ServerState,
        rel: &str,
        req: CreateRequest,
    ) -> Result<CreateResponse, FileOpError> {
        let dst = Self::resolve_new_target(config, rel)?;
        if !Self::path_has_markdown_extension(&dst, &config.markdown_extensions) {
            return Err(FileOpError::NotMarkdown);
        }
        if dst.exists() {
            return Err(FileOpError::AlreadyExists);
        }
        let parent = dst.parent().unwrap_or_else(|| Path::new("."));
        if !parent.exists() {
            if req.create_dirs {
                std::fs::create_dir_all(parent).map_err(FileOpError::Io)?;
            } else {
                return Err(FileOpError::ParentMissing);
            }
        }
        Self::atomic_write_file(&dst, req.content.as_bytes())?;

        let url_path =
            crate::repo::build_markdown_url_path(&dst, &config.base_dir, &config.index_file);
        let rel_path = Self::rel_path_string(&dst, &config.base_dir);

        // Surgical state update (Created): the new file adds its own tags inline
        // in invalidate_file, so no tag-index rebuild is needed.
        config
            .repo
            .invalidate_file(&dst, &crate::watcher::ChangeEventType::Created);
        config.repo.build_relationship_index();
        config.repo.build_wikilink_index();
        ListingCaches::from(config).invalidate();
        config.inbound_link_cache.invalidate_all();

        Self::broadcast_change(config, &dst, crate::watcher::ChangeEventType::Created);

        Ok(CreateResponse {
            url_path,
            path: rel_path,
        })
    }

    /// POST /.mbr/mkdir/{*path} — creates a directory (idempotent). `{*path}` is
    /// a repo-relative filesystem path. No request body.
    pub async fn mkdir_handler(
        extract::Path(path): extract::Path<String>,
        State(config): State<ServerState>,
        ConnectInfo(peer): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
    ) -> Response {
        if let Err(err) = Self::check_edit_access(&config, &headers, peer.ip()) {
            return err.into_response();
        }
        let result = tokio::task::spawn_blocking(move || Self::do_mkdir(&config, &path)).await;
        match result {
            Ok(Ok(resp)) => (StatusCode::OK, Json(resp)).into_response(),
            Ok(Err(e)) => e.into_response(),
            Err(e) => {
                tracing::error!("mkdir task panicked: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Mkdir failed").into_response()
            }
        }
    }

    /// Blocking body of [`Self::mkdir_handler`].
    fn do_mkdir(config: &ServerState, rel: &str) -> Result<MkdirResponse, FileOpError> {
        let target = Self::resolve_new_target(config, rel)?;
        let rel_path = Self::rel_path_string(&target, &config.base_dir);
        if target.is_dir() {
            // Idempotent: pre-creating an existing folder is retry-safe.
            return Ok(MkdirResponse { path: rel_path });
        }
        if target.exists() {
            // A file occupies the path.
            return Err(FileOpError::AlreadyExists);
        }
        std::fs::create_dir_all(&target).map_err(FileOpError::Io)?;
        Self::broadcast_change(config, &target, crate::watcher::ChangeEventType::Created);
        Ok(MkdirResponse { path: rel_path })
    }

    /// POST /.mbr/upload?dir=<>&name=<> — writes a raw binary asset (the
    /// editor's image uploader) into the repo next to the note being edited and
    /// returns a URL the editor and rendered site both resolve. The body is the
    /// raw file bytes; the body-size cap comes from `upload_max_bytes`
    /// (oversized bodies are rejected with `413` by `DefaultBodyLimit`).
    pub async fn upload_handler(
        State(config): State<ServerState>,
        ConnectInfo(peer): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
        extract::Query(params): extract::Query<UploadParams>,
        body: axum::body::Bytes,
    ) -> Response {
        if let Err(err) = Self::check_edit_access(&config, &headers, peer.ip()) {
            return err.into_response();
        }
        let result = tokio::task::spawn_blocking(move || {
            Self::do_upload(&config, &params.dir, &params.name, &body)
        })
        .await;
        match result {
            Ok(Ok(resp)) => (StatusCode::OK, Json(resp)).into_response(),
            Ok(Err(e)) => e.into_response(),
            Err(e) => {
                tracing::error!("upload task panicked: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Upload failed").into_response()
            }
        }
    }

    /// Blocking body of [`Self::upload_handler`].
    fn do_upload(
        config: &ServerState,
        dir: &str,
        name: &str,
        bytes: &[u8],
    ) -> Result<UploadResponse, FileOpError> {
        let safe_name = sanitize_upload_name(name, &config.markdown_extensions)
            .ok_or(FileOpError::InvalidUploadName)?;

        // Compose `<dir>/<name>` and resolve it traversal-safely. `dir` may be
        // empty (root-level note); leading/trailing slashes are tolerated.
        let dir_clean = dir.trim_matches('/');
        let rel = if dir_clean.is_empty() {
            safe_name.clone()
        } else {
            format!("{dir_clean}/{safe_name}")
        };
        let target = Self::resolve_new_target(config, &rel)?;

        // `.mbr` is an ordinary path component to the resolver, so without this
        // the uploader could drop a file into the template folder — where the
        // watcher hot-reloads `*.html` as Tera templates and `components/*.js`
        // shadows the compiled-in bundle in every page.
        if is_template_folder_path(
            &target,
            config.base_dir.as_path(),
            config.canonical_base_dir.as_deref(),
            config.template_folder.as_deref(),
        ) {
            return Err(FileOpError::ForbiddenUploadDir);
        }

        // The destination is normally the note's own (existing) folder; create
        // it defensively if missing. `safe_name` is a pure basename, so the
        // parent is the resolved, containment-checked directory.
        let dest_dir = target
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        if !dest_dir.exists() {
            std::fs::create_dir_all(&dest_dir).map_err(FileOpError::Io)?;
        }

        // Collision policy: keep the name, suffix `-1`, `-2`, … on collision.
        let name_path = Path::new(&safe_name);
        let stem = name_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&safe_name);
        let ext = name_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let final_path = dedupe_name(&dest_dir, stem, ext, |p| p.exists());

        Self::atomic_write_file(&final_path, bytes)?;

        // Root-absolute URL that matches how mbr serves the file. Reuse
        // `build_static_url_path` (same util used for every static asset URL),
        // then percent-encode so special characters load in the browser.
        let root_abs = crate::repo::build_static_url_path(
            &final_path,
            &config.base_dir,
            &config.static_folder,
        );
        let url = utf8_percent_encode(&root_abs, UPLOAD_URL_ENCODE_SET).to_string();
        let rel_path = Self::rel_path_string(&final_path, &config.base_dir);
        let final_name = final_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&safe_name)
            .to_string();

        // A static asset needs no markdown/index rebuild; the watcher will pick
        // it up into media.json. Broadcast so live-reload clients learn of it.
        Self::broadcast_change(
            config,
            &final_path,
            crate::watcher::ChangeEventType::Created,
        );

        Ok(UploadResponse {
            url,
            path: rel_path,
            name: final_name,
        })
    }

    /// POST /.mbr/move/{*path} — moves/renames a markdown file, rewriting
    /// inbound links repo-wide and (on stem change) bare `[[Name]]` links.
    /// `{*path}` is the source repo-relative filesystem path with extension.
    pub async fn move_markdown_handler(
        extract::Path(path): extract::Path<String>,
        State(config): State<ServerState>,
        ConnectInfo(peer): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
        Json(req): Json<MoveRequest>,
    ) -> Response {
        if let Err(err) = Self::check_edit_access(&config, &headers, peer.ip()) {
            return err.into_response();
        }

        // One inbound-grep permit guards the whole repo-wide rewrite pass, held
        // across the blocking work (mirrors links.json grep throttling).
        let _permit = match config.inbound_grep_semaphore.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, "Server shutting down").into_response();
            }
        };

        let result = tokio::task::spawn_blocking(move || Self::do_move(&config, &path, req)).await;
        match result {
            Ok(Ok(resp)) => (StatusCode::OK, Json(resp)).into_response(),
            Ok(Err(e)) => e.into_response(),
            Err(e) => {
                tracing::error!("move task panicked: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Move failed").into_response()
            }
        }
    }

    /// Blocking body of [`Self::move_markdown_handler`].
    fn do_move(
        config: &ServerState,
        from: &str,
        req: MoveRequest,
    ) -> Result<MoveResponse, FileOpError> {
        // Resolve source (existing markdown) and destination (new path).
        let src = Self::resolve_editable_markdown(config, from).map_err(|(status, _)| {
            if status == StatusCode::NOT_FOUND {
                FileOpError::SourceNotFound
            } else {
                FileOpError::Traversal
            }
        })?;
        let dst = Self::resolve_new_target(config, &req.to)?;
        if !Self::path_has_markdown_extension(&dst, &config.markdown_extensions) {
            return Err(FileOpError::NotMarkdown);
        }

        // Collision: destination exists and is not the same file. A case-only
        // rename on a case-insensitive filesystem canonicalizes back to source.
        let dst_canon = dst.canonicalize().ok();
        let case_only = dst.exists() && dst_canon.as_deref() == Some(src.as_path());
        if dst.exists() && !case_only {
            return Err(FileOpError::AlreadyExists);
        }

        // Create the destination parent if requested.
        let parent = dst.parent().unwrap_or_else(|| Path::new("."));
        let mut created_dirs = false;
        if !parent.exists() {
            if req.create_dirs {
                std::fs::create_dir_all(parent).map_err(FileOpError::Io)?;
                created_dirs = true;
            } else {
                return Err(FileOpError::ParentMissing);
            }
        }

        // Compute index-stripped page URLs before touching disk.
        let old_url =
            crate::repo::build_markdown_url_path(&src, &config.base_dir, &config.index_file);
        let new_url =
            crate::repo::build_markdown_url_path(&dst, &config.base_dir, &config.index_file);
        let old_is_index = Self::path_is_index(&src, &config.index_file);
        let new_is_index = Self::path_is_index(&dst, &config.index_file);

        // A4-C delta names (bare `[[Name]]` rewrite on stem change) need the
        // source frontmatter (title/aliases) so still-resolvable names are kept.
        let src_meta = crate::markdown::extract_metadata_from_file(&src).ok();
        let old_stem = src
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let new_stem = dst
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let delta = Self::wikilink_delta_names(
            src_meta.as_ref().map(|m| &m.metadata),
            &old_stem,
            &new_stem,
        );

        // Read source, re-express its own relative links against the new folder
        // (A4-B), then write the rewritten content to the destination.
        let src_content = std::fs::read_to_string(&src).map_err(FileOpError::Io)?;
        let moved_content = crate::link_rewrite::rewrite_moved_file_outbound_links(
            &old_url,
            old_is_index,
            &new_url,
            new_is_index,
            &config.markdown_extensions,
            &src_content,
        );

        // Content changed (A4-B), so this is a temp-write + delete-source, not a
        // plain rename. For a case-only rename the old-cased name must be removed
        // before the rename lands so the new case is preserved.
        let parent_dir = dst.parent().unwrap_or_else(|| Path::new("."));
        let dst_name = dst
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file.md");
        let tmp = parent_dir.join(format!(".{dst_name}.mbr-tmp"));
        std::fs::write(&tmp, moved_content.as_bytes()).map_err(FileOpError::Io)?;
        let rename_result = if case_only {
            std::fs::remove_file(&src).and_then(|()| std::fs::rename(&tmp, &dst))
        } else {
            std::fs::rename(&tmp, &dst).and_then(|()| std::fs::remove_file(&src))
        };
        if let Err(e) = rename_result {
            let _ = std::fs::remove_file(&tmp);
            return Err(FileOpError::Io(e));
        }

        // Skip set for the repo-wide walkers: the destination file (already
        // written); the source no longer exists so it won't be walked.
        let mut skip: HashSet<PathBuf> = HashSet::new();
        skip.insert(dst.clone());
        if let Ok(c) = dst.canonicalize() {
            skip.insert(c);
        }

        // A4-A: rewrite inbound links across the whole repo.
        let rewritten_paths = crate::link_rewrite::rewrite_inbound_links_for_move(
            &old_url,
            &new_url,
            &config.base_dir,
            &config.markdown_extensions,
            &config.ignore_dirs,
            &config.ignore_globs,
            &skip,
        )
        .map_err(FileOpError::Io)?;

        // A4-C: rewrite bare `[[Name]]` links, guarded by the pre-move index.
        let wiki_paths = crate::link_rewrite::rewrite_bare_wikilinks_for_rename(
            &delta,
            &old_url,
            &config.base_dir,
            &config.markdown_extensions,
            &config.ignore_dirs,
            &config.ignore_globs,
            &config.index_file,
            &config.repo.wikilink_index,
            &skip,
        )
        .map_err(FileOpError::Io)?;

        // A5: surgical repo/cache updates + broadcasts.
        config
            .repo
            .invalidate_file(&src, &crate::watcher::ChangeEventType::Deleted);
        config
            .repo
            .invalidate_file(&dst, &crate::watcher::ChangeEventType::Created);
        let mut changed_union: Vec<PathBuf> = Vec::new();
        for p in rewritten_paths.iter().chain(wiki_paths.iter()) {
            if !changed_union.iter().any(|q| q == p) {
                changed_union.push(p.clone());
            }
        }
        for p in &changed_union {
            config
                .repo
                .invalidate_file(p, &crate::watcher::ChangeEventType::Modified);
        }
        config.repo.build_relationship_index();
        config.repo.build_wikilink_index();
        config.repo.rebuild_tag_index();
        ListingCaches::from(config).invalidate();
        config.inbound_link_cache.invalidate_all();
        config.link_cache.invalidate_all();

        Self::broadcast_change(config, &src, crate::watcher::ChangeEventType::Deleted);
        Self::broadcast_change(config, &dst, crate::watcher::ChangeEventType::Created);
        for p in &changed_union {
            Self::broadcast_change(config, p, crate::watcher::ChangeEventType::Modified);
        }

        let to_urls = |paths: &[PathBuf]| -> Vec<String> {
            paths
                .iter()
                .map(|p| {
                    crate::repo::build_markdown_url_path(p, &config.base_dir, &config.index_file)
                })
                .collect()
        };

        Ok(MoveResponse {
            from_url: old_url,
            url_path: new_url,
            path: Self::rel_path_string(&dst, &config.base_dir),
            rewritten: to_urls(&rewritten_paths),
            wikilinks_rewritten: to_urls(&wiki_paths),
            created_dirs,
        })
    }

    /// Computes the old resolvable names that no longer resolve to the file
    /// after a move (for A4-C bare-`[[Name]]` rewriting), each paired with the
    /// new filename stem. Title and aliases are unchanged by a move, so in
    /// practice only a changed filename stem contributes.
    fn wikilink_delta_names(
        frontmatter: Option<&crate::markdown::SimpleMetadata>,
        old_stem: &str,
        new_stem: &str,
    ) -> Vec<(String, String)> {
        let title_for = |stem: &str| -> String {
            frontmatter
                .and_then(|fm| fm.get("title"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| stem.to_string())
        };
        let aliases: Vec<String> = frontmatter
            .and_then(|fm| fm.get("aliases"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let old_title = title_for(old_stem);
        let new_title = title_for(new_stem);

        // Names that still resolve to the file after the move must NOT be
        // rewritten: the new stem, the (unchanged) title, and any aliases.
        let mut new_names: HashSet<String> = HashSet::new();
        new_names.insert(crate::relationships::normalize_name(new_stem));
        new_names.insert(crate::relationships::normalize_name(&new_title));
        for a in &aliases {
            new_names.insert(crate::relationships::normalize_name(a));
        }

        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for cand in std::iter::once(old_stem.to_string())
            .chain(std::iter::once(old_title))
            .chain(aliases.iter().cloned())
        {
            let norm = crate::relationships::normalize_name(&cand);
            if new_names.contains(&norm) || !seen.insert(norm) {
                continue;
            }
            out.push((cand, new_stem.to_string()));
        }
        out
    }

    /// Media viewer endpoint for video, PDF, audio, and image content.
    ///
    /// GET /.mbr/videos/?path=<encoded_path>
    /// GET /.mbr/pdfs/?path=<encoded_path>
    /// GET /.mbr/audio/?path=<encoded_path>
    /// GET /.mbr/images/?path=<encoded_path>
    ///
    /// Renders the media_viewer.html template with the appropriate media type
    /// and validated media path. The path query parameter must be URL-encoded
    /// and point to a valid file within the repository.
    pub async fn serve_media_viewer(
        State(config): State<ServerState>,
        OriginalUri(uri): OriginalUri,
        extract::Query(query): extract::Query<MediaViewerQuery>,
    ) -> impl IntoResponse {
        use serde_json::json;

        // Determine media type from route path (the URI path without query string)
        let route_path = uri.path();
        let media_type = match MediaViewerType::from_route(route_path) {
            Some(mt) => mt,
            None => {
                tracing::error!("Invalid media viewer route: {}", route_path);
                return Self::render_error_page(
                    &config.templates,
                    StatusCode::NOT_FOUND,
                    "Not Found",
                    Some("Invalid media viewer route"),
                    route_path,
                    config.gui_mode,
                    &config.sidebar_style,
                    config.sidebar_max_items,
                    config.graph_depth,
                    config.tasks_enabled,
                );
            }
        };

        // Check for missing path parameter
        let media_path = match &query.path {
            Some(p) if !p.is_empty() => p,
            _ => {
                tracing::warn!("Media viewer called without path parameter");
                return Self::render_error_page(
                    &config.templates,
                    StatusCode::BAD_REQUEST,
                    "Bad Request",
                    Some("Missing required 'path' query parameter"),
                    route_path,
                    config.gui_mode,
                    &config.sidebar_style,
                    config.sidebar_max_items,
                    config.graph_depth,
                    config.tasks_enabled,
                );
            }
        };

        // Validate the media path
        let validated_path =
            match validate_media_path(media_path, &config.base_dir, &config.static_folder) {
                Ok(p) => p,
                Err(MbrError::DirectoryTraversal) => {
                    tracing::warn!("Directory traversal attempt: {}", media_path);
                    return Self::render_error_page(
                        &config.templates,
                        StatusCode::FORBIDDEN,
                        "Forbidden",
                        Some("Access denied: Invalid path"),
                        route_path,
                        config.gui_mode,
                        &config.sidebar_style,
                        config.sidebar_max_items,
                        config.graph_depth,
                        config.tasks_enabled,
                    );
                }
                Err(MbrError::InvalidMediaPath(msg)) => {
                    tracing::warn!("Invalid media path: {} - {}", media_path, msg);
                    return Self::render_error_page(
                        &config.templates,
                        StatusCode::NOT_FOUND,
                        "Not Found",
                        Some(&format!("Media file not found: {}", msg)),
                        route_path,
                        config.gui_mode,
                        &config.sidebar_style,
                        config.sidebar_max_items,
                        config.graph_depth,
                        config.tasks_enabled,
                    );
                }
                Err(e) => {
                    tracing::error!("Unexpected error validating media path: {}", e);
                    return Self::render_error_page(
                        &config.templates,
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal Server Error",
                        Some("Failed to validate media path"),
                        route_path,
                        config.gui_mode,
                        &config.sidebar_style,
                        config.sidebar_max_items,
                        config.graph_depth,
                        config.tasks_enabled,
                    );
                }
            };

        // Extract title from filename
        let title = validated_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Media Viewer")
            .to_string();

        // Generate breadcrumbs from the URL path (media_path), not the filesystem path
        // The media_path is already the URL path (e.g., "/videos/Jay Sankey/video.mp4")
        let url_path = std::path::Path::new(media_path);
        let breadcrumbs =
            generate_breadcrumbs(url_path.parent().unwrap_or(std::path::Path::new("")));
        let breadcrumbs_json = page_context::breadcrumbs_to_json(&breadcrumbs, &UrlMode::Absolute);

        // Get parent path for back navigation (from URL path)
        let parent_path = url_path.parent().and_then(|p| p.to_str()).map(|p| {
            if p.is_empty() || p == "/" {
                "/".to_string()
            } else {
                // Ensure trailing slash and clean up leading slash for format
                let clean = p.trim_start_matches('/');
                format!("/{}/", clean)
            }
        });

        // Build template context
        let mut context = std::collections::HashMap::new();
        context.insert("media_type".to_string(), json!(media_type.as_str()));
        context.insert("title".to_string(), json!(title));
        context.insert("media_path".to_string(), json!(media_path));
        context.insert("breadcrumbs".to_string(), json!(breadcrumbs_json));
        if let Some(parent) = parent_path {
            context.insert("parent_path".to_string(), json!(parent));
        }
        page_context::insert_page_chrome(
            &mut context,
            &PageChrome {
                mode: ModeFlags::Server {
                    gui_mode: Some(config.gui_mode),
                    mbr_base: true,
                },
                sidebar_style: &config.sidebar_style,
                sidebar_max_items: config.sidebar_max_items,
                graph_depth: config.graph_depth,
                tasks_enabled: config.tasks_enabled,
                title_affixes: Some((&config.title_prefix, &config.title_suffix)),
            },
        );

        // Render the media viewer template
        match config.templates.render_media_viewer(context) {
            Ok(html) => {
                let etag = generate_etag(html.as_bytes());
                build_response_or_500(
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                        .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_STORE)
                        .header(header::ETAG, etag)
                        .body(Body::from(html)),
                )
            }
            Err(e) => {
                tracing::error!("Failed to render media viewer template: {}", e);
                Self::render_error_page(
                    &config.templates,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal Server Error",
                    Some("Failed to render media viewer"),
                    route_path,
                    config.gui_mode,
                    &config.sidebar_style,
                    config.sidebar_max_items,
                    config.graph_depth,
                    config.tasks_enabled,
                )
            }
        }
    }

    /// Serves assets from /.mbr/* path.
    ///
    /// Priority:
    /// 1. If template_folder is set, serve from there (js/ for components, rest from root)
    /// 2. Otherwise, check .mbr/ directory in base_dir
    /// 3. Fall back to compiled-in DEFAULT_FILES
    ///
    /// # Security
    ///
    /// Path traversal attacks are blocked by `safe_join_asset` which validates
    /// that resolved paths remain within the intended directory, and
    /// [`is_servable_mbr_asset`] restricts what may be read out of that
    /// directory at all — the template folder also holds `config.toml`, which
    /// carries the Argon2 `edit_token_hash` and must never be served.
    pub async fn serve_mbr_assets(
        extract::Path(path): extract::Path<String>,
        State(config): State<ServerState>,
    ) -> Result<impl IntoResponse, StatusCode> {
        tracing::debug!("serve_mbr_assets: {}", path);

        // Normalize path: add leading slash if missing
        let asset_path = if path.starts_with('/') {
            path.clone()
        } else {
            format!("/{}", path)
        };

        // Asset allowlist: `.mbr/` is a config directory as much as an asset
        // directory, so only recognized asset types are ever served from it.
        if !is_servable_mbr_asset(&asset_path) {
            tracing::debug!("serve_mbr_assets: not a servable asset: {}", asset_path);
            return Err(StatusCode::NOT_FOUND);
        }

        // Try template_folder first if set (with path traversal protection)
        if let Some(ref template_folder) = config.template_folder {
            // Map components/* -> components-js/* in template folder
            let relative_path = if asset_path.starts_with("/components/") {
                let component_name = asset_path
                    .strip_prefix("/components/")
                    .unwrap_or(&asset_path);
                format!("components-js/{}", component_name)
            } else {
                asset_path.trim_start_matches('/').to_string()
            };

            tracing::trace!("Checking template folder for: {}", relative_path);

            if let Some(file_path) = safe_join_asset(template_folder, &relative_path) {
                return Self::serve_file_from_path(&file_path).await;
            }
        }

        // Try .mbr/ directory in base_dir (with path traversal protection)
        let mbr_dir = config.base_dir.join(MBR_TEMPLATE_DIR);
        tracing::trace!("Checking .mbr dir for: {}", asset_path);

        if let Some(file_path) = safe_join_asset(&mbr_dir, &asset_path) {
            return Self::serve_file_from_path(&file_path).await;
        }

        // Handle /pico.min.css dynamically based on theme config
        if asset_path == "/pico.min.css" {
            return Self::serve_themed_pico(&config.theme);
        }

        // Fall back to compiled-in defaults
        Self::serve_default_file(&asset_path)
    }

    /// Serve a file from the filesystem with appropriate MIME type and cache headers.
    async fn serve_file_from_path(path: &std::path::Path) -> Result<Response<Body>, StatusCode> {
        let mime = Self::guess_mime_type(path);
        let bytes = tokio::fs::read(path).await.map_err(|e| {
            tracing::error!("Failed to read file {}: {}", path.display(), e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        // Generate ETag from content
        let etag = generate_etag(&bytes);

        // Get Last-Modified from file metadata
        let last_modified = tokio::fs::metadata(path)
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| generate_last_modified(d.as_secs()));

        let mut builder = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime)
            .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_CACHE)
            .header(header::ETAG, etag);

        if let Some(lm) = last_modified {
            builder = builder.header(header::LAST_MODIFIED, lm);
        }

        builder
            .body(axum::body::Body::from(bytes))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    }

    /// Serve themed Pico CSS based on the configured theme.
    ///
    /// Returns the appropriate Pico CSS variant based on theme config:
    /// - "" or "default" -> pico.min.css
    /// - "{color}" (e.g., "amber") -> pico.{color}.min.css
    /// - "fluid" -> pico.fluid.classless.min.css
    /// - "fluid.{color}" (e.g., "fluid.amber") -> pico.fluid.classless.{color}.min.css
    fn serve_themed_pico(theme: &str) -> Result<Response<Body>, StatusCode> {
        match embedded_pico::get_pico_css(theme) {
            Some(bytes) => {
                let etag = generate_etag(bytes);
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/css")
                    .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_CACHE)
                    .header(header::ETAG, etag)
                    .body(Body::from(bytes.to_vec()))
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
            }
            None => {
                eprintln!(
                    "Warning: Invalid theme '{}'. Valid themes: {}",
                    theme,
                    embedded_pico::valid_themes_display()
                );
                Err(StatusCode::NOT_FOUND)
            }
        }
    }

    /// Guess MIME type from file extension
    fn guess_mime_type(path: &std::path::Path) -> &'static str {
        match path.extension().and_then(|e| e.to_str()) {
            Some("html") => "text/html",
            Some("css") => "text/css",
            Some("js") => "application/javascript",
            Some("json") => "application/json",
            Some("map") => "application/json",
            Some("svg") => "image/svg+xml",
            Some("png") => "image/png",
            Some("jpg") | Some("jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("woff") => "font/woff",
            Some("woff2") => "font/woff2",
            Some("ttf") => "font/ttf",
            Some("eot") => "application/vnd.ms-fontobject",
            _ => "application/octet-stream",
        }
    }

    /// Render an error page using the error.html template.
    /// Falls back to a plain text response if template rendering fails.
    #[allow(clippy::too_many_arguments)]
    fn render_error_page(
        templates: &templates::Templates,
        status_code: StatusCode,
        error_title: &str,
        error_message: Option<&str>,
        requested_url: &str,
        gui_mode: bool,
        sidebar_style: &str,
        sidebar_max_items: usize,
        graph_depth: usize,
        tasks_enabled: bool,
    ) -> Response<Body> {
        use std::collections::HashMap;

        let mut context: HashMap<String, serde_json::Value> = HashMap::new();
        page_context::insert_error_keys(
            &mut context,
            status_code.as_u16(),
            error_title,
            error_message,
        );
        context.insert(
            "requested_url".to_string(),
            serde_json::Value::String(requested_url.to_string()),
        );
        // Server mode uses absolute paths; error pages omit title affixes
        page_context::insert_page_chrome(
            &mut context,
            &PageChrome {
                mode: ModeFlags::Server {
                    gui_mode: Some(gui_mode),
                    mbr_base: false,
                },
                sidebar_style,
                sidebar_max_items,
                graph_depth,
                tasks_enabled,
                title_affixes: None,
            },
        );

        match templates.render_error(context) {
            Ok(html) => build_response_or_500(
                Response::builder()
                    .status(status_code)
                    .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                    .body(Body::from(html)),
            ),
            Err(e) => {
                tracing::error!("Failed to render error page: {}", e);
                build_response_or_500(
                    Response::builder()
                        .status(status_code)
                        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                        .body(Body::from(format!(
                            "{} {}",
                            status_code.as_u16(),
                            error_title
                        ))),
                )
            }
        }
    }

    /// Serve from compiled-in DEFAULT_FILES or KATEX_FILES with cache headers.
    fn serve_default_file(path: &str) -> Result<Response<Body>, StatusCode> {
        // First check DEFAULT_FILES
        let file = DEFAULT_FILES
            .iter()
            .find(|(name, _, _)| path == *name)
            // Then check KATEX_FILES (embedded KaTeX CSS, JS, and fonts)
            .or_else(|| {
                embedded_katex::KATEX_FILES
                    .iter()
                    .find(|(name, _, _)| path == *name)
            });

        if let Some((_name, bytes, mime)) = file {
            tracing::debug!("found default file");

            // Generate ETag from content
            let etag = generate_etag(bytes);

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, *mime)
                .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_CACHE)
                .header(header::ETAG, etag)
                .body(axum::body::Body::from(*bytes))
                .inspect_err(|e| tracing::error!("Error rendering default file: {e}"))
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
        } else {
            tracing::debug!("no default found for: {}", path);
            Err(StatusCode::NOT_FOUND)
        }
    }

    async fn handle(
        extract::Path(path): extract::Path<String>,
        State(config): State<ServerState>,
        req: extract::Request<Body>,
    ) -> Result<impl IntoResponse, StatusCode> {
        tracing::debug!("handle: {}", &path);

        let tag_url_sources = crate::config::tag_sources_to_url_sources(&config.tag_sources);
        let resolver_config = PathResolverConfig {
            base_dir: config.base_dir.as_path(),
            canonical_base_dir: config.canonical_base_dir.as_deref(),
            static_folder: &config.static_folder,
            markdown_extensions: &config.markdown_extensions,
            index_file: &config.index_file,
            tag_sources: &tag_url_sources,
        };

        // Defense in depth: never serve a resolved filesystem path that escapes
        // the repository root (or the static-folder overlay). A symlink inside
        // the repo pointing outside it is lexically contained, so the check has
        // to be made on the canonicalized path, right before serving.
        let resolved = match resolve_request_path(&resolver_config, &path) {
            ResolvedPath::StaticFile(resolved_path)
            | ResolvedPath::MarkdownFile(resolved_path)
            | ResolvedPath::DirectoryListing(resolved_path)
                if !is_within_served_roots(
                    &resolved_path,
                    config.base_dir.as_path(),
                    config.canonical_base_dir.as_deref(),
                    &config.static_folder,
                ) =>
            {
                tracing::warn!(
                    "Blocked request for a path outside the repository root: {}",
                    resolved_path.display()
                );
                ResolvedPath::NotFound
            }
            other => other,
        };

        match resolved {
            ResolvedPath::StaticFile(file_path) => {
                // Check if this is a PDF cover sidecar file that might be stale
                #[cfg(feature = "media-metadata")]
                if let Some(response) =
                    Self::try_serve_pdf_cover_sidecar(&path, &file_path, &config).await
                {
                    return Ok(response);
                }
                tracing::debug!("serving static file: {:?}", &file_path);
                Self::serve_static_file(file_path, req).await
            }
            ResolvedPath::MarkdownFile(md_path) => {
                // A markdown page has exactly one URL — the directory-style one
                // (`docs/guide.md` -> `/docs/guide/`). Serving it at any other
                // spelling with a 200 is not harmless: the browser's base for
                // that page's own relative links becomes `/docs/` instead of
                // `/docs/guide/`, so every `../` href the renderer emitted for
                // the trailing-slash convention lands one directory too high
                // and 404s. The damage therefore shows up one click *after* the
                // wrong URL, which is what makes it so hard to trace back.
                //
                // Redirecting here is the general fix: it repairs hand-typed
                // URLs, inbound external links and stale bookmarks, not just
                // hrefs mbr generated itself.
                if let Some(canonical) = crate::path_resolver::canonical_page_redirect(
                    &path,
                    &crate::repo::build_markdown_url_path(
                        &md_path,
                        config
                            .canonical_base_dir
                            .as_deref()
                            .unwrap_or(config.base_dir.as_path()),
                        &config.index_file,
                    ),
                ) {
                    return Ok(canonical_redirect_response(&canonical, req.uri().query()));
                }
                tracing::debug!("rendering markdown: {:?}", &md_path);
                Self::markdown_to_html(&md_path, &config)
                    .await
                    .map_err(|e| {
                        tracing::error!("Error rendering markdown: {e}");
                        StatusCode::INTERNAL_SERVER_ERROR
                    })
            }
            ResolvedPath::DirectoryListing(dir_path) => {
                tracing::debug!("generating directory listing: {:?}", &dir_path);
                Self::directory_to_html(
                    &dir_path,
                    &config.templates,
                    config.base_dir.as_path(),
                    &config,
                )
                .await
                .map_err(|e| {
                    tracing::error!("Error generating directory listing: {e}");
                    StatusCode::INTERNAL_SERVER_ERROR
                })
            }
            ResolvedPath::TagPage { source, value } => {
                tracing::debug!("generating tag page: source={}, value={}", source, value);
                Self::tag_page_to_html(&source, &value, &config)
                    .await
                    .map_err(|e| {
                        tracing::error!("Error generating tag page: {e}");
                        StatusCode::INTERNAL_SERVER_ERROR
                    })
            }
            ResolvedPath::TagSourceIndex { source } => {
                tracing::debug!("generating tag source index: source={}", source);
                Self::tag_source_index_to_html(&source, &config)
                    .await
                    .map_err(|e| {
                        tracing::error!("Error generating tag source index: {e}");
                        StatusCode::INTERNAL_SERVER_ERROR
                    })
            }
            ResolvedPath::Redirect(canonical_url) => Ok(canonical_redirect_response(
                &canonical_url,
                req.uri().query(),
            )),
            ResolvedPath::NotFound => {
                // Try to serve HLS content (playlist or segment) for transcoded variants
                #[cfg(feature = "media-metadata")]
                if config.transcode_enabled
                    && let Some(response) = Self::try_serve_hls_content(&path, &config).await
                {
                    return Ok(response);
                }

                // Try to serve the stream-copy (remux) HLS variant. Deliberately
                // NOT gated on `transcode_enabled`: a remux costs a fraction of
                // a re-encode and is how a video the browser refused to play
                // gets recovered.
                #[cfg(feature = "media-metadata")]
                if let Some(response) = Self::try_serve_remux_content(&path, &config).await {
                    return Ok(response);
                }

                // Try to serve dynamically generated video metadata (server mode only)
                #[cfg(feature = "media-metadata")]
                if let Some(response) = Self::try_serve_video_metadata(&path, &config).await {
                    return Ok(response);
                }

                // Try to serve dynamically generated PDF cover image (server mode only)
                #[cfg(feature = "media-metadata")]
                if let Some(response) = Self::try_serve_pdf_cover(&path, &config).await {
                    return Ok(response);
                }

                // Try to serve errors.json for per-page problem reporting
                // (server/GUI only — static builds never register this)
                if let Some(response) = Self::try_serve_errors_json(&path, &config).await {
                    return Ok(response);
                }

                // Try to serve links.json for bidirectional link tracking
                if let Some(response) = Self::try_serve_links_json(&path, &config).await {
                    return Ok(response);
                }

                tracing::debug!("resource not found: {}", &path);
                let requested_url = format!("/{}", path);
                Ok(Self::render_error_page(
                    &config.templates,
                    StatusCode::NOT_FOUND,
                    "Not Found",
                    Some("The requested page could not be found."),
                    &requested_url,
                    config.gui_mode,
                    &config.sidebar_style,
                    config.sidebar_max_items,
                    config.graph_depth,
                    config.tasks_enabled,
                ))
            }
        }
    }

    /// Serves a static file using tower's ServeFile service with cache headers.
    /// ServeFile already provides Last-Modified and ETag headers.
    async fn serve_static_file(
        file_path: std::path::PathBuf,
        req: extract::Request<Body>,
    ) -> Result<Response, StatusCode> {
        let static_service = ServeFile::new(file_path);
        let mut response = static_service
            .oneshot(req)
            .await
            .map(|r| r.into_response())
            .map_err(|e| {
                tracing::error!("Error serving static file: {e}");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        // Add Cache-Control header for browser revalidation
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static(CACHE_CONTROL_NO_CACHE),
        );

        Ok(response)
    }

    /// Builds a response from a cached video-metadata entry, or `None` for a
    /// negative (`NotAvailable`) marker so the caller falls through to a 404.
    #[cfg(feature = "media-metadata")]
    fn metadata_response_from_cache(
        cached: crate::video_metadata_cache::CachedMetadata,
    ) -> Option<Response<Body>> {
        use crate::video_metadata_cache::CachedMetadata;
        match cached {
            CachedMetadata::Cover(bytes) => Some(Self::build_jpg_response(bytes)),
            CachedMetadata::Chapters(vtt) | CachedMetadata::Captions(vtt) => {
                Some(Self::build_vtt_response(vtt))
            }
            CachedMetadata::NotAvailable => None,
        }
    }

    /// Runs the (blocking) ffmpeg extraction for a single metadata type off the
    /// async worker, stores the result (positive or negative) in the cache, and
    /// returns the response. This is only ever called by the single producer for
    /// a given cache key (see `try_serve_video_metadata`).
    #[cfg(feature = "media-metadata")]
    async fn extract_video_metadata_and_cache(
        video_file: std::path::PathBuf,
        metadata_type: crate::video_metadata::MetadataType,
        key: String,
        config: &ServerState,
    ) -> Option<Response<Body>> {
        use crate::video_metadata::{
            MetadataType, extract_captions, extract_chapters, extract_cover,
        };
        use crate::video_metadata_cache::CachedMetadata;

        // ffmpeg decoding is blocking CPU/IO work; keep it off the tokio worker
        // threads (finding #16). Owned `video_file`/`metadata_type` are `Send`.
        let extracted = tokio::task::spawn_blocking(move || match metadata_type {
            MetadataType::Cover => extract_cover(&video_file).map(CachedMetadata::Cover),
            MetadataType::Chapters => extract_chapters(&video_file).map(CachedMetadata::Chapters),
            MetadataType::Captions => extract_captions(&video_file).map(CachedMetadata::Captions),
        })
        .await;

        match extracted {
            Ok(Ok(cached)) => {
                config.video_metadata_cache.insert(key, cached.clone());
                Self::metadata_response_from_cache(cached)
            }
            Ok(Err(e)) => {
                tracing::debug!("Failed to extract video metadata: {}", e);
                config
                    .video_metadata_cache
                    .insert(key, CachedMetadata::NotAvailable);
                None
            }
            Err(e) => {
                tracing::warn!("Video metadata extraction task panicked: {}", e);
                None
            }
        }
    }

    /// Try to serve dynamically generated video metadata (cover, chapters, captions).
    ///
    /// Returns Some(Response) if the request was for video metadata and we successfully
    /// generated it, None otherwise (fall through to 404).
    #[cfg(feature = "media-metadata")]
    async fn try_serve_video_metadata(path: &str, config: &ServerState) -> Option<Response<Body>> {
        use crate::video_metadata::{MetadataType, parse_metadata_request};
        use crate::video_metadata_cache::cache_key_with_mtime;

        // Check if this is a video metadata request
        let (video_url_path, metadata_type) = parse_metadata_request(path)?;

        let cache_type_str = match metadata_type {
            MetadataType::Cover => "cover",
            MetadataType::Chapters => "chapters",
            MetadataType::Captions => "captions",
        };

        // Resolve the video file path (with path-traversal protection) *before*
        // computing the cache key so the key can be scoped to the file's mtime
        // (finding #13). If the file no longer exists, fall through to 404.
        let Some(video_file) =
            resolve_media_source_file(video_url_path, &config.base_dir, &config.static_folder)
        else {
            tracing::debug!(
                "Video file not found for metadata generation: {}",
                video_url_path
            );
            return None;
        };

        // mtime-scoped key: editing the source file yields a new key so stale
        // positive/negative entries are naturally missed and re-extracted.
        let key = cache_key_with_mtime(&video_file, cache_type_str);

        // Fast path: serve a cached result (may be a negative marker).
        if let Some(cached) = config.video_metadata_cache.get(&key) {
            return Self::metadata_response_from_cache(cached);
        }

        // Single-flight (finding #20): ensure only one ffmpeg decode runs per
        // (path, type). Either we claim the slot and produce, or another request
        // is already producing and we await its result. Mirrors the race-free
        // HlsCache pattern (register interest before re-checking).
        match claim_inflight(&config.metadata_inflight, &key) {
            InflightClaim::Produce(notify) => {
                tracing::debug!(
                    "Generating {} for: {}",
                    cache_type_str,
                    video_file.display()
                );
                // Hold the slot in a drop guard: a client disconnect drops this
                // future at the await below, and a release that only runs on the
                // success path would leave `key` claimed forever, wedging every
                // later request for it into a full `METADATA_WAIT_TIMEOUT` wait
                // followed by a 404. The guard releases the slot and wakes any
                // waiters (who then read the freshly-populated cache entry) on
                // every exit path.
                let _slot =
                    InflightSlot::new(Arc::clone(&config.metadata_inflight), key.clone(), notify);
                Self::extract_video_metadata_and_cache(
                    video_file,
                    metadata_type,
                    key.clone(),
                    config,
                )
                .await
            }
            InflightClaim::Wait(notify) => {
                // Register interest before re-checking so a completion that lands
                // between claim and here is not missed.
                let notified = notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if let Some(cached) = config.video_metadata_cache.get(&key) {
                    return Self::metadata_response_from_cache(cached);
                }
                match tokio::time::timeout(METADATA_WAIT_TIMEOUT, notified).await {
                    Ok(()) => config
                        .video_metadata_cache
                        .get(&key)
                        .and_then(Self::metadata_response_from_cache),
                    Err(_) => {
                        tracing::warn!(
                            "Timed out waiting for in-progress metadata decode: {}",
                            key
                        );
                        None
                    }
                }
            }
        }
    }

    /// Try to serve a PDF cover sidecar file, handling staleness detection.
    ///
    /// This is called from the StaticFile branch when a `.pdf.cover.jpg` sidecar exists.
    /// It checks if the sidecar is stale (PDF modified after sidecar) and regenerates if needed.
    ///
    /// Returns:
    /// - `Some(Response)` if the sidecar is stale and we regenerated the cover
    /// - `None` if the sidecar is fresh (caller should serve as normal static file)
    ///   or if this is not a PDF cover sidecar request
    #[cfg(feature = "media-metadata")]
    async fn try_serve_pdf_cover_sidecar(
        url_path: &str,
        sidecar_file_path: &std::path::Path,
        config: &ServerState,
    ) -> Option<Response<Body>> {
        use crate::pdf_metadata::parse_pdf_cover_request;
        use crate::video_metadata_cache::{CachedMetadata, cache_key_with_mtime};

        // Check if this is a PDF cover request
        let _pdf_url_path = parse_pdf_cover_request(url_path)?;

        // Find the PDF file path (corresponding to this sidecar)
        // The sidecar is at {pdf_path}.cover.jpg, so remove .cover.jpg to get pdf_path
        let pdf_file = {
            let sidecar_str = sidecar_file_path.to_str()?;
            let pdf_path_str = sidecar_str.strip_suffix(".cover.jpg")?;
            std::path::PathBuf::from(pdf_path_str)
        };

        // Build an mtime-scoped cache key from the resolved source file so an
        // edited PDF invalidates a stale cached cover at runtime (finding #13).
        let key = cache_key_with_mtime(&pdf_file, "pdf_cover");

        // Check memory cache first
        if let Some(cached) = config.video_metadata_cache.get(&key) {
            return match cached {
                CachedMetadata::Cover(bytes) => Some(Self::build_jpg_response(bytes)),
                CachedMetadata::NotAvailable => None, // Cached negative result
                _ => None,                            // Other types not relevant for PDF covers
            };
        }

        // If PDF doesn't exist, let static file serving handle it
        if !pdf_file.is_file() {
            // Cache and serve the sidecar contents
            if let Ok(bytes) = tokio::fs::read(sidecar_file_path).await {
                tracing::debug!(
                    "Serving PDF cover sidecar (orphaned, no PDF): {}",
                    sidecar_file_path.display()
                );
                config
                    .video_metadata_cache
                    .insert(key, CachedMetadata::Cover(bytes.clone()));
                return Some(Self::build_jpg_response(bytes));
            }
            return None;
        }

        // Compare modification times
        let pdf_meta = tokio::fs::metadata(&pdf_file).await.ok()?;
        let sidecar_meta = tokio::fs::metadata(sidecar_file_path).await.ok()?;
        let pdf_mtime = pdf_meta.modified().ok()?;
        let sidecar_mtime = sidecar_meta.modified().ok()?;

        if pdf_mtime > sidecar_mtime {
            // Sidecar is stale - regenerate
            tracing::debug!(
                "Sidecar is stale (PDF modified after sidecar), regenerating: {}",
                sidecar_file_path.display()
            );

            // Generate new cover (async with concurrency control)
            match crate::pdf_metadata::extract_cover_async(&pdf_file).await {
                Ok(bytes) => {
                    config
                        .video_metadata_cache
                        .insert(key, CachedMetadata::Cover(bytes.clone()));
                    return Some(Self::build_jpg_response(bytes));
                }
                Err(e) => {
                    tracing::debug!("Failed to regenerate PDF cover: {}", e);
                    // Fall through to serve stale sidecar instead of failing
                }
            }
        }

        // Sidecar is fresh (or regeneration failed) - read, cache, and serve
        if let Ok(bytes) = tokio::fs::read(sidecar_file_path).await {
            tracing::debug!(
                "Serving PDF cover from fresh sidecar: {}",
                sidecar_file_path.display()
            );
            config
                .video_metadata_cache
                .insert(key, CachedMetadata::Cover(bytes.clone()));
            return Some(Self::build_jpg_response(bytes));
        }

        // Let static file serving handle it
        None
    }

    /// Try to serve dynamically generated PDF cover image.
    ///
    /// Returns Some(Response) if the request was for a PDF cover image and we successfully
    /// generated it, None otherwise (fall through to 404).
    ///
    /// Request pattern: `/path/to/document.pdf.cover.jpg` -> extract cover from `/path/to/document.pdf`
    ///
    /// This function implements accelerated serving with pre-generated covers:
    /// 1. If a sidecar file (e.g., `document.pdf.cover.jpg`) exists and is newer than the PDF,
    ///    it is served directly from disk (with memory caching for subsequent requests).
    /// 2. If the sidecar is stale (PDF modified after sidecar), the cover is regenerated.
    /// 3. If no sidecar exists, the cover is dynamically generated from the PDF.
    #[cfg(feature = "media-metadata")]
    async fn try_serve_pdf_cover(path: &str, config: &ServerState) -> Option<Response<Body>> {
        use crate::pdf_metadata::parse_pdf_cover_request;
        use crate::video_metadata_cache::{CachedMetadata, cache_key_with_mtime};

        // Check if this is a PDF cover request
        let pdf_url_path = parse_pdf_cover_request(path)?;

        // Resolve the PDF file path (with path-traversal protection) *before*
        // computing the cache key so the key can be scoped to the file's mtime
        // (finding #13). First try the direct path, then the static_folder prefix.
        let Some(pdf_file) =
            resolve_media_source_file(pdf_url_path, &config.base_dir, &config.static_folder)
        else {
            tracing::debug!("PDF file not found for cover generation: {}", pdf_url_path);
            return None;
        };

        // Build an mtime-scoped cache key so an edited PDF invalidates a stale
        // cached cover at runtime (finding #13).
        let key = cache_key_with_mtime(&pdf_file, "pdf_cover");

        // Check memory cache first
        if let Some(cached) = config.video_metadata_cache.get(&key) {
            return match cached {
                CachedMetadata::Cover(bytes) => Some(Self::build_jpg_response(bytes)),
                CachedMetadata::NotAvailable => None, // Cached negative result
                _ => None,                            // Other types not relevant for PDF covers
            };
        }

        // Build sidecar path: {pdf_path}.cover.jpg
        let sidecar_path = {
            let mut sidecar = pdf_file.clone();
            let file_name = sidecar.file_name()?.to_str()?;
            sidecar.set_file_name(format!("{}.cover.jpg", file_name));
            sidecar
        };

        // Check if we can serve from sidecar file
        if let Some(bytes) = Self::try_serve_from_sidecar(&pdf_file, &sidecar_path).await {
            tracing::debug!("Serving PDF cover from sidecar: {}", sidecar_path.display());
            // Cache the sidecar contents for subsequent requests
            config
                .video_metadata_cache
                .insert(key, CachedMetadata::Cover(bytes.clone()));
            return Some(Self::build_jpg_response(bytes));
        }

        // Sidecar doesn't exist or is stale - generate dynamically
        tracing::debug!("Generating PDF cover for: {}", pdf_file.display());

        // Generate the cover image (async with concurrency control)
        match crate::pdf_metadata::extract_cover_async(&pdf_file).await {
            Ok(bytes) => {
                config
                    .video_metadata_cache
                    .insert(key, CachedMetadata::Cover(bytes.clone()));
                Some(Self::build_jpg_response(bytes))
            }
            Err(crate::errors::PdfMetadataError::PasswordProtected { .. }) => {
                tracing::debug!("PDF is password-protected: {}", pdf_file.display());
                config
                    .video_metadata_cache
                    .insert(key, CachedMetadata::NotAvailable);
                None
            }
            Err(e) => {
                tracing::debug!("Failed to extract PDF cover: {}", e);
                config
                    .video_metadata_cache
                    .insert(key, CachedMetadata::NotAvailable);
                None
            }
        }
    }

    /// Try to serve a PDF cover from a pre-generated sidecar file.
    ///
    /// Returns `Some(bytes)` if:
    /// 1. The sidecar file exists
    /// 2. The sidecar is newer than the PDF (not stale)
    /// 3. The file can be read successfully
    ///
    /// Returns `None` if the sidecar doesn't exist, is stale, or can't be read.
    #[cfg(feature = "media-metadata")]
    async fn try_serve_from_sidecar(
        pdf_path: &std::path::Path,
        sidecar_path: &std::path::Path,
    ) -> Option<Vec<u8>> {
        // Ensure the sidecar path stays within the same directory as the validated PDF path.
        // This provides an additional defense-in-depth check against path traversal before
        // performing any filesystem operations on the sidecar file.
        if let Some(pdf_dir) = pdf_path.parent() {
            // First, ensure that the sidecar's parent directory is exactly the same as the PDF's
            // parent directory. Since `sidecar_path` was constructed from `pdf_path` by only
            // changing the file name, any deviation here indicates an unexpected or unsafe path.
            if let Some(sidecar_dir) = sidecar_path.parent() {
                if sidecar_dir != pdf_dir {
                    tracing::warn!(
                        "Sidecar path is not in the same directory as PDF; skipping sidecar. \
                         pdf_dir='{}', sidecar_dir='{}'",
                        pdf_dir.display(),
                        sidecar_dir.display()
                    );
                    return None;
                }
            } else {
                // A sidecar without a parent directory is unexpected; treat as invalid.
                tracing::warn!(
                    "Sidecar path has no parent directory; skipping sidecar: {}",
                    sidecar_path.display()
                );
                return None;
            }

            // Additionally, validate that the sidecar path, once canonicalized, still resides
            // under the PDF's directory. This guards against any remaining path traversal risks.
            if validate_path_containment(sidecar_path, pdf_dir).is_none() {
                tracing::warn!(
                    "Sidecar path failed containment validation: {}",
                    sidecar_path.display()
                );
                return None;
            }
        } else {
            // If the PDF has no parent directory, treat this as invalid and do not use the sidecar.
            tracing::warn!(
                "PDF path has no parent directory; skipping sidecar: {}",
                pdf_path.display()
            );
            return None;
        }

        // Check if sidecar exists
        let sidecar_meta = tokio::fs::metadata(sidecar_path).await.ok()?;

        // Get PDF modification time for staleness check
        let pdf_meta = tokio::fs::metadata(pdf_path).await.ok()?;
        let pdf_mtime = pdf_meta.modified().ok()?;
        let sidecar_mtime = sidecar_meta.modified().ok()?;

        // If PDF is newer than sidecar, sidecar is stale
        if pdf_mtime > sidecar_mtime {
            tracing::debug!(
                "Sidecar is stale (PDF modified after sidecar): {}",
                sidecar_path.display()
            );
            return None;
        }

        // Read and return sidecar contents
        tokio::fs::read(sidecar_path).await.ok()
    }

    /// Try to serve links.json for bidirectional link tracking.
    ///
    /// Returns Some(Response) if the request was for links.json and we successfully
    /// generated it, None otherwise (fall through to 404).
    async fn try_serve_links_json(path: &str, config: &ServerState) -> Option<Response<Body>> {
        use crate::link_index::PageLinks;

        // Check if this is a links.json request
        if !path.ends_with("links.json") {
            return None;
        }

        // If link tracking is disabled, return None (404)
        if !config.link_tracking {
            tracing::debug!("links.json requested but link tracking is disabled");
            return None;
        }

        // Extract the page URL path from the request
        // e.g., "docs/guide/links.json" -> "/docs/guide/". The axum catch-all
        // delivers `path` without a leading slash, so add one explicitly (as
        // `try_serve_errors_json` does): this is the shared `link_cache` /
        // `inbound_link_cache` key, and the site-absolute form is what
        // `markdown_to_html` and `try_serve_errors_json` write. Keying it
        // `docs/guide/` here instead made every render's back-fill invisible to
        // this handler, so an edited page kept serving its first-seen links.
        let page_path = path.strip_suffix("links.json")?;
        let page_url_path = if page_path.is_empty() || page_path == "/" {
            "/".to_string()
        } else {
            let normalized = page_path.trim_end_matches('/').trim_start_matches('/');
            format!("/{}/", normalized)
        };

        tracing::debug!("links.json request for page: {}", page_url_path);

        // Check if the page exists and get outbound links
        // If not cached, we need to verify the page exists and render it to extract links
        let outbound = if let Some(cached) = config.link_cache.get(&page_url_path) {
            cached
        } else {
            // Resolve the path to find the markdown file
            let tag_url_sources = crate::config::tag_sources_to_url_sources(&config.tag_sources);
            let resolver_config = PathResolverConfig {
                base_dir: &config.base_dir,
                canonical_base_dir: config.canonical_base_dir.as_deref(),
                static_folder: &config.static_folder,
                markdown_extensions: &config.markdown_extensions,
                index_file: &config.index_file,
                tag_sources: &tag_url_sources,
            };

            // Convert page_url_path to a request path for the resolver
            // "/docs/guide/" -> "docs/guide"
            let request_path = page_url_path.trim_matches('/');

            match resolve_request_path(&resolver_config, request_path) {
                ResolvedPath::MarkdownFile(md_path) => {
                    tracing::debug!(
                        "links.json: rendering page to extract links: {:?}",
                        &md_path
                    );

                    // Render the page to extract outbound links
                    let is_index_file = md_path
                        .file_name()
                        .and_then(|f| f.to_str())
                        .is_some_and(|f| f == config.index_file);

                    let link_transform_config = LinkTransformConfig {
                        markdown_extensions: config.markdown_extensions.clone(),
                        index_file: config.index_file.clone(),
                        is_index_file,
                        url_depth: None,
                        current_page_url: page_url_path.clone(),
                        markdown_page_probe: None,
                    };

                    let valid_tag_sources = crate::config::tag_sources_to_set(&config.tag_sources);
                    match markdown::render_with_cache(
                        md_path,
                        &config.base_dir,
                        config.oembed_timeout_ms,
                        link_transform_config,
                        Some(config.oembed_cache.clone()),
                        true,  // server_mode
                        false, // transcode_enabled (not needed for link extraction)
                        valid_tag_sources,
                        false, // mark_incomplete: not needed for link extraction
                        &config.incomplete_markers,
                        Some(config.repo.wikilink_index.clone()),
                    )
                    .await
                    {
                        Ok(render_result) => {
                            // Resolve relative URLs to absolute before caching
                            let resolved_links = resolve_outbound_links(
                                &page_url_path,
                                render_result.outbound_links,
                                is_index_file,
                            );
                            // Cache the outbound links
                            config
                                .link_cache
                                .insert(page_url_path.clone(), resolved_links.clone());
                            resolved_links
                        }
                        Err(e) => {
                            tracing::error!("links.json: failed to render page: {}", e);
                            return None;
                        }
                    }
                }
                ResolvedPath::TagPage { source, value } => {
                    tracing::debug!(
                        "links.json: building tag page links for {}/{}",
                        source,
                        value
                    );
                    build_tag_page_outbound_links(
                        &source,
                        &value,
                        &config.repo.tag_index,
                        &config.tag_sources,
                    )
                }
                ResolvedPath::TagSourceIndex { source } => {
                    tracing::debug!("links.json: building tag index links for {}", source);
                    build_tag_index_outbound_links(&source, &config.repo.tag_index)
                }
                _ => {
                    // Page doesn't exist
                    tracing::debug!("links.json: page not found: {}", page_url_path);
                    return None;
                }
            }
        };

        // Get inbound links from cache, or grep with single-flight + bounded
        // concurrency: each miss walks the whole repository, and the sidebar
        // mini graph fans out many links.json requests at once, so a burst
        // must not stampede the filesystem. Mirrors the race-free video
        // metadata pattern (register interest before re-checking).
        let inbound = if config.inbound_index.is_ready() {
            // The repository-wide index is authoritative once built: it was
            // produced by inverting every page's resolved links, the same way
            // static builds do, so no grep and no per-page cache is involved.
            config.inbound_index.get(&page_url_path)
        } else if let Some(cached) = config.inbound_link_cache.get(&page_url_path) {
            cached
        } else {
            match claim_inflight(&config.inbound_grep_inflight, &page_url_path) {
                InflightClaim::Produce(notify) => {
                    // Release the slot and wake waiters on every exit path —
                    // failure, `?`, panic, and a client disconnect that drops
                    // this future at the grep await — so waiters degrade to a
                    // retryable miss instead of hanging and the key is never
                    // left permanently claimed.
                    let _slot = InflightSlot::new(
                        Arc::clone(&config.inbound_grep_inflight),
                        page_url_path.clone(),
                        notify,
                    );
                    // Re-check the cache after winning the slot: a previous
                    // producer may have populated it between our miss and the
                    // claim.
                    let links = match config.inbound_link_cache.get(&page_url_path) {
                        Some(cached) => Some(cached),
                        None => Self::grep_inbound_links_bounded(&page_url_path, config).await,
                    };
                    links?
                }
                InflightClaim::Wait(notify) => {
                    // Register interest before re-checking so a completion
                    // that lands between claim and here is not missed.
                    let notified = notify.notified();
                    tokio::pin!(notified);
                    notified.as_mut().enable();
                    if let Some(cached) = config.inbound_link_cache.get(&page_url_path) {
                        cached
                    } else {
                        match tokio::time::timeout(INBOUND_GREP_WAIT_TIMEOUT, notified).await {
                            Ok(()) => config.inbound_link_cache.get(&page_url_path)?,
                            Err(_) => {
                                tracing::warn!(
                                    "Timed out waiting for in-progress inbound link grep: {}",
                                    page_url_path
                                );
                                return None;
                            }
                        }
                    }
                }
            }
        };

        // Typed relationships (declared + derived) for this page, if enabled.
        // The index normalises keys (leading slash) on both insert and lookup,
        // so `page_url_path` can be passed through without compensation.
        let relationships = if config.relationship_tracking {
            config.repo.relationship_index.get(&page_url_path)
        } else {
            Vec::new()
        };

        let page_links = PageLinks {
            inbound,
            outbound,
            relationships,
        };

        let json = match serde_json::to_string(&page_links) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!("Failed to serialize links.json: {}", e);
                return None;
            }
        };

        Some(build_response_or_500(
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_CACHE)
                .body(Body::from(json)),
        ))
    }

    /// Runs the inbound-link grep for `page_url_path` on a blocking thread,
    /// bounded by the inbound-grep semaphore, and caches the result. Returns
    /// `None` if the task fails (callers degrade to a retryable miss).
    async fn grep_inbound_links_bounded(
        page_url_path: &str,
        config: &ServerState,
    ) -> Option<Vec<crate::link_index::InboundLink>> {
        use crate::link_grep::find_inbound_links;

        // Bound concurrent full-repo walks. The semaphore is never closed, so
        // acquire only fails if the runtime is shutting down.
        let _permit = config.inbound_grep_semaphore.acquire().await.ok()?;

        // Grep for inbound links. This walks the whole repository (blocking
        // filesystem + CPU work), so run it on a blocking thread to avoid
        // stalling the async worker. All captured data is owned/`Send`.
        let target = page_url_path.to_string();
        let base_dir = config.base_dir.clone();
        let markdown_extensions = config.markdown_extensions.clone();
        let ignore_dirs = config.ignore_dirs.clone();
        let ignore_globs = config.ignore_globs.clone();
        let index_file = config.index_file.clone();
        let links = tokio::task::spawn_blocking(move || {
            find_inbound_links(
                &target,
                &base_dir,
                &markdown_extensions,
                &ignore_dirs,
                &ignore_globs,
                &index_file,
            )
        })
        .await
        .inspect_err(|e| tracing::error!("inbound link grep task failed: {e}"))
        .ok()?;
        // Cache the result
        config
            .inbound_link_cache
            .insert(page_url_path.to_string(), links.clone());
        Some(links)
    }

    /// Try to serve `errors.json` for per-page problem reporting.
    ///
    /// This endpoint is server/GUI only — static builds never register it and
    /// the template guards the corresponding `<mbr-page-errors>` element with
    /// `{% if server_mode %}`. Triple-gated to guarantee zero leakage into
    /// static output.
    ///
    /// Returns:
    /// - `Some(200 JSON)` with an `errors: []` array (possibly empty) when the
    ///   request targets a valid page.
    /// - `None` (→ 404) when the path is not `errors.json`, when link tracking
    ///   is disabled, or when the underlying page does not exist.
    async fn try_serve_errors_json(path: &str, config: &ServerState) -> Option<Response<Body>> {
        use crate::page_errors::{
            PageErrors, ambiguous_relationship_endpoint_errors, ambiguous_wikilink_errors,
            detect_unresolved_wikilinks, frontmatter_parse_errors, relationship_cycle_errors,
            validate_internal_links, validate_media_references, validate_rendered_links,
        };

        // Only handle exact `errors.json` tails. This keeps the endpoint
        // tightly scoped and avoids colliding with a user-authored file that
        // happens to have "errors" in its stem.
        if !path.ends_with("errors.json") {
            return None;
        }

        // Respect the `--no-link-tracking` flag: the whole feature is gated on
        // link tracking being on, so reuse that switch rather than introducing
        // a new toggle. When disabled, the component stays silent (see
        // `mbr-page-errors.ts`'s 404 handling).
        if !config.link_tracking {
            tracing::debug!("errors.json requested but link tracking is disabled");
            return None;
        }

        // Reconstruct the canonical page URL for the resolver.
        // e.g. "docs/guide/errors.json" -> "/docs/guide/". The axum catch-all
        // delivers `path` without a leading slash, so we add one explicitly
        // to yield a canonical site-absolute URL in the JSON payload.
        let page_path = path.strip_suffix("errors.json")?;
        let page_url_path = if page_path.is_empty() || page_path == "/" {
            "/".to_string()
        } else {
            let normalized = page_path.trim_end_matches('/').trim_start_matches('/');
            format!("/{}/", normalized)
        };

        tracing::debug!("errors.json request for page: {}", page_url_path);

        let tag_url_sources = crate::config::tag_sources_to_url_sources(&config.tag_sources);
        let resolver_config = PathResolverConfig {
            base_dir: &config.base_dir,
            canonical_base_dir: config.canonical_base_dir.as_deref(),
            static_folder: &config.static_folder,
            markdown_extensions: &config.markdown_extensions,
            index_file: &config.index_file,
            tag_sources: &tag_url_sources,
        };

        let request_path = page_url_path.trim_matches('/');

        // Resolve the page and render if it is a markdown file. We need the
        // rendered HTML for media / wikilink scans (the `LinkCache` only holds
        // outbound links), so unlike `try_serve_links_json` we cannot short-
        // circuit through the cache when the HTML is missing.
        let (outbound_links, html_for_scan, frontmatter_error, ambiguous_wikilinks): (
            Vec<crate::link_index::OutboundLink>,
            String,
            Option<String>,
            Vec<crate::wikilink_index::AmbiguousWikilink>,
        ) = match resolve_request_path(&resolver_config, request_path) {
            ResolvedPath::MarkdownFile(md_path) => {
                let is_index_file = md_path
                    .file_name()
                    .and_then(|f| f.to_str())
                    .is_some_and(|f| f == config.index_file);

                let link_transform_config = LinkTransformConfig {
                    markdown_extensions: config.markdown_extensions.clone(),
                    index_file: config.index_file.clone(),
                    is_index_file,
                    url_depth: None,
                    current_page_url: page_url_path.clone(),
                    // Must match `markdown_to_html`: the hrefs this render
                    // produces are the ones `validate_rendered_links` judges,
                    // so a checker rendering without the probe would report
                    // every extension-less markdown link as non-canonical while
                    // the page a reader sees is perfectly fine.
                    markdown_page_probe: Some(
                        crate::link_transform::filesystem_markdown_page_probe(
                            owned_resolver_config(config),
                        ),
                    ),
                };

                let valid_tag_sources = crate::config::tag_sources_to_set(&config.tag_sources);

                match markdown::render_with_cache(
                    md_path,
                    &config.base_dir,
                    config.oembed_timeout_ms,
                    link_transform_config,
                    Some(config.oembed_cache.clone()),
                    true,  // server_mode
                    false, // transcode_enabled (not needed for error scan)
                    valid_tag_sources,
                    false, // mark_incomplete: not needed for error scan
                    &config.incomplete_markers,
                    Some(config.repo.wikilink_index.clone()),
                )
                .await
                {
                    Ok(render_result) => {
                        let resolved_links = resolve_outbound_links(
                            &page_url_path,
                            render_result.outbound_links,
                            is_index_file,
                        );
                        // Back-fill the link cache so a subsequent links.json
                        // call is a hit (mirrors `try_serve_links_json`).
                        config
                            .link_cache
                            .insert(page_url_path.clone(), resolved_links.clone());
                        (
                            resolved_links,
                            render_result.html,
                            render_result.frontmatter_error,
                            render_result.ambiguous_wikilinks,
                        )
                    }
                    Err(e) => {
                        tracing::error!("errors.json: failed to render page: {}", e);
                        return None;
                    }
                }
            }
            ResolvedPath::TagPage { source, value } => {
                // Tag pages have no authored HTML and no media; there is
                // nothing for the validators to flag. Return an empty
                // payload so the UI can stay silent.
                let outbound = build_tag_page_outbound_links(
                    &source,
                    &value,
                    &config.repo.tag_index,
                    &config.tag_sources,
                );
                (outbound, String::new(), None, Vec::new())
            }
            ResolvedPath::TagSourceIndex { source } => {
                let outbound = build_tag_index_outbound_links(&source, &config.repo.tag_index);
                (outbound, String::new(), None, Vec::new())
            }
            _ => {
                tracing::debug!("errors.json: page not found: {}", page_url_path);
                return None;
            }
        };

        let mut errors = Vec::new();
        errors.extend(frontmatter_parse_errors(&frontmatter_error));
        if html_for_scan.is_empty() {
            // Tag pages and tag indexes have no authored body; their outbound
            // links are synthesized absolute URLs, so that list is the only
            // thing to check.
            errors.extend(validate_internal_links(&outbound_links, &resolver_config));
        } else {
            // For a rendered page, read the hrefs that actually went into the
            // HTML. `outbound_links` holds the *authored* destinations resolved
            // with markdown semantics, so re-resolving those re-applies the
            // same rules the transform used and can never see a transform bug —
            // which is exactly how a missing trailing slash stayed invisible.
            errors.extend(validate_rendered_links(
                &html_for_scan,
                &resolver_config,
                &page_url_path,
            ));
            errors.extend(validate_media_references(
                &html_for_scan,
                &resolver_config,
                &page_url_path,
            ));
            errors.extend(detect_unresolved_wikilinks(&html_for_scan));
            #[cfg(feature = "media-metadata")]
            errors.extend(
                Self::detect_unplayable_media(
                    &html_for_scan,
                    &resolver_config,
                    &page_url_path,
                    config,
                )
                .await,
            );
        }
        errors.extend(ambiguous_wikilink_errors(&ambiguous_wikilinks));

        // Relationship data problems. Detected once per index rebuild, so this
        // is a pair of map lookups; gated on the feature that produced them so
        // `--no-relationship-tracking` reports nothing (the index is empty then
        // anyway, but the gate keeps the contract explicit).
        if config.relationship_tracking {
            let index = &config.repo.relationship_index;
            errors.extend(relationship_cycle_errors(&index.cycles_for(&page_url_path)));
            errors.extend(ambiguous_relationship_endpoint_errors(
                &index.ambiguous_endpoints_for(&page_url_path),
            ));
        }

        let payload = PageErrors {
            page_url: page_url_path,
            errors,
        };

        let json = match serde_json::to_string(&payload) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!("Failed to serialize errors.json: {}", e);
                return None;
            }
        };

        Some(build_response_or_500(
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_CACHE)
                .body(Body::from(json)),
        ))
    }

    /// Probes a media file for tracks that break browser playback, memoized by
    /// path+mtime.
    ///
    /// `probe_playback_compatibility` opens the container (blocking work), so a
    /// cache miss runs on a blocking thread. Both outcomes are cached, so a
    /// page with fine videos costs one probe per file for the lifetime of the
    /// server rather than one per `errors.json` request.
    #[cfg(feature = "media-metadata")]
    async fn probe_playback_compat_cached(
        media_file: &std::path::Path,
        config: &ServerState,
    ) -> Option<crate::video_metadata::PlaybackCompatibility> {
        use crate::video_metadata::probe_playback_compatibility;
        use crate::video_metadata_cache::cache_key_with_mtime;

        let key = cache_key_with_mtime(media_file, "playback-compat");
        if let Some(compat) = config.media_compat_cache.pin().get(&key).cloned() {
            return Some(compat);
        }

        let path = media_file.to_path_buf();
        let probed = tokio::task::spawn_blocking(move || probe_playback_compatibility(&path))
            .await
            .inspect_err(|e| tracing::warn!("playback-compat probe task failed: {e}"))
            .ok()?
            .inspect_err(|e| tracing::debug!("playback-compat probe of {media_file:?}: {e}"))
            .ok()?;

        config.media_compat_cache.pin().insert(key, probed.clone());
        Some(probed)
    }

    /// Detects videos on the page that resolve and serve correctly but that
    /// the browser cannot decode.
    ///
    /// Deliberately lives here rather than in `vid.rs` / `markdown.rs`: this
    /// opens every referenced video with ffmpeg, which must never sit on the
    /// markdown render path. `errors.json` is fetched lazily in the background
    /// by `<mbr-page-errors>`, so the cost is off the critical path, bounded by
    /// [`MEDIA_COMPAT_PROBE_CONCURRENCY`], and paid at most once per
    /// file+mtime.
    #[cfg(feature = "media-metadata")]
    async fn detect_unplayable_media(
        html: &str,
        resolver_config: &PathResolverConfig<'_>,
        page_url: &str,
        config: &ServerState,
    ) -> Vec<crate::page_errors::PageError> {
        use crate::page_errors::{MediaKind, PageError, collect_media_references};
        use crate::video_metadata::{PlaybackCompatibility, has_video_extension};
        use futures::stream::StreamExt;
        use itertools::Itertools;

        // `collect_media_references` already dedupes by src; filtering to video
        // extensions keeps images and PDFs out of the ffmpeg path entirely.
        let references: Vec<_> = collect_media_references(html, resolver_config, page_url)
            .into_iter()
            .filter(|reference| has_video_extension(&reference.path.to_string_lossy()))
            .collect();

        if references.is_empty() {
            return Vec::new();
        }

        // One probe per distinct file. A page can reference the same clip under
        // several spellings (the reported repro embeds it both percent-encoded
        // and angle-bracketed), and each spelling needs its own error so the
        // frontend can match every element — but the bytes are read once.
        //
        // `buffered` (not `buffer_unordered`) bounds concurrency without making
        // the result order depend on probe timing.
        let unique_paths: Vec<PathBuf> = references
            .iter()
            .map(|reference| reference.path.clone())
            .unique()
            .collect();

        let probes: std::collections::HashMap<PathBuf, PlaybackCompatibility> =
            futures::stream::iter(unique_paths)
                .map(|path| async move {
                    let compat = Self::probe_playback_compat_cached(&path, config).await?;
                    Some((path, compat))
                })
                .buffered(MEDIA_COMPAT_PROBE_CONCURRENCY)
                .filter_map(|probe| async move { probe })
                .collect()
                .await;

        references
            .into_iter()
            .filter_map(|reference| {
                let compat = probes.get(&reference.path)?;
                let reason = compat.reason()?;
                Some(PageError::UnplayableMedia {
                    src: reference.src,
                    // The diagnosis is about the file, not the element, so the
                    // media type is reported rather than `source`.
                    kind: MediaKind::Video,
                    reason,
                    remedy: compat.remedy(),
                    // Heuristic hint, never a verdict — see the variant docs.
                    advisory: true,
                })
            })
            .collect()
    }

    /// Build a JPEG image response.
    #[cfg(feature = "media-metadata")]
    fn build_jpg_response(bytes: Vec<u8>) -> Response<Body> {
        build_response_or_500(
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "image/jpeg")
                .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_CACHE)
                .body(Body::from(bytes)),
        )
    }

    /// Build a WebVTT response.
    #[cfg(feature = "media-metadata")]
    fn build_vtt_response(vtt: String) -> Response<Body> {
        build_response_or_500(
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/vtt; charset=utf-8")
                .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_CACHE)
                .body(Body::from(vtt)),
        )
    }

    /// Build an HLS playlist response.
    #[cfg(feature = "media-metadata")]
    fn build_hls_playlist_response(playlist: Arc<Vec<u8>>) -> Response<Body> {
        build_response_or_500(
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")
                .header(header::CONTENT_LENGTH, playlist.len())
                .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_CACHE)
                .body(Body::from(playlist.as_ref().clone())),
        )
    }

    /// Build an HLS segment response.
    #[cfg(feature = "media-metadata")]
    fn build_hls_segment_response(segment: Arc<Vec<u8>>) -> Response<Body> {
        build_response_or_500(
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "video/mp2t")
                .header(header::CONTENT_LENGTH, segment.len())
                .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_CACHE)
                .body(Body::from(segment.as_ref().clone())),
        )
    }

    /// Build a response for one part of the stream-copy (remux) variant.
    #[cfg(feature = "media-metadata")]
    fn build_remux_response(
        part: crate::video_remux::RemuxPart,
        data: Arc<Vec<u8>>,
    ) -> Response<Body> {
        use crate::video_remux::{
            INIT_CONTENT_TYPE, PLAYLIST_CONTENT_TYPE, RemuxPart, SEGMENT_CONTENT_TYPE,
        };

        let content_type = match part {
            RemuxPart::Playlist => PLAYLIST_CONTENT_TYPE,
            RemuxPart::Init => INIT_CONTENT_TYPE,
            RemuxPart::Segment(_) => SEGMENT_CONTENT_TYPE,
        };

        build_response_or_500(
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .header(header::CONTENT_LENGTH, data.len())
                .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_CACHE)
                .body(Body::from(data.as_ref().clone())),
        )
    }

    /// Serve one part of the remux variant from cache, or generate it exactly
    /// once and serve that.
    ///
    /// Wraps the single-flight state machine in `HlsCache` so N concurrent
    /// requests for the same key run one generation: the winner produces and the
    /// rest await its result. That, plus the caching, is what bounds the
    /// concurrent ffmpeg work a player can provoke.
    ///
    /// The generation is detached (see `HlsCache::spawn_generation`), so a client
    /// that disconnects mid-request — which Safari's HLS loader does routinely —
    /// cannot cancel the work or skip the cache bookkeeping. Awaiting the returned
    /// `JoinHandle` is how this request reads the result; dropping it merely stops
    /// listening.
    ///
    /// Every failure carries a status: an empty body or a bare 404 would leave a
    /// player retrying or stalled with nothing to report.
    #[cfg(feature = "media-metadata")]
    async fn generate_remux_part<F>(
        cache: &Arc<crate::video_transcode_cache::HlsCache>,
        key: crate::video_transcode_cache::HlsCacheKey,
        generate: F,
    ) -> Result<Arc<Vec<u8>>, Response<Body>>
    where
        F: FnOnce() -> Result<Vec<u8>, crate::video_transcode::TranscodeError> + Send + 'static,
    {
        use crate::video_transcode_cache::{HLS_WAIT_TIMEOUT, HlsCache, HlsCacheStartResult};

        match cache.start_generation(key.clone()) {
            HlsCacheStartResult::Started(notify) => {
                let handle = HlsCache::spawn_generation(cache, key.clone(), notify, generate);
                match handle.await {
                    Ok(Ok(data)) => Ok(data),
                    Ok(Err(error)) => Err(Self::build_remux_error_response(&error)),
                    // The detached task itself failed to finish; its guard has
                    // released the claim, so a retry can succeed.
                    Err(join_error) => {
                        tracing::warn!("remux generation task did not finish: {join_error}");
                        Err(Self::build_remux_retry_response(
                            "generation did not complete",
                        ))
                    }
                }
            }
            HlsCacheStartResult::AlreadyInProgress(notify) => {
                match cache
                    .wait_for_completion(&key, notify, HLS_WAIT_TIMEOUT)
                    .await
                {
                    Some(data) => Ok(data),
                    // Not complete: either the producer recorded a failure, or
                    // the wait timed out / the claim was released. Distinguish
                    // them so the client is told something true.
                    None => Err(Self::remux_incomplete_response(cache, &key)),
                }
            }
            HlsCacheStartResult::AlreadyComplete(data) => Ok(data),
            HlsCacheStartResult::PreviouslyFailed(message) => {
                tracing::debug!("previous remux generation failed: {message}");
                Err(Self::build_remux_unprocessable_response(&message))
            }
            // With caching off there is no entry to settle, but the work is still
            // detached so a disconnect cannot orphan a half-finished mux.
            HlsCacheStartResult::CacheDisabled => {
                let notify = Arc::new(tokio::sync::Notify::new());
                let handle = HlsCache::spawn_generation(cache, key, notify, generate);
                match handle.await {
                    Ok(Ok(data)) => Ok(data),
                    Ok(Err(error)) => Err(Self::build_remux_error_response(&error)),
                    Err(join_error) => {
                        tracing::warn!("remux generation task did not finish: {join_error}");
                        Err(Self::build_remux_retry_response(
                            "generation did not complete",
                        ))
                    }
                }
            }
        }
    }

    /// Explain why a waiter never saw completed content.
    ///
    /// A `Failed` entry means the producer got a real error worth reporting; the
    /// alternative (timed out, or the claim was released by an interrupted
    /// producer) is transient and retryable, so it must not be reported as a
    /// permanent failure.
    #[cfg(feature = "media-metadata")]
    fn remux_incomplete_response(
        cache: &crate::video_transcode_cache::HlsCache,
        key: &crate::video_transcode_cache::HlsCacheKey,
    ) -> Response<Body> {
        use crate::video_transcode_cache::HlsCacheState;

        match cache.get_state(key) {
            Some(HlsCacheState::Failed(message)) => {
                Self::build_remux_unprocessable_response(&message)
            }
            _ => Self::build_remux_retry_response("generation is not available yet"),
        }
    }

    /// A permanent "this variant cannot be produced" answer.
    #[cfg(feature = "media-metadata")]
    fn build_remux_unprocessable_response(message: &str) -> Response<Body> {
        build_response_or_500(
            Response::builder()
                .status(StatusCode::UNPROCESSABLE_ENTITY)
                .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(Body::from(format!("Cannot serve this variant: {message}"))),
        )
    }

    /// A transient "try again" answer.
    ///
    /// `503` with `Retry-After: 1` rather than a 404, because the content may
    /// well exist on the next attempt and a player should be told to come back
    /// rather than treat the segment as missing.
    #[cfg(feature = "media-metadata")]
    fn build_remux_retry_response(reason: &str) -> Response<Body> {
        build_response_or_500(
            Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .header(header::RETRY_AFTER, "1")
                .body(Body::from(format!("{reason}; please retry"))),
        )
    }

    /// Map a remux failure to an honest status code.
    ///
    /// An empty playlist or a silent 404 would make a player hang or retry
    /// forever, so every failure gets a status and a reason.
    #[cfg(feature = "media-metadata")]
    fn build_remux_error_response(
        error: &crate::video_transcode::TranscodeError,
    ) -> Response<Body> {
        use crate::video_transcode::TranscodeError;

        let status = match error {
            // Nothing to copy, or nothing a player could decode.
            TranscodeError::NoVideoStream { .. }
            | TranscodeError::NoKeyframeIndex
            | TranscodeError::UnsupportedFormat => StatusCode::UNPROCESSABLE_ENTITY,
            // The client asked for a segment that does not exist.
            TranscodeError::SegmentOutOfRange { .. } => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };

        tracing::warn!("remux failed ({status}): {error}");
        build_response_or_500(
            Response::builder()
                .status(status)
                .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(Body::from(error.to_string())),
        )
    }

    /// Try to serve the stream-copy (remux) HLS variant.
    ///
    /// Reachable **without** `--transcode`: a remux is a stream copy costing a
    /// tiny fraction of the 720p/480p re-encode, and it exists to recover a
    /// video the browser has already failed to play, so it does not warrant the
    /// same opt-in. Still server/GUI only — static builds never register this
    /// route.
    ///
    /// Returns `Some(Response)` when the path was a remux URL, `None` to fall
    /// through to the remaining handlers.
    #[cfg(feature = "media-metadata")]
    async fn try_serve_remux_content(path: &str, config: &ServerState) -> Option<Response<Body>> {
        use crate::video_metadata_cache::cache_key_with_mtime;
        use crate::video_remux::{
            RemuxPart, generate_remux_init, generate_remux_playlist, generate_remux_segment,
            parse_remux_request,
        };
        use crate::video_transcode_cache::HlsCacheKey;

        let request = parse_remux_request(path)?;

        // Path-traversal protection, and the source of the mtime the cache key
        // is scoped to.
        let Some(video_file) =
            resolve_media_source_file(request.video_path, &config.base_dir, &config.static_folder)
        else {
            tracing::debug!("no video file for remux request: {}", request.video_path);
            return None;
        };

        let cache_key =
            HlsCacheKey::remux(cache_key_with_mtime(&video_file, "remux"), request.part);

        // Segment URIs in the playlist are relative to the playlist URL, so the
        // base name keeps the video's own extension.
        let base_name = std::path::Path::new(request.video_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("video")
            .to_string();

        let part = request.part;
        let source = video_file.clone();
        let generate = move || match part {
            RemuxPart::Playlist => {
                generate_remux_playlist(&source, &base_name).map(String::into_bytes)
            }
            RemuxPart::Init => generate_remux_init(&source),
            RemuxPart::Segment(index) => generate_remux_segment(&source, index),
        };

        tracing::debug!("remux request for {:?}: {:?}", video_file, part);
        Some(
            match Self::generate_remux_part(&config.hls_cache, cache_key, generate).await {
                Ok(data) => Self::build_remux_response(part, data),
                Err(response) => response,
            },
        )
    }

    /// Probes a video's resolution, memoized by path+mtime.
    ///
    /// The underlying `probe_video_resolution` demuxes the file, which is
    /// blocking work, so on a cache miss it runs on a blocking thread. The
    /// result is cached keyed by the file's mtime, so an unchanged file is never
    /// re-demuxed and an edited file is transparently re-probed (finding #17).
    #[cfg(feature = "media-metadata")]
    async fn probe_resolution_cached(
        video_file: &std::path::Path,
        config: &ServerState,
    ) -> Option<crate::video_transcode::VideoResolution> {
        use crate::video_metadata_cache::cache_key_with_mtime;
        use crate::video_transcode::probe_video_resolution;

        let key = cache_key_with_mtime(video_file, "resolution");
        if let Some(res) = config.video_resolution_cache.pin().get(&key).cloned() {
            return Some(res);
        }

        let path = video_file.to_path_buf();
        let probed = tokio::task::spawn_blocking(move || probe_video_resolution(&path))
            .await
            .inspect_err(|e| tracing::warn!("resolution probe task failed: {e}"))
            .ok()?
            .ok()?;
        config
            .video_resolution_cache
            .pin()
            .insert(key, probed.clone());
        Some(probed)
    }

    /// Try to serve HLS content (playlist or segment) for transcoded video variants.
    ///
    /// Returns Some(Response) if the request was for HLS content and we
    /// successfully served it, None otherwise (fall through to other handlers).
    #[cfg(feature = "media-metadata")]
    async fn try_serve_hls_content(path: &str, config: &ServerState) -> Option<Response<Body>> {
        use crate::video_transcode::{
            HlsRequest, TranscodeError, generate_hls_playlist, parse_hls_request, should_transcode,
            transcode_segment,
        };
        use crate::video_transcode_cache::{
            HLS_WAIT_TIMEOUT, HlsCacheKey, HlsCacheStartResult, HlsCacheState,
        };

        // Helper to build error response for transcode errors
        fn build_transcode_error_response(error: &TranscodeError) -> Option<Response<Body>> {
            match error {
                TranscodeError::SourceTooSmall {
                    source_height,
                    target_height,
                } => Some(build_response_or_500(
                    Response::builder()
                        .status(StatusCode::UNPROCESSABLE_ENTITY)
                        .header(header::CONTENT_TYPE, "text/plain")
                        .body(Body::from(format!(
                            "Cannot transcode: source ({}p) not larger than target ({}p)",
                            source_height, target_height
                        ))),
                )),
                TranscodeError::SegmentOutOfRange {
                    segment_index,
                    video_duration,
                } => Some(build_response_or_500(
                    Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .header(header::CONTENT_TYPE, "text/plain")
                        .body(Body::from(format!(
                            "Segment {} is out of range (video duration: {:.1}s)",
                            segment_index, video_duration
                        ))),
                )),
                // For other errors, fall through to 404 (return None)
                _ => None,
            }
        }

        // Check if this is an HLS request
        let hls_request = parse_hls_request(path)?;

        // Extract the video path and target from the request
        let (video_path, target) = match &hls_request {
            HlsRequest::Playlist { video_path, target } => (video_path.clone(), *target),
            HlsRequest::Segment {
                video_path, target, ..
            } => (video_path.clone(), *target),
        };

        tracing::debug!("HLS request: {:?}", hls_request);

        // Resolve the original video file path with path traversal protection
        let Some(video_file) =
            resolve_media_source_file(&video_path, &config.base_dir, &config.static_folder)
        else {
            tracing::debug!("Original video file not found for HLS: {}", video_path);
            return None;
        };

        // Fast path (finding #17): if the requested playlist/segment is already
        // cached, serve it without probing the source. The blocking ffmpeg
        // demux in `probe_video_resolution` must not run on cache hits.
        let content_key = match &hls_request {
            HlsRequest::Playlist { .. } => HlsCacheKey::playlist(video_file.clone(), target),
            HlsRequest::Segment { segment_index, .. } => {
                HlsCacheKey::segment(video_file.clone(), target, *segment_index)
            }
        };
        if let Some(HlsCacheState::Complete(data)) = config.hls_cache.get_state(&content_key) {
            tracing::debug!("Serving cached HLS content (pre-probe fast path)");
            return Some(match hls_request {
                HlsRequest::Playlist { .. } => Self::build_hls_playlist_response(data),
                HlsRequest::Segment { .. } => Self::build_hls_segment_response(data),
            });
        }

        // Cache miss: probe the source to decide whether transcoding applies.
        // The probe demuxes the file (blocking), so it runs off the async worker
        // and its result is cached by path+mtime so repeated misses for an
        // unchanged file never re-demux (finding #17 + #16).
        let resolution = Self::probe_resolution_cached(&video_file, config).await?;
        if !should_transcode(resolution.height, target) {
            tracing::debug!(
                "Video already at or below target resolution: {}x{} <= {}",
                resolution.width,
                resolution.height,
                target.height()
            );
            // Return 422 instead of None (404) with helpful message
            return Some(build_response_or_500(
                Response::builder()
                    .status(StatusCode::UNPROCESSABLE_ENTITY)
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from(format!(
                        "Cannot transcode: source ({}p) not larger than target ({}p)",
                        resolution.height,
                        target.height()
                    ))),
            ));
        }

        match hls_request {
            HlsRequest::Playlist { .. } => {
                // Generate or serve cached playlist
                let cache_key = HlsCacheKey::playlist(video_file.clone(), target);

                // Extract base name for playlist URLs
                let base_name = std::path::Path::new(&video_path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("video");

                match config.hls_cache.start_generation(cache_key.clone()) {
                    HlsCacheStartResult::Started(notify) => {
                        tracing::debug!("Generating HLS playlist for {:?}", video_file);

                        // Guard the claimed slot: if this future is dropped at
                        // the await below (client disconnect) or the worker
                        // panics, the guard records a failure so the key is not
                        // stuck `InProgress` forever.
                        let mut slot = HlsGenerationSlot::new(
                            &config.hls_cache,
                            cache_key.clone(),
                            Arc::clone(&notify),
                        );

                        let video_file_clone = video_file.clone();
                        let base_name = base_name.to_string();
                        let result = tokio::task::spawn_blocking(move || {
                            generate_hls_playlist(&video_file_clone, target, &base_name)
                        })
                        .await;

                        match result {
                            Ok(Ok(playlist)) => {
                                config
                                    .hls_cache
                                    .complete_generation(cache_key.clone(), playlist.into_bytes());
                                slot.settled();
                                notify.notify_waiters();

                                if let Some(HlsCacheState::Complete(data)) =
                                    config.hls_cache.get_state(&cache_key)
                                {
                                    return Some(Self::build_hls_playlist_response(data));
                                }
                            }
                            Ok(Err(e)) => {
                                tracing::warn!("Playlist generation failed: {}", e);
                                config.hls_cache.fail_generation(cache_key, &e);
                                slot.settled();
                                notify.notify_waiters();
                                // Return meaningful error response for known error types
                                if let Some(response) = build_transcode_error_response(&e) {
                                    return Some(response);
                                }
                                return None;
                            }
                            Err(e) => {
                                // `slot` releases the wedged entry on the way out.
                                tracing::warn!("Playlist generation task panicked: {}", e);
                                return None;
                            }
                        }
                    }
                    HlsCacheStartResult::AlreadyInProgress(notify) => {
                        tracing::debug!("Waiting for in-progress playlist generation");
                        match config
                            .hls_cache
                            .wait_for_completion(&cache_key, notify, HLS_WAIT_TIMEOUT)
                            .await
                        {
                            Some(data) => return Some(Self::build_hls_playlist_response(data)),
                            None => return None,
                        }
                    }
                    HlsCacheStartResult::AlreadyComplete(data) => {
                        tracing::debug!("Serving cached playlist");
                        return Some(Self::build_hls_playlist_response(data));
                    }
                    HlsCacheStartResult::PreviouslyFailed(msg) => {
                        tracing::debug!("Previous playlist generation failed: {}", msg);
                        // Return 422 with cached error message instead of None (404)
                        return Some(build_response_or_500(
                            Response::builder()
                                .status(StatusCode::UNPROCESSABLE_ENTITY)
                                .header(header::CONTENT_TYPE, "text/plain")
                                .body(Body::from(format!("Transcode failed: {}", msg))),
                        ));
                    }
                    HlsCacheStartResult::CacheDisabled => {
                        // Generate without caching
                        let video_file_clone = video_file.clone();
                        let base_name = base_name.to_string();
                        let result = tokio::task::spawn_blocking(move || {
                            generate_hls_playlist(&video_file_clone, target, &base_name)
                        })
                        .await;

                        match result {
                            Ok(Ok(playlist)) => {
                                return Some(Self::build_hls_playlist_response(Arc::new(
                                    playlist.into_bytes(),
                                )));
                            }
                            Ok(Err(e)) => {
                                tracing::warn!("Playlist generation failed: {}", e);
                                // Return meaningful error response for known error types
                                if let Some(response) = build_transcode_error_response(&e) {
                                    return Some(response);
                                }
                                return None;
                            }
                            Err(e) => {
                                tracing::warn!("Playlist generation task panicked: {}", e);
                                return None;
                            }
                        }
                    }
                }
            }
            HlsRequest::Segment { segment_index, .. } => {
                // Generate or serve cached segment
                let cache_key = HlsCacheKey::segment(video_file.clone(), target, segment_index);

                match config.hls_cache.start_generation(cache_key.clone()) {
                    HlsCacheStartResult::Started(notify) => {
                        tracing::info!(
                            "Transcoding segment {} for {:?} @ {:?}",
                            segment_index,
                            video_file,
                            target
                        );

                        // See the playlist arm: the guard frees the claimed
                        // slot if this future is cancelled mid-transcode or the
                        // worker panics.
                        let mut slot = HlsGenerationSlot::new(
                            &config.hls_cache,
                            cache_key.clone(),
                            Arc::clone(&notify),
                        );

                        let video_file_clone = video_file.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            transcode_segment(&video_file_clone, target, segment_index)
                        })
                        .await;

                        match result {
                            Ok(Ok(data)) => {
                                config
                                    .hls_cache
                                    .complete_generation(cache_key.clone(), data);
                                slot.settled();
                                notify.notify_waiters();

                                if let Some(HlsCacheState::Complete(data)) =
                                    config.hls_cache.get_state(&cache_key)
                                {
                                    return Some(Self::build_hls_segment_response(data));
                                }
                            }
                            Ok(Err(e)) => {
                                tracing::warn!("Segment transcode failed: {}", e);
                                config.hls_cache.fail_generation(cache_key, &e);
                                slot.settled();
                                notify.notify_waiters();
                                // Return meaningful error response for known error types
                                if let Some(response) = build_transcode_error_response(&e) {
                                    return Some(response);
                                }
                                return None;
                            }
                            Err(e) => {
                                // `slot` releases the wedged entry on the way out.
                                tracing::warn!("Segment transcode task panicked: {}", e);
                                return None;
                            }
                        }
                    }
                    HlsCacheStartResult::AlreadyInProgress(notify) => {
                        tracing::debug!("Waiting for in-progress segment transcode");
                        match config
                            .hls_cache
                            .wait_for_completion(&cache_key, notify, HLS_WAIT_TIMEOUT)
                            .await
                        {
                            Some(data) => return Some(Self::build_hls_segment_response(data)),
                            None => return None,
                        }
                    }
                    HlsCacheStartResult::AlreadyComplete(data) => {
                        tracing::debug!("Serving cached segment");
                        return Some(Self::build_hls_segment_response(data));
                    }
                    HlsCacheStartResult::PreviouslyFailed(msg) => {
                        tracing::debug!("Previous segment transcode failed: {}", msg);
                        // Return 422 with cached error message instead of None (404)
                        return Some(build_response_or_500(
                            Response::builder()
                                .status(StatusCode::UNPROCESSABLE_ENTITY)
                                .header(header::CONTENT_TYPE, "text/plain")
                                .body(Body::from(format!("Transcode failed: {}", msg))),
                        ));
                    }
                    HlsCacheStartResult::CacheDisabled => {
                        // Transcode without caching (not recommended for segments)
                        let video_file_clone = video_file.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            transcode_segment(&video_file_clone, target, segment_index)
                        })
                        .await;

                        match result {
                            Ok(Ok(data)) => {
                                return Some(Self::build_hls_segment_response(Arc::new(data)));
                            }
                            Ok(Err(e)) => {
                                tracing::warn!("Segment transcode failed: {}", e);
                                // Return meaningful error response for known error types
                                if let Some(response) = build_transcode_error_response(&e) {
                                    return Some(response);
                                }
                                return None;
                            }
                            Err(e) => {
                                tracing::warn!("Segment transcode task panicked: {}", e);
                                return None;
                            }
                        }
                    }
                }
            }
        }

        None
    }

    async fn markdown_to_html(
        md_path: &Path,
        config: &ServerState,
    ) -> Result<Response<Body>, MbrError> {
        let root_path = config.base_dir.as_path();

        // Determine if this is an index file (which doesn't need ../ prefix for links)
        let is_index_file = md_path
            .file_name()
            .and_then(|f| f.to_str())
            .is_some_and(|f| f == config.index_file);

        let link_transform_config = LinkTransformConfig {
            markdown_extensions: config.markdown_extensions.clone(),
            index_file: config.index_file.clone(),
            is_index_file,
            url_depth: None,
            current_page_url: crate::repo::build_markdown_url_path(
                md_path,
                root_path,
                &config.index_file,
            ),
            // The page a reader is actually looking at: extension-less
            // markdown targets must come out with the trailing slash their
            // canonical URL has, or every relative link on the page they lead
            // to resolves one directory too high.
            markdown_page_probe: Some(crate::link_transform::filesystem_markdown_page_probe(
                owned_resolver_config(config),
            )),
        };

        // Transcoding is only available with media-metadata feature
        #[cfg(feature = "media-metadata")]
        let transcode_enabled = config.transcode_enabled;
        #[cfg(not(feature = "media-metadata"))]
        let transcode_enabled = false;

        let valid_tag_sources = crate::config::tag_sources_to_set(&config.tag_sources);
        let render_result = markdown::render_with_cache(
            md_path.to_path_buf(),
            root_path,
            config.oembed_timeout_ms,
            link_transform_config,
            Some(config.oembed_cache.clone()),
            true, // server_mode is always true in server
            transcode_enabled,
            valid_tag_sources,
            config.mark_incomplete,
            &config.incomplete_markers,
            Some(config.repo.wikilink_index.clone()),
        )
        .await
        .inspect_err(|e| tracing::error!("Error rendering markdown: {e}"))?;
        let mut frontmatter = render_result.frontmatter;
        let headings = render_result.headings;
        let inner_html_output = render_result.html;
        let outbound_links = render_result.outbound_links;
        let has_h1 = render_result.has_h1;
        let word_count = render_result.word_count;
        let readability_counts = crate::readability::ReadabilityCounts {
            words: render_result.word_count,
            sentences: render_result.sentence_count,
            syllables: render_result.syllable_count,
        };
        let readability_scores = crate::readability::scores(&readability_counts);
        // Use relative path for markdown_source so live reload can match it.
        // `path_to_url` keeps it `/`-separated: the editor splits this value on
        // `/` to build the raw/save URLs and to derive the note's folder for
        // image uploads, so a Windows `docs\guide.md` would collapse to a single
        // segment and drop uploads into the repo root.
        let relative_md_path =
            pathdiff::diff_paths(md_path, root_path).unwrap_or_else(|| md_path.to_path_buf());
        frontmatter.insert(
            "markdown_source".into(),
            crate::url_path::path_to_url(&relative_md_path).into(),
        );
        // Indicate server mode for frontend search functionality
        frontmatter.insert("server_mode".into(), "true".into());
        // Indicate GUI mode for native window detection
        frontmatter.insert(
            "gui_mode".into(),
            if config.gui_mode { "true" } else { "" }.into(),
        );
        // Indicate whether in-browser editing is enabled (drives the edit button)
        frontmatter.insert(
            "edit_enabled".into(),
            if config.edit_enabled { "true" } else { "" }.into(),
        );

        // Compute breadcrumbs based on the URL path, not the file path
        // For a file like docs/guide.md, the URL is /docs/guide/ so breadcrumbs should include docs
        let url_path_buf = if is_index_file {
            // index.md -> use parent directory path
            // e.g., docs/index.md -> /docs/ -> breadcrumbs path is "docs"
            relative_md_path
                .parent()
                .unwrap_or(Path::new(""))
                .to_path_buf()
        } else {
            // regular.md -> use parent + file stem
            // e.g., docs/guide.md -> /docs/guide/ -> breadcrumbs path is "docs/guide"
            let parent = relative_md_path.parent().unwrap_or(Path::new(""));
            let stem = relative_md_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            parent.join(stem)
        };

        // This page's canonical URL. Built with `path_to_url` rather than
        // `display()`: on Windows the latter yields `/docs\guide/`, which never
        // matches the `/docs/guide/` key the links.json request handler looks
        // up, making the link cache a permanent miss. (The `replace` collapses
        // the `//` produced when the path is empty, i.e. the root index.)
        let current_url =
            format!("/{}/", crate::url_path::path_to_url(&url_path_buf)).replace("//", "/");

        // Cache outbound links for links.json endpoint if link tracking is
        // enabled. An empty list is cached too: skipping the insert would leave
        // a page that just lost its last link serving its stale pre-edit entry
        // (`LinkCache` has no TTL and nothing else overwrites it).
        if config.link_tracking {
            // Resolve relative URLs to absolute before caching
            let resolved_links =
                resolve_outbound_links(&current_url, outbound_links, is_index_file);
            config
                .link_cache
                .insert(current_url.clone(), resolved_links);
        }

        // Get modified date from file metadata (blocking fs work stays async here)
        let modified_secs = tokio::fs::metadata(md_path)
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());

        // Compute prev/next sibling pages for navigation (reuses `current_url`
        // computed above).
        let parent_dir = relative_md_path.parent().unwrap_or(Path::new(""));

        // Get sibling markdown files in the same directory. The sorted list is
        // memoized per parent directory so we avoid an O(repo) scan on every
        // render; the cache is cleared whenever files change.
        let siblings: Arc<Vec<serde_json::Value>> = cached_dir_files(config, parent_dir);

        // Build the extra context (navigation, TOC, readability, chrome) via
        // the shared builder; server mode uses absolute URLs.
        let extra_context = page_context::markdown_extra_context(
            &page_context::MarkdownPageParams {
                breadcrumb_path: &url_path_buf,
                headings: &headings,
                has_h1,
                word_count,
                readability: &readability_scores,
                file_path: &relative_md_path.to_string_lossy(),
                modified_secs,
                current_url: &current_url,
                siblings: &siblings,
            },
            &page_context::MarkdownContextOptions {
                tag_sources: &config.tag_sources,
                sidebar_style: &config.sidebar_style,
                sidebar_max_items: config.sidebar_max_items,
                graph_depth: config.graph_depth,
                tasks_enabled: config.tasks_enabled,
                title_prefix: &config.title_prefix,
                title_suffix: &config.title_suffix,
            },
            &page_context::UrlMode::Absolute,
        );

        let full_html_output = config
            .templates
            .render_markdown(&inner_html_output, frontmatter, extra_context)
            .inspect_err(|e| tracing::error!("Error rendering template: {e}"))?;
        tracing::debug!("generated the html");

        // Generate ETag from rendered content
        let etag = generate_etag(full_html_output.as_bytes());

        // Get Last-Modified from markdown file
        let last_modified = tokio::fs::metadata(md_path)
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| generate_last_modified(d.as_secs()));

        let mut builder = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_CACHE)
            .header(header::ETAG, etag);

        if let Some(lm) = last_modified {
            builder = builder.header(header::LAST_MODIFIED, lm);
        }

        builder
            .body(Body::from(full_html_output))
            .map_err(MbrError::from)
    }

    /// Scans `dir_path` from disk to produce a directory listing's files and
    /// subdirectories.
    ///
    /// Only used before the background scan finishes — see
    /// [`Self::directory_to_html`]. Scanning is blocking filesystem work (and
    /// re-parses the YAML frontmatter of every file in the directory), so it
    /// runs on a blocking thread. All captured data is owned/`Send`.
    async fn scan_directory_children(
        dir_path: &Path,
        root_path: &Path,
        relative_path: &Path,
        config: &ServerState,
    ) -> Result<(Vec<serde_json::Value>, Vec<serde_json::Value>), MbrError> {
        use serde_json::json;

        let root_path = root_path.to_path_buf();
        let dir_path = dir_path.to_path_buf();
        let relative_path = relative_path.to_path_buf();
        let static_folder = config.static_folder.clone();
        let markdown_extensions = config.markdown_extensions.clone();
        let ignore_dirs = config.ignore_dirs.clone();
        let ignore_globs = config.ignore_globs.clone();
        let index_file = config.index_file.clone();
        let tag_sources = config.tag_sources.clone();
        let relationship_types = config.relationship_types.clone();
        let sort = config.sort.clone();

        let scan_result = tokio::task::spawn_blocking(move || {
            // Create a temporary repo instance to scan this directory
            let temp_repo = Repo::init(
                &root_path,
                &static_folder,
                &markdown_extensions,
                &ignore_dirs,
                &ignore_globs,
                &index_file,
                &tag_sources,
                &relationship_types,
            );

            // Scan this directory only (non-recursive)
            temp_repo.scan_folder(&relative_path).inspect_err(|e| {
                tracing::error!("Error scanning directory: {e}");
            })?;

            // Extract markdown files and transform to JSON using helper
            let mut files: Vec<serde_json::Value> = temp_repo
                .markdown_files
                .pin()
                .iter()
                .map(|(_, file_info)| markdown_file_to_json(file_info))
                .collect();

            // Sort files using configurable sort order
            sort_files(&mut files, &sort);

            // Extract subdirectories
            let subdirs: Vec<serde_json::Value> = temp_repo
                .queued_folders
                .pin()
                .iter()
                .filter_map(|(abs_path, rel_path)| {
                    // Only include immediate children
                    let parent = abs_path.parent()?;
                    if parent == dir_path.as_path() {
                        let name = abs_path.file_name()?.to_str()?.to_string();
                        let mut url_path = crate::url_path::path_to_url(rel_path);
                        if !url_path.starts_with('/') {
                            url_path = "/".to_string() + &url_path;
                        }
                        if !url_path.ends_with('/') {
                            url_path.push('/');
                        }
                        Some(json!({
                            "name": name,
                            "url_path": url_path,
                        }))
                    } else {
                        None
                    }
                })
                .collect();

            Ok::<_, crate::errors::RepoError>((files, subdirs))
        })
        .await;

        match scan_result {
            Ok(Ok(pair)) => Ok(pair),
            Ok(Err(e)) => Err(e.into()),
            Err(e) => {
                tracing::error!("directory scan task failed: {e}");
                Err(e.into())
            }
        }
    }

    async fn directory_to_html(
        dir_path: &Path,
        templates: &crate::templates::Templates,
        root_path: &Path,
        config: &ServerState,
    ) -> Result<Response<Body>, MbrError> {
        use serde_json::json;

        // Calculate relative path from root
        let relative_path = pathdiff::diff_paths(dir_path, root_path)
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        // Serve the listing from the in-memory repository index, memoized per
        // directory. The previous implementation built a throwaway `Repo` and
        // re-parsed the YAML frontmatter of every file in the directory on
        // *every* request — data already resident in `repo.markdown_files`.
        // While the initial scan is still running that index is incomplete, so
        // a per-request disk scan remains the fallback.
        // The memoized lists are shared between requests, so this page takes its
        // own copy to hand to the template engine.
        let (files, subdirs) = if config.repo.is_scan_complete() {
            let dir_key = listing_dir_key(&relative_path);
            (
                cached_dir_files(config, &dir_key).as_ref().clone(),
                cached_dir_subdirs(config, &dir_key, root_path)
                    .as_ref()
                    .clone(),
            )
        } else {
            Self::scan_directory_children(dir_path, root_path, &relative_path, config).await?
        };

        // Use helper functions for navigation elements
        let breadcrumbs = generate_breadcrumbs(&relative_path);
        let breadcrumbs_json = page_context::breadcrumbs_to_json(&breadcrumbs, &UrlMode::Absolute);

        let current_dir_name = get_current_dir_name(&relative_path);
        let parent_path = get_parent_path(&relative_path);

        // Build context
        let mut context = std::collections::HashMap::new();
        // `Value::Array`, not `json!`: the lists are already `Value`s, and the
        // macro would deep-clone every entry a second time.
        context.insert("files".to_string(), serde_json::Value::Array(files));
        context.insert("subdirs".to_string(), serde_json::Value::Array(subdirs));
        context.insert("breadcrumbs".to_string(), json!(breadcrumbs_json));
        context.insert("current_dir_name".to_string(), json!(current_dir_name));
        context.insert(
            "current_path".to_string(),
            json!(relative_path.to_string_lossy()),
        );
        if let Some(parent) = parent_path {
            context.insert("parent_path".to_string(), json!(parent));
        }

        // Add full config to template context
        context.insert(
            "config".to_string(),
            json!({
                "static_folder": config.static_folder,
                "markdown_extensions": config.markdown_extensions,
                "index_file": config.index_file,
                "oembed_timeout_ms": config.oembed_timeout_ms,
            }),
        );

        // Pass tag_sources configuration for frontend (consistent with markdown pages)
        context.insert(
            "tag_sources".to_string(),
            json!(page_context::tag_sources_json(&config.tag_sources)),
        );

        // Mode flags, sidebar navigation configuration, and title affixes
        page_context::insert_page_chrome(
            &mut context,
            &PageChrome {
                mode: ModeFlags::Server {
                    gui_mode: Some(config.gui_mode),
                    mbr_base: false,
                },
                sidebar_style: &config.sidebar_style,
                sidebar_max_items: config.sidebar_max_items,
                graph_depth: config.graph_depth,
                tasks_enabled: config.tasks_enabled,
                title_affixes: Some((&config.title_prefix, &config.title_suffix)),
            },
        );

        // Detect if we're at the root directory
        let is_root =
            relative_path.as_os_str().is_empty() || relative_path == std::path::Path::new(".");

        // Add is_home to context for template conditional rendering
        context.insert("is_home".to_string(), json!(is_root));

        let full_html_output = if is_root {
            templates
                .render_home(context)
                .inspect_err(|e| tracing::error!("Error rendering home template: {e}"))?
        } else {
            templates
                .render_section(context)
                .inspect_err(|e| tracing::error!("Error rendering section template: {e}"))?
        };

        tracing::debug!("generated directory listing html");

        // Generate ETag from rendered content
        let etag = generate_etag(full_html_output.as_bytes());

        // Directory listings are dynamic - use no-store to always fetch fresh
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_STORE)
            .header(header::ETAG, etag)
            .body(Body::from(full_html_output))
            .map_err(MbrError::from)
    }

    /// Renders a tag page showing all pages with a specific tag value.
    async fn tag_page_to_html(
        source: &str,
        value: &str,
        config: &ServerState,
    ) -> Result<Response<Body>, MbrError> {
        // Find the TagSource config to get labels (fallback: capitalized source name)
        let (label, label_plural) =
            page_context::tag_labels(&config.tag_sources, source, &capitalize_first(source));

        // Get pages with this tag from the index
        let pages = config.repo.tag_index.get_pages(source, value);

        // Get display value for the tag
        let display_value = config
            .repo
            .tag_index
            .get_tag_display(source, value)
            .unwrap_or_else(|| value.to_string());

        // Build template context
        let mut context = std::collections::HashMap::new();
        page_context::insert_tag_page_keys(
            &mut context,
            source,
            &display_value,
            &label,
            &label_plural,
            &pages,
            &UrlMode::Absolute,
        );
        page_context::insert_page_chrome(
            &mut context,
            &PageChrome {
                mode: ModeFlags::Server {
                    gui_mode: None,
                    mbr_base: true,
                },
                sidebar_style: &config.sidebar_style,
                sidebar_max_items: config.sidebar_max_items,
                graph_depth: config.graph_depth,
                tasks_enabled: config.tasks_enabled,
                title_affixes: Some((&config.title_prefix, &config.title_suffix)),
            },
        );

        let html_output = config.templates.render_tag(context)?;

        let etag = generate_etag(html_output.as_bytes());

        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_STORE)
            .header(header::ETAG, etag)
            .body(Body::from(html_output))
            .map_err(MbrError::from)
    }

    /// Renders a tag source index showing all tags from a source.
    async fn tag_source_index_to_html(
        source: &str,
        config: &ServerState,
    ) -> Result<Response<Body>, MbrError> {
        // Find the TagSource config to get labels (fallback: capitalized source name)
        let (label, label_plural) =
            page_context::tag_labels(&config.tag_sources, source, &capitalize_first(source));

        // Get all tags for this source
        let tags = config.repo.tag_index.get_all_tags(source);

        // Build template context
        let mut context = std::collections::HashMap::new();
        page_context::insert_tag_index_keys(&mut context, source, &label, &label_plural, &tags);
        page_context::insert_page_chrome(
            &mut context,
            &PageChrome {
                mode: ModeFlags::Server {
                    gui_mode: None,
                    mbr_base: true,
                },
                sidebar_style: &config.sidebar_style,
                sidebar_max_items: config.sidebar_max_items,
                graph_depth: config.graph_depth,
                tasks_enabled: config.tasks_enabled,
                title_affixes: Some((&config.title_prefix, &config.title_suffix)),
            },
        );

        let html_output = config.templates.render_tag_index(context)?;

        let etag = generate_etag(html_output.as_bytes());

        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_STORE)
            .header(header::ETAG, etag)
            .body(Body::from(html_output))
            .map_err(MbrError::from)
    }

    /// Handler for the root path "/" - renders the home page using the same
    /// logic as other directories but with the home.html template.
    async fn home_page(State(config): State<ServerState>) -> Result<impl IntoResponse, StatusCode> {
        tracing::debug!("home_page handler");

        let tag_url_sources = crate::config::tag_sources_to_url_sources(&config.tag_sources);
        let resolver_config = PathResolverConfig {
            base_dir: config.base_dir.as_path(),
            canonical_base_dir: config.canonical_base_dir.as_deref(),
            static_folder: &config.static_folder,
            markdown_extensions: &config.markdown_extensions,
            index_file: &config.index_file,
            tag_sources: &tag_url_sources,
        };

        // Resolve empty path (root)
        match resolve_request_path(&resolver_config, "") {
            ResolvedPath::MarkdownFile(md_path) => {
                tracing::debug!("home: rendering index markdown: {:?}", &md_path);
                Self::markdown_to_html(&md_path, &config)
                    .await
                    .map_err(|e| {
                        tracing::error!("Error rendering home markdown: {e}");
                        StatusCode::INTERNAL_SERVER_ERROR
                    })
            }
            ResolvedPath::DirectoryListing(dir_path) => {
                tracing::debug!("home: generating directory listing: {:?}", &dir_path);
                Self::directory_to_html(
                    &dir_path,
                    &config.templates,
                    config.base_dir.as_path(),
                    &config,
                )
                .await
                .map_err(|e| {
                    tracing::error!("Error generating home directory listing: {e}");
                    StatusCode::INTERNAL_SERVER_ERROR
                })
            }
            _ => {
                tracing::debug!("home: unexpected resolution, showing directory listing");
                Self::directory_to_html(
                    &config.base_dir,
                    &config.templates,
                    config.base_dir.as_path(),
                    &config,
                )
                .await
                .map_err(|e| {
                    tracing::error!("Error generating home directory listing: {e}");
                    StatusCode::INTERNAL_SERVER_ERROR
                })
            }
        }
    }
}

// ============================================================================
// Pure helper functions for directory listing (extracted for testability)
// ============================================================================

/// A breadcrumb entry for navigation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Breadcrumb {
    pub name: String,
    pub url: String,
}

impl Breadcrumb {
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
        }
    }
}

/// Generates breadcrumb navigation from a relative path.
///
/// Always starts with "Home" → "/" and includes all path components.
/// The last component is not included in the returned breadcrumbs (it's the current page).
pub fn generate_breadcrumbs(relative_path: &Path) -> Vec<Breadcrumb> {
    let path_components: Vec<_> = relative_path
        .components()
        .filter_map(|c| {
            if let std::path::Component::Normal(s) = c {
                s.to_str()
            } else {
                None
            }
        })
        .collect();

    // For root (no path components), return empty breadcrumbs
    // The current page name will be shown separately, avoiding "Home > Home"
    if path_components.is_empty() {
        return vec![];
    }

    // Start with Home
    let mut breadcrumbs = vec![Breadcrumb::new("Home", "/")];

    // Add all but the last component (last is current page/directory)
    for (idx, _) in path_components
        .iter()
        .enumerate()
        .take(path_components.len().saturating_sub(1))
    {
        // Join the already-extracted `&str` components directly: routing them
        // back through a `PathBuf` would reintroduce the platform separator.
        let url = format!("/{}/", path_components[..=idx].join("/"));
        let name = path_components[idx].to_string();
        breadcrumbs.push(Breadcrumb::new(name, url));
    }

    breadcrumbs
}

/// Decides whether a watcher file event should trigger a template reload.
///
/// `template_folder` must already be canonicalized by the caller. `notify`
/// reports canonical absolute paths, but the configured template folder is
/// stored as written: only the CLI canonicalizes it, while `.mbr/config.toml`
/// and `MBR_TEMPLATE_FOLDER` deserialize verbatim into a plain `PathBuf`. A
/// relative value (`mytemplates`) or one reached through a symlink therefore
/// never matches a canonical event path, which silently disables template hot
/// reload. Canonicalization belongs in the caller so this stays pure and off
/// the per-event hot path — the folder cannot change during the process.
///
/// Comparison is component-wise via [`Path::starts_with`], not a string prefix
/// test: `"/x/tmpl-backup/index.html"` textually starts with `"/x/tmpl"` but is
/// a sibling directory, not a template.
fn should_reload_template(event_path: &str, template_folder: Option<&Path>) -> bool {
    match template_folder {
        Some(tf) => Path::new(event_path).starts_with(tf),
        // Match `.mbr` as a path component rather than substring
        // matching "/.mbr/". The watcher reports native separators,
        // so on Windows this string is `...\.mbr\theme.css` and the
        // slash form would never match, silently disabling template
        // hot reload.
        None => Path::new(event_path)
            .components()
            .any(|c| c.as_os_str() == ".mbr"),
    }
}

/// Gets the current directory name from a relative path.
pub fn get_current_dir_name(relative_path: &Path) -> String {
    relative_path
        .file_name()
        .and_then(|s| s.to_str())
        .map(String::from)
        .unwrap_or_else(|| "Home".to_string())
}

/// Gets the parent path URL for "up" navigation.
pub fn get_parent_path(relative_path: &Path) -> Option<String> {
    let path_components: Vec<_> = relative_path
        .components()
        .filter_map(|c| {
            if let std::path::Component::Normal(s) = c {
                s.to_str()
            } else {
                None
            }
        })
        .collect();

    if path_components.len() > 1 {
        // Join the `&str` components directly rather than via `PathBuf`, which
        // would use `\` on Windows.
        let parent = path_components[..path_components.len() - 1].join("/");
        Some(format!("/{}/", parent))
    } else if !path_components.is_empty() {
        Some("/".to_string())
    } else {
        None
    }
}

/// Builds the sorted list of sibling markdown files that share `parent_dir`.
///
/// This is the pure core of prev/next navigation: it filters the provided
/// markdown-file infos to those whose parent directory equals `parent_dir`,
/// converts each to its template JSON form, and sorts using `sort`. Extracted
/// so it can be memoized per directory (see `sibling_nav_cache`) and unit
/// tested independently of the live repository.
fn compute_sibling_files<'a>(
    files: impl Iterator<Item = &'a MarkdownInfo>,
    parent_dir: &Path,
    sort: &[SortField],
) -> Vec<serde_json::Value> {
    let mut siblings: Vec<serde_json::Value> = files
        .filter_map(|info| {
            let file_parent = info.raw_path.parent()?;
            (file_parent == parent_dir).then(|| markdown_file_to_json(info))
        })
        .collect();
    sort_files(&mut siblings, sort);
    siblings
}

/// Normalizes a repo-relative directory path into the key used by the listing
/// caches and by `MarkdownInfo::raw_path.parent()`.
///
/// `pathdiff` yields an empty path for the repo root, but the callers that go
/// through `home_page`'s fallback branch can produce `.`; both must map to the
/// same key, and it must be the empty path because that is what
/// `Path::new("root.md").parent()` returns.
fn listing_dir_key(relative_dir: &Path) -> PathBuf {
    if relative_dir == Path::new(".") {
        PathBuf::new()
    } else {
        relative_dir.to_path_buf()
    }
}

/// Returns the immediate subdirectory name of `dir` on the way to `file_path`,
/// or `None` if `file_path` is not below a subdirectory of `dir`.
///
/// Both paths are repo-relative. A direct child *file* of `dir` yields `None`
/// (its first remaining component is the file itself, not a directory).
fn immediate_subdir_name<'a>(file_path: &'a Path, dir: &Path) -> Option<&'a std::ffi::OsStr> {
    let rest = if dir.as_os_str().is_empty() {
        file_path
    } else {
        file_path.strip_prefix(dir).ok()?
    };
    let mut components = rest.components();
    let first = components.next()?;
    // There must be at least one more component (the file name), otherwise
    // `first` is the file, not a directory.
    components.next()?;
    match first {
        std::path::Component::Normal(name) => Some(name),
        _ => None,
    }
}

/// Builds the deduplicated, name-sorted list of immediate subdirectories of
/// `dir` from the repo-relative paths of every indexed file.
///
/// Derived from the file index rather than from a disk walk so it can be
/// memoized and refreshed by the same invalidation that refreshes the file
/// lists. Like the static builder's section pages
/// (`build::build_dir_children_index`), a directory that contains no files at
/// any depth is not listed.
fn compute_subdir_entries<'a>(
    file_paths: impl Iterator<Item = &'a Path>,
    dir: &Path,
) -> Vec<serde_json::Value> {
    file_paths
        .filter_map(|path| immediate_subdir_name(path, dir))
        .map(|name| name.to_string_lossy().into_owned())
        .collect::<std::collections::BTreeSet<String>>()
        .into_iter()
        .map(|name| {
            let url_path = format!("/{}/", crate::url_path::path_to_url(&dir.join(&name)));
            serde_json::json!({
                "name": name,
                "url_path": url_path,
            })
        })
        .collect()
}

/// Returns the sorted markdown-file list for `dir`, memoized per directory.
///
/// Shared by prev/next sibling navigation and by directory listings: both need
/// exactly "the files whose parent directory is `dir`, in sort order". Only
/// memoized once the initial scan is complete, so a partially populated result
/// is never frozen into the cache.
fn cached_dir_files(config: &ServerState, dir: &Path) -> Arc<Vec<serde_json::Value>> {
    if let Some(cached) = config.sibling_nav_cache.pin().get(dir).cloned() {
        return cached;
    }
    let computed = Arc::new(compute_sibling_files(
        config
            .repo
            .markdown_files
            .pin()
            .iter()
            .map(|(_, info)| info),
        dir,
        &config.sort,
    ));
    if config.repo.is_scan_complete() {
        config
            .sibling_nav_cache
            .pin()
            .insert(dir.to_path_buf(), Arc::clone(&computed));
    }
    computed
}

/// Returns the immediate subdirectories of `dir`, memoized per directory.
///
/// Non-markdown files are keyed by absolute path in the repo index, so they
/// are relativized against `root_path` here; anything that does not sit under
/// the root is skipped rather than emitting a `../` entry.
fn cached_dir_subdirs(
    config: &ServerState,
    dir: &Path,
    root_path: &Path,
) -> Arc<Vec<serde_json::Value>> {
    if let Some(cached) = config.subdir_cache.pin().get(dir).cloned() {
        return cached;
    }
    let markdown_guard = config.repo.markdown_files.pin();
    let other_guard = config.repo.other_files.pin();
    let computed = Arc::new(compute_subdir_entries(
        markdown_guard
            .iter()
            .map(|(_, info)| info.raw_path.as_path())
            .chain(
                other_guard
                    .iter()
                    .filter_map(|(abs_path, _)| abs_path.strip_prefix(root_path).ok()),
            ),
        dir,
    ));
    if config.repo.is_scan_complete() {
        config
            .subdir_cache
            .pin()
            .insert(dir.to_path_buf(), Arc::clone(&computed));
    }
    computed
}

/// Transforms markdown file info into a JSON value for template rendering.
pub fn markdown_file_to_json(file_info: &MarkdownInfo) -> serde_json::Value {
    use serde_json::json;

    let title = file_info
        .frontmatter
        .as_ref()
        .and_then(|fm| fm.get("title"))
        .cloned()
        .unwrap_or_else(|| {
            serde_json::Value::String(
                file_info
                    .raw_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Untitled")
                    .to_string(),
            )
        });

    let description = file_info
        .frontmatter
        .as_ref()
        .and_then(|fm| fm.get("description"))
        .cloned();

    let tags = file_info
        .frontmatter
        .as_ref()
        .and_then(|fm| fm.get("tags"))
        .cloned();

    let note_type = file_info
        .frontmatter
        .as_ref()
        .and_then(|fm| fm.get("type"))
        .cloned();

    let modified_date = chrono::DateTime::from_timestamp(file_info.modified as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    json!({
        "title": title,
        "url_path": file_info.url_path,
        "description": description,
        "tags": tags,
        "type": note_type,
        "modified_date": modified_date,
        "modified": file_info.modified,
        "name": file_info.raw_path.file_name().and_then(|s| s.to_str()).unwrap_or(""),
    })
}

// ============================================================================
// Cache header helpers (extracted for testability and reuse)
// ============================================================================

/// Capitalizes the first character of a string, leaving the remainder intact.
///
/// UTF-8 safe: uses character boundaries rather than byte indexing, so a
/// leading multi-byte character (e.g. "étiquettes") does not panic.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Finalizes an HTTP response builder, falling back to a plain 500 response
/// if the builder was misconfigured (e.g. an invalid header value).
///
/// Our builders use fixed, known-valid headers, so the fallback is effectively
/// unreachable — but a malformed value must degrade to a 500, never panic the
/// request handler.
fn build_response_or_500(result: Result<Response<Body>, axum::http::Error>) -> Response<Body> {
    result.unwrap_or_else(|e| {
        tracing::error!("Failed to build HTTP response: {e}");
        let mut response = Response::new(Body::from("Internal Server Error"));
        *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        response
    })
}

/// Builds the `301 Moved Permanently` that sends a non-canonical page URL to
/// its canonical, trailing-slash form.
///
/// The query string is carried over verbatim, because it is part of the
/// request the client made and dropping it silently changes what the target
/// page renders. The **fragment** is deliberately *not* carried: fragments
/// never leave the browser, and RFC 9110 §10.2.2 says a client applies the
/// original request's fragment to a `Location` that has none — so emitting no
/// fragment is exactly how `#section` survives the hop.
fn canonical_redirect_response(canonical_url: &str, query: Option<&str>) -> Response<Body> {
    let location = match query {
        Some(query) if !query.is_empty() => format!("{canonical_url}?{query}"),
        _ => canonical_url.to_string(),
    };
    tracing::debug!("redirecting to canonical URL: {}", &location);
    build_response_or_500(
        Response::builder()
            .status(StatusCode::MOVED_PERMANENTLY)
            .header(header::LOCATION, &location)
            .body(Body::empty()),
    )
}

/// Generates a weak ETag from content bytes using a simple hash.
/// Weak ETags (W/"...") indicate semantic equivalence, not byte-for-byte identity.
fn generate_etag(content: &[u8]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    let hash = hasher.finish();
    format!("W/\"{:x}\"", hash)
}

/// Generates a Last-Modified header value from a Unix timestamp.
fn generate_last_modified(timestamp: u64) -> Option<String> {
    chrono::DateTime::from_timestamp(timestamp as i64, 0)
        .map(|dt| dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string())
}

/// Standard cache control header value for development mode.
/// `no-cache` allows the browser to cache but requires revalidation on every request.
const CACHE_CONTROL_NO_CACHE: &str = "no-cache";

/// Standard cache control header for truly dynamic content that shouldn't be cached.
const CACHE_CONTROL_NO_STORE: &str = "no-store";

/// [`DEFAULT_FILES`] route of the lazy task-browser chunk.
///
/// Named because `build.rs` has to skip exactly this entry: the task browser is
/// server/GUI only (the index is built from live files), so shipping the chunk
/// into a static site would be dead weight behind a button that cannot exist.
pub const TASKS_CHUNK_ROUTE: &str = "/components/mbr-tasks.min.js";

pub const DEFAULT_FILES: &[(&str, &[u8], &str)] = &[
    (
        "/favicon.png",
        include_bytes!("../templates/favicon.png"),
        "image/png",
    ),
    (
        "/theme.css",
        include_bytes!("../templates/theme.css"),
        "text/css",
    ),
    (
        "/user.css",
        include_bytes!("../templates/user.css"),
        "text/css",
    ),
    (
        "/pico.min.css",
        include_bytes!("../templates/pico-main/pico.min.css"),
        "text/css",
    ),
    (
        "/components/mbr-components.min.js",
        include_bytes!("../templates/components-js/mbr-components.min.js"),
        "application/javascript",
    ),
    (
        // Heavy Milkdown/Crepe editor chunk, lazy-loaded by <mbr-editor>.
        "/components/mbr-editor.min.js",
        include_bytes!("../templates/components-js/mbr-editor.min.js"),
        "application/javascript",
    ),
    (
        // Sidebar mini force-graph chunk (d3-force), lazy-loaded by <mbr-info>
        // when the info panel first opens.
        "/components/mbr-graph.min.js",
        include_bytes!("../templates/components-js/mbr-graph.min.js"),
        "application/javascript",
    ),
    (
        // Genealogy chart chunk (family-chart + timeline tree), lazy-loaded by
        // <mbr-genealogy> on `type: person` pages only.
        "/components/mbr-genealogy.min.js",
        include_bytes!("../templates/components-js/mbr-genealogy.min.js"),
        "application/javascript",
    ),
    (
        // Task-browser panel chunk, lazy-loaded by <mbr-tasks> the first time
        // the panel is opened. Deliberately excluded from static builds — see
        // `TASKS_CHUNK_ROUTE` in build.rs.
        TASKS_CHUNK_ROUTE,
        include_bytes!("../templates/components-js/mbr-tasks.min.js"),
        "application/javascript",
    ),
    (
        "/hljs.dark.css",
        include_bytes!("../templates/hljs.dark.11.11.2.css"),
        "text/css",
    ),
    (
        "/hljs.atom-one-dark.css",
        include_bytes!("../templates/hljs.atom-one-dark.11.11.2.css"),
        "text/css",
    ),
    (
        "/hljs.js",
        include_bytes!("../templates/hljs.11.11.2.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.css.js",
        include_bytes!("../templates/hljs.lang.css.11.11.2.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.javascript.js",
        include_bytes!("../templates/hljs.lang.javascript.11.11.2.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.typescript.js",
        include_bytes!("../templates/hljs.lang.typescript.11.11.2.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.rust.js",
        include_bytes!("../templates/hljs.lang.rust.11.11.2.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.python.js",
        include_bytes!("../templates/hljs.lang.python.11.11.2.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.bash.js",
        include_bytes!("../templates/hljs.lang.bash.11.11.2.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.java.js",
        include_bytes!("../templates/hljs.lang.java.11.11.2.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.scala.js",
        include_bytes!("../templates/hljs.lang.scala.11.11.2.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.go.js",
        include_bytes!("../templates/hljs.lang.go.11.11.2.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.ruby.js",
        include_bytes!("../templates/hljs.lang.ruby.11.11.2.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.nix.js",
        include_bytes!("../templates/hljs.lang.nix.11.11.2.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.json.js",
        include_bytes!("../templates/hljs.lang.json.11.11.2.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.yaml.js",
        include_bytes!("../templates/hljs.lang.yaml.11.11.2.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.xml.js",
        include_bytes!("../templates/hljs.lang.xml.11.11.2.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.sql.js",
        include_bytes!("../templates/hljs.lang.sql.11.11.2.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.dockerfile.js",
        include_bytes!("../templates/hljs.lang.dockerfile.11.11.2.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.markdown.js",
        include_bytes!("../templates/hljs.lang.markdown.11.11.2.js"),
        "application/javascript",
    ),
    (
        "/mermaid.min.js",
        include_bytes!("../templates/mermaid.11.16.1.min.js"),
        "application/javascript",
    ),
    // Reveal.js presentation framework
    (
        "/reveal.js",
        include_bytes!("../templates/reveal.5.2.1.js"),
        "application/javascript",
    ),
    (
        "/reveal.css",
        include_bytes!("../templates/reveal.5.2.1.css"),
        "text/css",
    ),
    (
        "/reveal-theme-blank.css",
        include_bytes!("../templates/reveal.theme.blank.5.2.1.css"),
        "text/css",
    ),
    (
        "/reveal-theme-black.css",
        include_bytes!("../templates/reveal.theme.black.5.2.1.css"),
        "text/css",
    ),
    (
        "/reveal-theme-white.css",
        include_bytes!("../templates/reveal.theme.white.5.2.1.css"),
        "text/css",
    ),
    (
        "/reveal-slides.css",
        include_bytes!("../templates/reveal-slides.css"),
        "text/css",
    ),
    (
        "/reveal-notes.js",
        include_bytes!("../templates/reveal.notes.5.2.1.js"),
        "application/javascript",
    ),
];

// ============================================================================
// Tag page link helpers
// ============================================================================

/// Builds outbound links for a tag page (e.g., /tags/rust/).
///
/// Returns links to all pages tagged with this tag, plus a link back to the tag source index.
fn build_tag_page_outbound_links(
    source: &str,
    value: &str,
    tag_index: &crate::tag_index::TagIndex,
    tag_sources: &[TagSource],
) -> Vec<crate::link_index::OutboundLink> {
    use crate::link_index::OutboundLink;

    let mut outbound = Vec::new();

    // Add links to all tagged pages
    for page in tag_index.get_pages(source, value) {
        outbound.push(OutboundLink {
            to: page.url_path,
            text: page.title,
            anchor: None,
            internal: true,
        });
    }

    // Add link back to tag source index
    let label = tag_sources
        .iter()
        .find(|ts| ts.url_source() == source)
        .map(|ts| ts.plural_label())
        .unwrap_or_else(|| source.to_string());

    outbound.push(OutboundLink {
        to: format!("/{}/", source),
        text: label,
        anchor: None,
        internal: true,
    });

    outbound
}

/// Builds outbound links for a tag source index page (e.g., /tags/).
///
/// Returns links to all individual tag pages under this source.
fn build_tag_index_outbound_links(
    source: &str,
    tag_index: &crate::tag_index::TagIndex,
) -> Vec<crate::link_index::OutboundLink> {
    use crate::link_index::OutboundLink;

    tag_index
        .get_all_tags(source)
        .into_iter()
        .map(|tag| OutboundLink {
            to: format!("/{}/{}/", source, tag.normalized),
            text: tag.display,
            anchor: None,
            internal: true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_capitalize_first_ascii() {
        assert_eq!(capitalize_first("tags"), "Tags");
        assert_eq!(capitalize_first("t"), "T");
    }

    #[test]
    fn test_capitalize_first_multibyte() {
        // A leading multi-byte character must not panic (regression for
        // byte-slicing `&source[1..]`).
        assert_eq!(capitalize_first("étiquettes"), "Étiquettes");
        assert_eq!(capitalize_first("über"), "Über");
        // Non-Latin scripts have no uppercase form; content is preserved.
        assert_eq!(capitalize_first("日本語"), "日本語");
    }

    #[test]
    fn test_capitalize_first_empty() {
        assert_eq!(capitalize_first(""), "");
    }

    #[test]
    fn test_generate_breadcrumbs_root() {
        let path = Path::new("");
        let breadcrumbs = generate_breadcrumbs(path);

        // Root returns empty breadcrumbs to avoid "Home > Home" duplication
        // The template handles showing just "Home" as the current page
        assert_eq!(breadcrumbs.len(), 0);
    }

    #[test]
    fn test_generate_breadcrumbs_single_level() {
        let path = Path::new("docs");
        let breadcrumbs = generate_breadcrumbs(path);

        // Home only - "docs" is the current directory, not shown in breadcrumbs
        assert_eq!(breadcrumbs.len(), 1);
        assert_eq!(breadcrumbs[0], Breadcrumb::new("Home", "/"));
    }

    #[test]
    fn test_generate_breadcrumbs_two_levels() {
        let path = Path::new("docs/api");
        let breadcrumbs = generate_breadcrumbs(path);

        assert_eq!(breadcrumbs.len(), 2);
        assert_eq!(breadcrumbs[0], Breadcrumb::new("Home", "/"));
        assert_eq!(breadcrumbs[1], Breadcrumb::new("docs", "/docs/"));
    }

    #[test]
    fn test_generate_breadcrumbs_deep_nesting() {
        let path = Path::new("/a/b/c/d");
        let breadcrumbs = generate_breadcrumbs(path);

        assert_eq!(breadcrumbs.len(), 4);
        assert_eq!(breadcrumbs[0], Breadcrumb::new("Home", "/"));
        assert_eq!(breadcrumbs[1], Breadcrumb::new("a", "/a/"));
        assert_eq!(breadcrumbs[2], Breadcrumb::new("b", "/a/b/"));
        assert_eq!(breadcrumbs[3], Breadcrumb::new("c", "/a/b/c/"));
    }

    #[test]
    fn test_get_current_dir_name_root() {
        let path = Path::new("");
        assert_eq!(get_current_dir_name(path), "Home");
    }

    #[test]
    fn test_get_current_dir_name_single_level() {
        let path = Path::new("docs");
        assert_eq!(get_current_dir_name(path), "docs");
    }

    #[test]
    fn test_get_current_dir_name_nested() {
        let path = Path::new("a/b/c");
        assert_eq!(get_current_dir_name(path), "c");
    }

    #[test]
    fn test_get_parent_path_root() {
        let path = Path::new("");
        assert_eq!(get_parent_path(path), None);
    }

    #[test]
    fn test_get_parent_path_single_level() {
        let path = Path::new("docs");
        assert_eq!(get_parent_path(path), Some("/".to_string()));
    }

    #[test]
    fn test_get_parent_path_two_levels() {
        let path = Path::new("docs/api");
        assert_eq!(get_parent_path(path), Some("/docs/".to_string()));
    }

    #[test]
    fn test_get_parent_path_deep() {
        let path = Path::new("a/b/c/d");
        assert_eq!(get_parent_path(path), Some("/a/b/c/".to_string()));
    }

    #[test]
    fn test_should_reload_template_inside_template_folder() {
        let tf = Path::new("/x/tmpl");
        assert!(should_reload_template("/x/tmpl/index.html", Some(tf)));
        assert!(should_reload_template(
            "/x/tmpl/partials/_nav.html",
            Some(tf)
        ));
    }

    #[test]
    fn test_should_reload_template_rejects_sibling_prefix() {
        // `str::starts_with` matches this sibling directory; `Path::starts_with`
        // compares whole components and does not.
        assert!(!should_reload_template(
            "/x/tmpl-backup/index.html",
            Some(Path::new("/x/tmpl"))
        ));
    }

    #[test]
    fn test_should_reload_template_outside_template_folder() {
        assert!(!should_reload_template(
            "/other/place/index.html",
            Some(Path::new("/x/tmpl"))
        ));
    }

    #[test]
    fn test_should_reload_template_without_folder_matches_mbr_component() {
        assert!(should_reload_template("/repo/.mbr/index.html", None));
        assert!(!should_reload_template("/repo/docs/index.html", None));
        // Substring-only matches are not path components.
        assert!(!should_reload_template("/repo/.mbrx/index.html", None));
    }

    /// The caller canonicalizes the template folder before handing it to
    /// `should_reload_template`; without that step a symlinked folder never
    /// matches the canonical paths the watcher reports.
    #[cfg(unix)]
    #[test]
    fn test_should_reload_template_matches_through_symlinked_folder() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let real = temp.path().join("real-templates");
        std::fs::create_dir(&real).expect("create real template dir");
        let link = temp.path().join("link-templates");
        std::os::unix::fs::symlink(&real, &link).expect("create symlink");

        // What the watcher reports: a canonical path under the real directory.
        let event_path = real
            .canonicalize()
            .expect("canonicalize real dir")
            .join("index.html");
        let event_path = event_path.to_string_lossy().to_string();

        // Configured as the symlink, used verbatim: no match.
        assert!(!should_reload_template(&event_path, Some(link.as_path())));

        // Canonicalized once by the caller, as `Server::init` does: match.
        let canonical = link.canonicalize().unwrap_or(link);
        assert!(should_reload_template(&event_path, Some(&canonical)));
    }

    #[test]
    fn test_markdown_file_to_json_with_frontmatter() {
        let mut frontmatter = crate::markdown::SimpleMetadata::new();
        frontmatter.insert(
            "title".to_string(),
            serde_json::Value::String("My Title".to_string()),
        );
        frontmatter.insert(
            "description".to_string(),
            serde_json::Value::String("My description".to_string()),
        );
        frontmatter.insert("tags".to_string(), serde_json::json!(["rust", "testing"]));

        let file_info = MarkdownInfo {
            raw_path: PathBuf::from("test.md"),
            url_path: "/test/".to_string(),
            frontmatter: Some(frontmatter),
            created: 1699000000,
            modified: 1700000000,
            relationships: Vec::new(),
        };

        let json = markdown_file_to_json(&file_info);

        assert_eq!(json["title"], "My Title");
        assert_eq!(json["url_path"], "/test/");
        assert_eq!(json["description"], "My description");
        assert_eq!(json["tags"], serde_json::json!(["rust", "testing"]));
        assert_eq!(json["modified"], 1700000000);
        assert_eq!(json["name"], "test.md");
    }

    #[test]
    fn test_markdown_file_to_json_without_frontmatter() {
        let file_info = MarkdownInfo {
            raw_path: PathBuf::from("my-document.md"),
            url_path: "/my-document/".to_string(),
            frontmatter: None,
            created: 1699000000,
            modified: 1700000000,
            relationships: Vec::new(),
        };

        let json = markdown_file_to_json(&file_info);

        // Should use file stem as title when no frontmatter
        assert_eq!(json["title"], "my-document");
        assert_eq!(json["url_path"], "/my-document/");
        assert!(json["description"].is_null());
        assert!(json["tags"].is_null());
    }

    #[test]
    fn test_markdown_file_to_json_partial_frontmatter() {
        let mut frontmatter = crate::markdown::SimpleMetadata::new();
        frontmatter.insert(
            "title".to_string(),
            serde_json::Value::String("Only Title".to_string()),
        );
        // No description or tags

        let file_info = MarkdownInfo {
            raw_path: PathBuf::from("partial.md"),
            url_path: "/partial/".to_string(),
            frontmatter: Some(frontmatter),
            created: 1699000000,
            modified: 1700000000,
            relationships: Vec::new(),
        };

        let json = markdown_file_to_json(&file_info);

        assert_eq!(json["title"], "Only Title");
        assert!(json["description"].is_null());
        assert!(json["tags"].is_null());
    }

    #[test]
    fn test_breadcrumb_equality() {
        let b1 = Breadcrumb::new("Home", "/");
        let b2 = Breadcrumb::new("Home", "/");
        let b3 = Breadcrumb::new("Docs", "/docs/");

        assert_eq!(b1, b2);
        assert_ne!(b1, b3);
    }

    // ==================== MediaViewerType Tests ====================

    #[test]
    fn test_media_viewer_type_from_route_videos() {
        assert_eq!(
            MediaViewerType::from_route("/.mbr/videos/"),
            Some(MediaViewerType::Video)
        );
    }

    #[test]
    fn test_media_viewer_type_from_route_pdfs() {
        assert_eq!(
            MediaViewerType::from_route("/.mbr/pdfs/"),
            Some(MediaViewerType::Pdf)
        );
    }

    #[test]
    fn test_media_viewer_type_from_route_audio() {
        assert_eq!(
            MediaViewerType::from_route("/.mbr/audio/"),
            Some(MediaViewerType::Audio)
        );
    }

    #[test]
    fn test_media_viewer_type_from_route_images() {
        assert_eq!(
            MediaViewerType::from_route("/.mbr/images/"),
            Some(MediaViewerType::Image)
        );
    }

    #[test]
    fn test_media_viewer_type_from_route_invalid() {
        assert_eq!(MediaViewerType::from_route("/some/other/path"), None);
        assert_eq!(MediaViewerType::from_route("/.mbr/videos"), None); // missing trailing slash
        assert_eq!(MediaViewerType::from_route("/.mbr/unknown/"), None);
    }

    #[test]
    fn test_media_viewer_type_template_name() {
        assert_eq!(MediaViewerType::Video.template_name(), "media_viewer.html");
        assert_eq!(MediaViewerType::Pdf.template_name(), "media_viewer.html");
        assert_eq!(MediaViewerType::Audio.template_name(), "media_viewer.html");
    }

    #[test]
    fn test_media_viewer_type_label() {
        assert_eq!(MediaViewerType::Video.label(), "Video");
        assert_eq!(MediaViewerType::Pdf.label(), "PDF");
        assert_eq!(MediaViewerType::Audio.label(), "Audio");
    }

    #[test]
    fn test_media_viewer_type_as_str() {
        assert_eq!(MediaViewerType::Video.as_str(), "video");
        assert_eq!(MediaViewerType::Pdf.as_str(), "pdf");
        assert_eq!(MediaViewerType::Audio.as_str(), "audio");
    }

    #[test]
    fn test_media_viewer_type_from_extension_video() {
        for ext in &[
            "mp4", "m4v", "mov", "webm", "flv", "mpg", "mpeg", "avi", "3gp", "wmv", "mkv", "ts",
            "mts", "m2ts", "vob", "divx", "xvid", "asf", "rm", "rmvb", "f4v", "ogv",
        ] {
            assert_eq!(
                MediaViewerType::from_extension(ext),
                Some(MediaViewerType::Video),
                "Expected Video for extension '{ext}'"
            );
        }
    }

    #[test]
    fn test_media_viewer_type_from_extension_audio() {
        for ext in &[
            "mp3", "wav", "ogg", "flac", "aac", "m4a", "aiff", "aif", "oga", "opus", "wma",
        ] {
            assert_eq!(
                MediaViewerType::from_extension(ext),
                Some(MediaViewerType::Audio),
                "Expected Audio for extension '{ext}'"
            );
        }
    }

    #[test]
    fn test_media_viewer_type_from_extension_image() {
        for ext in &[
            "jpg", "jpeg", "png", "webp", "gif", "bmp", "tif", "tiff", "svg",
        ] {
            assert_eq!(
                MediaViewerType::from_extension(ext),
                Some(MediaViewerType::Image),
                "Expected Image for extension '{ext}'"
            );
        }
    }

    #[test]
    fn test_media_viewer_type_from_extension_pdf() {
        assert_eq!(
            MediaViewerType::from_extension("pdf"),
            Some(MediaViewerType::Pdf)
        );
    }

    #[test]
    fn test_media_viewer_type_from_extension_case_insensitive() {
        assert_eq!(
            MediaViewerType::from_extension("MP4"),
            Some(MediaViewerType::Video)
        );
        assert_eq!(
            MediaViewerType::from_extension("Pdf"),
            Some(MediaViewerType::Pdf)
        );
        assert_eq!(
            MediaViewerType::from_extension("JPG"),
            Some(MediaViewerType::Image)
        );
    }

    #[test]
    fn test_media_viewer_type_from_extension_unknown() {
        assert_eq!(MediaViewerType::from_extension("md"), None);
        assert_eq!(MediaViewerType::from_extension("html"), None);
        assert_eq!(MediaViewerType::from_extension("rs"), None);
        assert_eq!(MediaViewerType::from_extension(""), None);
    }

    #[test]
    fn test_media_viewer_type_from_path() {
        assert_eq!(
            MediaViewerType::from_path(Path::new("videos/demo.mp4")),
            Some(MediaViewerType::Video)
        );
        assert_eq!(
            MediaViewerType::from_path(Path::new("music/song.mp3")),
            Some(MediaViewerType::Audio)
        );
        assert_eq!(
            MediaViewerType::from_path(Path::new("images/photo.jpg")),
            Some(MediaViewerType::Image)
        );
        assert_eq!(
            MediaViewerType::from_path(Path::new("docs/paper.pdf")),
            Some(MediaViewerType::Pdf)
        );
        assert_eq!(MediaViewerType::from_path(Path::new("readme.md")), None);
        assert_eq!(MediaViewerType::from_path(Path::new("noext")), None);
    }

    #[test]
    fn test_media_viewer_type_route_path() {
        assert_eq!(MediaViewerType::Video.route_path(), "/.mbr/videos/");
        assert_eq!(MediaViewerType::Pdf.route_path(), "/.mbr/pdfs/");
        assert_eq!(MediaViewerType::Audio.route_path(), "/.mbr/audio/");
        assert_eq!(MediaViewerType::Image.route_path(), "/.mbr/images/");
    }

    #[test]
    fn test_media_viewer_type_route_path_roundtrips_with_from_route() {
        for media_type in &[
            MediaViewerType::Video,
            MediaViewerType::Pdf,
            MediaViewerType::Audio,
            MediaViewerType::Image,
        ] {
            assert_eq!(
                MediaViewerType::from_route(media_type.route_path()),
                Some(*media_type),
                "route_path -> from_route roundtrip failed for {media_type:?}"
            );
        }
    }

    // ==================== validate_media_path Tests ====================

    #[test]
    fn test_validate_media_path_rejects_directory_traversal() {
        let temp_dir = tempfile::tempdir().unwrap();
        let result = validate_media_path("../etc/passwd", temp_dir.path(), "");
        assert!(matches!(result, Err(MbrError::DirectoryTraversal)));
    }

    #[test]
    fn test_validate_media_path_rejects_embedded_directory_traversal() {
        let temp_dir = tempfile::tempdir().unwrap();
        let result = validate_media_path("some/../../etc/passwd", temp_dir.path(), "");
        assert!(matches!(result, Err(MbrError::DirectoryTraversal)));
    }

    #[test]
    fn test_validate_media_path_rejects_url_encoded_traversal() {
        let temp_dir = tempfile::tempdir().unwrap();
        // URL-encoded ".." = "%2e%2e"
        let result = validate_media_path("%2e%2e/etc/passwd", temp_dir.path(), "");
        assert!(matches!(result, Err(MbrError::DirectoryTraversal)));
    }

    #[test]
    fn test_validate_media_path_rejects_nonexistent_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let result = validate_media_path("nonexistent.mp4", temp_dir.path(), "");
        assert!(matches!(result, Err(MbrError::InvalidMediaPath(_))));
    }

    #[test]
    fn test_validate_media_path_accepts_valid_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let test_file = temp_dir.path().join("test.mp4");
        std::fs::write(&test_file, "dummy content").unwrap();

        let result = validate_media_path("test.mp4", temp_dir.path(), "");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), test_file.canonicalize().unwrap());
    }

    #[test]
    fn test_validate_media_path_handles_leading_slash() {
        let temp_dir = tempfile::tempdir().unwrap();
        let test_file = temp_dir.path().join("test.mp4");
        std::fs::write(&test_file, "dummy content").unwrap();

        let result = validate_media_path("/test.mp4", temp_dir.path(), "");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), test_file.canonicalize().unwrap());
    }

    #[test]
    fn test_validate_media_path_handles_url_encoded_spaces() {
        let temp_dir = tempfile::tempdir().unwrap();
        let test_file = temp_dir.path().join("test file.mp4");
        std::fs::write(&test_file, "dummy content").unwrap();

        // URL-encoded space = "%20"
        let result = validate_media_path("test%20file.mp4", temp_dir.path(), "");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), test_file.canonicalize().unwrap());
    }

    #[test]
    fn test_validate_media_path_handles_nested_paths() {
        let temp_dir = tempfile::tempdir().unwrap();
        let subdir = temp_dir.path().join("videos").join("2024");
        std::fs::create_dir_all(&subdir).unwrap();
        let test_file = subdir.join("demo.mp4");
        std::fs::write(&test_file, "dummy content").unwrap();

        let result = validate_media_path("videos/2024/demo.mp4", temp_dir.path(), "");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), test_file.canonicalize().unwrap());
    }

    // ==================== validate_media_path External Static Folder Tests ====================

    #[test]
    fn test_validate_media_path_external_static_folder_works() {
        // Create parent directory with content and static subdirs
        let parent_dir = tempfile::tempdir().unwrap();
        let content_dir = parent_dir.path().join("content");
        let static_dir = parent_dir.path().join("static");
        std::fs::create_dir_all(&content_dir).unwrap();
        std::fs::create_dir_all(static_dir.join("videos")).unwrap();

        // Create a video file in the external static folder
        let video_file = static_dir.join("videos").join("test.mp4");
        std::fs::write(&video_file, "video content").unwrap();

        // static_folder = "../static" relative to content_dir
        let result = validate_media_path("videos/test.mp4", &content_dir, "../static");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), video_file.canonicalize().unwrap());
    }

    #[test]
    fn test_validate_media_path_content_root_takes_precedence() {
        // Create parent directory with content and static subdirs
        let parent_dir = tempfile::tempdir().unwrap();
        let content_dir = parent_dir.path().join("content");
        let static_dir = parent_dir.path().join("static");
        std::fs::create_dir_all(content_dir.join("videos")).unwrap();
        std::fs::create_dir_all(static_dir.join("videos")).unwrap();

        // Create the same file in both locations
        let content_video = content_dir.join("videos").join("test.mp4");
        let static_video = static_dir.join("videos").join("test.mp4");
        std::fs::write(&content_video, "content version").unwrap();
        std::fs::write(&static_video, "static version").unwrap();

        // Content root should take precedence
        let result = validate_media_path("videos/test.mp4", &content_dir, "../static");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), content_video.canonicalize().unwrap());
    }

    #[test]
    fn test_validate_media_path_rejects_traversal_in_external_static() {
        // Create parent directory with content and static subdirs
        let parent_dir = tempfile::tempdir().unwrap();
        let content_dir = parent_dir.path().join("content");
        let static_dir = parent_dir.path().join("static");
        std::fs::create_dir_all(&content_dir).unwrap();
        std::fs::create_dir_all(&static_dir).unwrap();

        // Even with an external static folder, path traversal should be rejected
        let result = validate_media_path("../etc/passwd", &content_dir, "../static");
        assert!(matches!(result, Err(MbrError::DirectoryTraversal)));
    }

    #[test]
    fn test_validate_media_path_empty_static_folder_disables_fallback() {
        // Create a single directory
        let temp_dir = tempfile::tempdir().unwrap();

        // With empty static_folder, only content root is checked
        let result = validate_media_path("nonexistent.mp4", temp_dir.path(), "");
        assert!(matches!(result, Err(MbrError::InvalidMediaPath(_))));
    }

    #[test]
    fn test_validate_media_path_external_static_nested_path() {
        // Create parent directory with content and static subdirs
        let parent_dir = tempfile::tempdir().unwrap();
        let content_dir = parent_dir.path().join("content");
        let static_dir = parent_dir.path().join("static");
        std::fs::create_dir_all(&content_dir).unwrap();
        let nested_dir = static_dir.join("videos").join("Jay Sankey").join("2024");
        std::fs::create_dir_all(&nested_dir).unwrap();

        // Create a video file in nested directory
        let video_file = nested_dir.join("performance.mp4");
        std::fs::write(&video_file, "video content").unwrap();

        let result = validate_media_path(
            "videos/Jay%20Sankey/2024/performance.mp4",
            &content_dir,
            "../static",
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), video_file.canonicalize().unwrap());
    }

    #[test]
    fn test_validate_media_path_nonexistent_static_folder_fallback_fails() {
        // Create content directory only
        let temp_dir = tempfile::tempdir().unwrap();
        let content_dir = temp_dir.path().join("content");
        std::fs::create_dir_all(&content_dir).unwrap();

        // Static folder doesn't exist - should fail gracefully
        let result = validate_media_path("videos/test.mp4", &content_dir, "../nonexistent");
        assert!(matches!(result, Err(MbrError::InvalidMediaPath(_))));
    }

    // ==================== resolve_media_source_file Tests ====================

    #[cfg(feature = "media-metadata")]
    #[test]
    fn test_resolve_media_source_file_rejects_traversal() {
        let temp_dir = tempfile::tempdir().unwrap();
        let base = temp_dir.path().join("base");
        std::fs::create_dir_all(&base).unwrap();
        // Secret file OUTSIDE base - unvalidated join + is_file() would have found it
        std::fs::write(temp_dir.path().join("secret.mp4"), b"secret").unwrap();

        let result = resolve_media_source_file("../secret.mp4", &base, "static");
        assert!(
            result.is_none(),
            "path traversal outside base_dir must be rejected"
        );
    }

    #[cfg(feature = "media-metadata")]
    #[test]
    fn test_resolve_media_source_file_accepts_file_in_base() {
        let temp_dir = tempfile::tempdir().unwrap();
        let video_file = temp_dir.path().join("demo.mp4");
        std::fs::write(&video_file, b"video content").unwrap();

        let result = resolve_media_source_file("demo.mp4", temp_dir.path(), "static");
        assert_eq!(result, Some(video_file.canonicalize().unwrap()));
    }

    #[cfg(feature = "media-metadata")]
    #[test]
    fn test_resolve_media_source_file_accepts_file_in_static_folder() {
        let temp_dir = tempfile::tempdir().unwrap();
        let static_dir = temp_dir.path().join("static");
        std::fs::create_dir_all(&static_dir).unwrap();
        let video_file = static_dir.join("demo.mp4");
        std::fs::write(&video_file, b"video content").unwrap();

        let result = resolve_media_source_file("demo.mp4", temp_dir.path(), "static");
        assert_eq!(result, Some(video_file.canonicalize().unwrap()));
    }

    // ==================== safe_join_asset Tests ====================

    #[test]
    fn test_safe_join_asset_accepts_valid_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let test_file = temp_dir.path().join("theme.css");
        std::fs::write(&test_file, "body {}").unwrap();

        let result = safe_join_asset(temp_dir.path(), "theme.css");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), test_file.canonicalize().unwrap());
    }

    #[test]
    fn test_safe_join_asset_handles_leading_slash() {
        let temp_dir = tempfile::tempdir().unwrap();
        let test_file = temp_dir.path().join("theme.css");
        std::fs::write(&test_file, "body {}").unwrap();

        let result = safe_join_asset(temp_dir.path(), "/theme.css");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), test_file.canonicalize().unwrap());
    }

    #[test]
    fn test_safe_join_asset_rejects_directory_traversal() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Various path traversal attempts
        let attacks = vec![
            "../etc/passwd",
            "../../etc/passwd",
            "foo/../../../etc/passwd",
            "../theme.css",
        ];

        for attack in attacks {
            let result = safe_join_asset(temp_dir.path(), attack);
            assert!(
                result.is_none(),
                "Path traversal should be blocked for: {}",
                attack
            );
        }
    }

    #[test]
    fn test_safe_join_asset_rejects_nonexistent_file() {
        let temp_dir = tempfile::tempdir().unwrap();

        let result = safe_join_asset(temp_dir.path(), "nonexistent.css");
        assert!(result.is_none());
    }

    #[test]
    fn test_safe_join_asset_rejects_directory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let subdir = temp_dir.path().join("subdir");
        std::fs::create_dir(&subdir).unwrap();

        let result = safe_join_asset(temp_dir.path(), "subdir");
        assert!(result.is_none(), "Directories should not be served");
    }

    #[test]
    fn test_safe_join_asset_handles_nested_paths() {
        let temp_dir = tempfile::tempdir().unwrap();
        let nested = temp_dir.path().join("components-js").join("module");
        std::fs::create_dir_all(&nested).unwrap();
        let test_file = nested.join("app.js");
        std::fs::write(&test_file, "export {}").unwrap();

        let result = safe_join_asset(temp_dir.path(), "components-js/module/app.js");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), test_file.canonicalize().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn test_safe_join_asset_blocks_symlink_escape() {
        use std::os::unix::fs::symlink;

        let temp_dir = tempfile::tempdir().unwrap();

        // Create a symlink pointing outside the base directory
        let link_path = temp_dir.path().join("escape");
        if symlink("/tmp", &link_path).is_ok() {
            // Try to access a file through the symlink
            let result = safe_join_asset(temp_dir.path(), "escape/some_file");
            assert!(result.is_none(), "Symlink escape should be blocked");
        }
    }

    fn mk_markdown_info(raw: &str, url: &str, title: &str) -> MarkdownInfo {
        let mut frontmatter = crate::markdown::SimpleMetadata::new();
        frontmatter.insert(
            "title".to_string(),
            serde_json::Value::String(title.to_string()),
        );
        MarkdownInfo {
            raw_path: PathBuf::from(raw),
            url_path: url.to_string(),
            frontmatter: Some(frontmatter),
            created: 0,
            modified: 0,
            relationships: Vec::new(),
        }
    }

    fn title_sort() -> Vec<SortField> {
        vec![SortField {
            field: "title".to_string(),
            order: "asc".to_string(),
            compare: "string".to_string(),
        }]
    }

    /// The memoizable sibling helper (finding #18) must return exactly what the
    /// previous inline full-scan produced for the same directory and sort.
    #[test]
    fn test_compute_sibling_files_matches_full_scan() {
        let files = [
            mk_markdown_info("docs/b.md", "/docs/b/", "Beta"),
            mk_markdown_info("docs/a.md", "/docs/a/", "Alpha"),
            mk_markdown_info("other/c.md", "/other/c/", "Gamma"),
            mk_markdown_info("root.md", "/root/", "Root"),
        ];
        let sort = title_sort();
        let parent = Path::new("docs");

        let got = compute_sibling_files(files.iter(), parent, &sort);

        // Reference implementation of the prior full-scan behavior.
        let mut expected: Vec<serde_json::Value> = files
            .iter()
            .filter_map(|info| {
                let file_parent = info.raw_path.parent()?;
                (file_parent == parent).then(|| markdown_file_to_json(info))
            })
            .collect();
        sort_files(&mut expected, &sort);

        assert_eq!(got, expected);
        assert_eq!(got.len(), 2);
        // Sorted by title ascending regardless of input order.
        assert_eq!(got[0]["title"], "Alpha");
        assert_eq!(got[1]["title"], "Beta");
    }

    /// A directory with no markdown children yields an empty sibling list.
    #[test]
    fn test_compute_sibling_files_no_children() {
        let files = [mk_markdown_info("docs/a.md", "/docs/a/", "Alpha")];
        let got = compute_sibling_files(files.iter(), Path::new("empty"), &title_sort());
        assert!(got.is_empty());
    }

    // ===== directory listings served from the in-memory index =====

    /// Only a file that lives *below* a subdirectory of `dir` contributes a
    /// subdirectory name; a direct child file contributes nothing.
    #[test]
    fn test_immediate_subdir_name_rules() {
        let root = Path::new("");
        assert_eq!(
            immediate_subdir_name(Path::new("docs/guide.md"), root)
                .map(|n| n.to_string_lossy().into_owned()),
            Some("docs".to_string())
        );
        assert_eq!(immediate_subdir_name(Path::new("readme.md"), root), None);
        assert_eq!(
            immediate_subdir_name(Path::new("docs/deep/x.md"), Path::new("docs"))
                .map(|n| n.to_string_lossy().into_owned()),
            Some("deep".to_string())
        );
        // Not below `dir` at all.
        assert_eq!(
            immediate_subdir_name(Path::new("other/x.md"), Path::new("docs")),
            None
        );
        // A direct child file of a non-root directory.
        assert_eq!(
            immediate_subdir_name(Path::new("docs/guide.md"), Path::new("docs")),
            None
        );
    }

    /// Subdirectory entries are deduplicated and name-sorted, and their URLs
    /// are `/`-separated regardless of the host separator.
    #[test]
    fn test_compute_subdir_entries_dedupes_and_sorts() {
        let paths = [
            Path::new("docs/zeta/a.md"),
            Path::new("docs/alpha/b.md"),
            Path::new("docs/alpha/c.md"),
            Path::new("docs/readme.md"),
            Path::new("elsewhere/d.md"),
        ];
        let got = compute_subdir_entries(paths.into_iter(), Path::new("docs"));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0]["name"], "alpha");
        assert_eq!(got[0]["url_path"], "/docs/alpha/");
        assert_eq!(got[1]["name"], "zeta");
        assert_eq!(got[1]["url_path"], "/docs/zeta/");
    }

    /// `.` and the empty path both name the repository root, because that is
    /// the key `raw_path.parent()` produces for a root-level file.
    #[test]
    fn test_listing_dir_key_normalizes_dot_to_root() {
        assert_eq!(listing_dir_key(Path::new(".")), PathBuf::new());
        assert_eq!(listing_dir_key(Path::new("")), PathBuf::new());
        assert_eq!(listing_dir_key(Path::new("docs")), PathBuf::from("docs"));
    }

    /// Builds a small on-disk fixture and returns its canonical root.
    fn listing_fixture() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("docs/deep")).unwrap();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("beta.md"), "---\ntitle: Beta\n---\n\nB\n").unwrap();
        std::fs::write(root.join("alpha.md"), "---\ntitle: Alpha\n---\n\nA\n").unwrap();
        std::fs::write(root.join("docs/guide.md"), "---\ntitle: Guide\n---\n\nG\n").unwrap();
        std::fs::write(root.join("docs/deep/x.md"), "# X\n").unwrap();
        std::fs::write(root.join("assets/pic.png"), b"not-really-a-png").unwrap();
        tmp
    }

    fn fixture_repo(root: &Path) -> Repo {
        Repo::init(
            root,
            "static",
            &["md".to_string()],
            &[],
            &[],
            "index.md",
            &[],
            &[],
        )
    }

    /// The backlink index must report exactly what the full-repository grep it
    /// replaces reported.
    ///
    /// These are two independent implementations of the same question — the
    /// grep matches path spellings inside each file, the index inverts the
    /// renderer's own resolved links — and the server answered `links.json`
    /// with the grep until this index landed. Any disagreement is a
    /// user-visible change in the info panel and the mini graph, so it has to
    /// be asserted rather than assumed.
    #[test]
    fn test_inbound_index_agrees_with_grep_backlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("docs")).unwrap();

        std::fs::write(
            root.join("alpha.md"),
            "---\ntitle: Alpha\n---\n\nSee [the guide](docs/guide.md) and [beta](beta.md).\n",
        )
        .unwrap();
        std::fs::write(
            root.join("beta.md"),
            "---\ntitle: Beta\n---\n\nBack to [alpha](alpha.md).\n",
        )
        .unwrap();
        std::fs::write(
            root.join("docs/guide.md"),
            "---\ntitle: Guide\n---\n\nUp to [alpha](../alpha.md); also [beta](../beta.md).\n",
        )
        .unwrap();
        // A page nobody links to: both implementations must report empty.
        std::fs::write(
            root.join("orphan.md"),
            "---\ntitle: Orphan\n---\n\nAlone.\n",
        )
        .unwrap();

        let repo = Arc::new(fixture_repo(&root));
        repo.scan_all().unwrap();
        repo.build_wikilink_index();

        let cfg = LinkIndexConfig {
            base_dir: root.clone(),
            index_file: "index.md".to_string(),
            markdown_extensions: vec!["md".to_string()],
            valid_tag_sources: HashSet::new(),
        };
        let index = InboundIndex::new();
        populate_inbound_index(&repo, &index, &cfg);
        assert!(index.is_ready());

        for page in ["/alpha/", "/beta/", "/docs/guide/", "/orphan/"] {
            let indexed: Vec<String> = index.get(page).into_iter().map(|l| l.from).collect();

            let mut grepped = crate::link_grep::find_inbound_links(
                page,
                &root,
                &["md".to_string()],
                &[],
                &[],
                "index.md",
            );
            crate::link_index::sort_inbound_links(&mut grepped);
            let grepped: Vec<String> = grepped.into_iter().map(|l| l.from).collect();

            assert_eq!(
                indexed, grepped,
                "backlink sources for {page} disagree: index={indexed:?} grep={grepped:?}"
            );
        }

        // Sanity-check the fixture actually exercises the comparison, so a
        // future edit that silently stops producing backlinks still fails.
        assert_eq!(
            index.get("/alpha/").len(),
            2,
            "alpha should be linked from beta and the guide"
        );
        assert!(index.get("/orphan/").is_empty());
    }

    /// The in-memory listing must reproduce what the per-request disk scan
    /// produced: same files (order included) and same subdirectories (the old
    /// implementation read them out of a concurrent map, so order was
    /// arbitrary — compare as sets).
    #[test]
    fn test_directory_listing_from_index_matches_disk_scan() {
        let tmp = listing_fixture();
        let root = tmp.path().canonicalize().unwrap();
        let sort = title_sort();

        let indexed = fixture_repo(&root);
        indexed.scan_all().unwrap();

        for dir in [Path::new(""), Path::new("docs")] {
            // Reference: the previous per-request implementation.
            let scan_dir = if dir.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                dir.to_path_buf()
            };
            let temp_repo = fixture_repo(&root);
            temp_repo.scan_folder(&scan_dir).unwrap();
            let mut expected_files: Vec<serde_json::Value> = temp_repo
                .markdown_files
                .pin()
                .iter()
                .map(|(_, info)| markdown_file_to_json(info))
                .collect();
            sort_files(&mut expected_files, &sort);
            let abs_dir = root.join(dir);
            let expected_subdirs: std::collections::BTreeSet<String> = temp_repo
                .queued_folders
                .pin()
                .iter()
                .filter_map(|(abs_path, _)| {
                    (abs_path.parent()? == abs_dir.as_path())
                        .then(|| abs_path.file_name()?.to_str().map(|s| s.to_string()))?
                })
                .collect();

            // New: served straight from the repository index.
            let got_files = compute_sibling_files(
                indexed.markdown_files.pin().iter().map(|(_, info)| info),
                dir,
                &sort,
            );
            let markdown_guard = indexed.markdown_files.pin();
            let other_guard = indexed.other_files.pin();
            let got_subdir_entries = compute_subdir_entries(
                markdown_guard
                    .iter()
                    .map(|(_, info)| info.raw_path.as_path())
                    .chain(
                        other_guard
                            .iter()
                            .filter_map(|(abs, _)| abs.strip_prefix(&root).ok()),
                    ),
                dir,
            );
            let got_subdirs: std::collections::BTreeSet<String> = got_subdir_entries
                .iter()
                .filter_map(|entry| entry["name"].as_str().map(|s| s.to_string()))
                .collect();

            assert_eq!(got_files, expected_files, "files mismatch for {dir:?}");
            assert_eq!(
                got_subdirs, expected_subdirs,
                "subdirs mismatch for {dir:?}"
            );
            assert!(
                !got_files.is_empty(),
                "fixture should have files in {dir:?}"
            );
            assert!(
                !got_subdirs.is_empty(),
                "fixture should have subdirs in {dir:?}"
            );
        }
    }

    /// A directory holding only assets (no markdown at any depth) still shows
    /// up in its parent's listing, exactly as the disk scan reported it.
    #[test]
    fn test_subdirs_include_asset_only_directories() {
        let tmp = listing_fixture();
        let root = tmp.path().canonicalize().unwrap();
        let repo = fixture_repo(&root);
        repo.scan_all().unwrap();

        let markdown_guard = repo.markdown_files.pin();
        let other_guard = repo.other_files.pin();
        let entries = compute_subdir_entries(
            markdown_guard
                .iter()
                .map(|(_, info)| info.raw_path.as_path())
                .chain(
                    other_guard
                        .iter()
                        .filter_map(|(abs, _)| abs.strip_prefix(&root).ok()),
                ),
            Path::new(""),
        );
        let names: Vec<&str> = entries
            .iter()
            .filter_map(|e| e["name"].as_str())
            .collect::<Vec<_>>();
        assert!(
            names.contains(&"assets"),
            "asset-only directory must still be listed: {names:?}"
        );
    }

    // ===== site.json payload =====

    /// The site.json body must contain the markdown index and the sort/sidebar
    /// config, and must never materialize the media catalog (`other_files`),
    /// which is served by `/.mbr/media.json`.
    #[test]
    fn test_render_site_json_omits_other_files() {
        let tmp = listing_fixture();
        let root = tmp.path().canonicalize().unwrap();
        let repo = fixture_repo(&root);
        repo.scan_all().unwrap();
        repo.scan_static_folder().unwrap();
        assert!(
            !repo.other_files.pin().is_empty(),
            "fixture must contain at least one non-markdown file"
        );

        let params = SiteJsonParams {
            sort: title_sort(),
            sidebar_style: "panel".to_string(),
            sidebar_max_items: 42,
            relationship_tracking: false,
        };
        let bytes = render_site_json(&repo, &params).expect("site.json renders");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("valid json");

        assert!(
            value.get("other_files").is_none(),
            "site.json must not include the media catalog"
        );
        let files = value["markdown_files"]
            .as_array()
            .expect("markdown_files array");
        assert_eq!(files.len(), 4);
        assert_eq!(value["index_file"], "index.md");
        assert_eq!(value["sidebar_style"], "panel");
        assert_eq!(value["sidebar_max_items"], 42);
        assert_eq!(value["sort"][0]["field"], "title");

        // Deterministic: same snapshot, byte-identical body.
        let again = render_site_json(&repo, &params).expect("site.json renders");
        assert_eq!(bytes, again);
    }

    /// Finding #20: N concurrent requests for the same (path, type) must trigger
    /// exactly one decode; every other request awaits and reads the cached
    /// result rather than starting its own decode.
    #[cfg(feature = "media-metadata")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_metadata_single_flight_one_decode() {
        use crate::video_metadata_cache::{CachedMetadata, VideoMetadataCache};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        let inflight: Arc<papaya::HashMap<String, Arc<tokio::sync::Notify>>> =
            Arc::new(papaya::HashMap::new());
        let cache = Arc::new(VideoMetadataCache::new(1024 * 1024));
        let decodes = Arc::new(AtomicUsize::new(0));
        let key = "videos/clip.mp4::cover::mtime=1".to_string();

        // The producer claims the slot first (as the request handler would).
        let producer_notify = match claim_inflight(&inflight, &key) {
            InflightClaim::Produce(notify) => notify,
            InflightClaim::Wait(_) => panic!("first claim must produce"),
        };

        // Spawn several concurrent "requests" that all find the slot occupied.
        let mut handles = Vec::new();
        for _ in 0..8 {
            let inflight = Arc::clone(&inflight);
            let cache = Arc::clone(&cache);
            let decodes = Arc::clone(&decodes);
            let key = key.clone();
            handles.push(tokio::spawn(async move {
                match claim_inflight(&inflight, &key) {
                    InflightClaim::Produce(_) => {
                        // A waiter must never become a second producer.
                        decodes.fetch_add(1, Ordering::SeqCst);
                        panic!("concurrent request unexpectedly started a decode");
                    }
                    InflightClaim::Wait(notify) => {
                        let notified = notify.notified();
                        tokio::pin!(notified);
                        notified.as_mut().enable();
                        if cache.get(&key).is_none() {
                            tokio::time::timeout(Duration::from_secs(5), notified)
                                .await
                                .expect("waiter timed out");
                        }
                        cache.get(&key).is_some()
                    }
                }
            }));
        }

        // Give the waiters a moment to register interest.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // The single producer performs exactly one decode and publishes it.
        decodes.fetch_add(1, Ordering::SeqCst);
        cache.insert(key.clone(), CachedMetadata::Cover(vec![1, 2, 3, 4]));
        inflight.pin().remove(&key);
        producer_notify.notify_waiters();

        for handle in handles {
            assert!(handle.await.unwrap(), "waiter did not observe the result");
        }
        assert_eq!(
            decodes.load(Ordering::SeqCst),
            1,
            "only the producer should have decoded"
        );
    }

    // ===== single-flight slot release (cancellation safety) =====

    /// A producer whose future is dropped mid-flight (client disconnect) must
    /// release its single-flight slot, so the next request becomes the new
    /// producer instead of waiting on a claim nobody will ever settle.
    #[tokio::test]
    async fn test_inflight_slot_released_when_producer_future_dropped() {
        let inflight: Arc<papaya::HashMap<String, Arc<tokio::sync::Notify>>> =
            Arc::new(papaya::HashMap::new());
        let key = "videos/clip.mp4::cover::mtime=1";

        {
            let inflight = Arc::clone(&inflight);
            let producing = async {
                let notify = match claim_inflight(&inflight, key) {
                    InflightClaim::Produce(notify) => notify,
                    InflightClaim::Wait(_) => panic!("first claim must produce"),
                };
                let _slot = InflightSlot::new(Arc::clone(&inflight), key.to_string(), notify);
                // Stands in for the `spawn_blocking` await the handler is
                // parked on when the client goes away.
                std::future::pending::<()>().await;
            };
            // `timeout` polls the inner future once, then drops it — exactly
            // what axum does to a handler whose connection died.
            assert!(
                tokio::time::timeout(std::time::Duration::ZERO, producing)
                    .await
                    .is_err()
            );
        }

        assert!(
            inflight.pin().get(key).is_none(),
            "cancelled producer must not leave its claim behind"
        );
        assert!(
            matches!(claim_inflight(&inflight, key), InflightClaim::Produce(_)),
            "next request must be able to produce, not wedge on a stale claim"
        );
    }

    /// Waiters parked on a cancelled producer's notify are woken rather than
    /// left to burn the full wait timeout.
    #[tokio::test]
    async fn test_inflight_slot_wakes_waiters_when_dropped() {
        let inflight: Arc<papaya::HashMap<String, Arc<tokio::sync::Notify>>> =
            Arc::new(papaya::HashMap::new());
        let key = "videos/clip.mp4::chapters::mtime=1";

        let producer_notify = match claim_inflight(&inflight, key) {
            InflightClaim::Produce(notify) => notify,
            InflightClaim::Wait(_) => panic!("first claim must produce"),
        };
        let waiter_notify = match claim_inflight(&inflight, key) {
            InflightClaim::Wait(notify) => notify,
            InflightClaim::Produce(_) => panic!("second claim must wait"),
        };
        let notified = waiter_notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();

        drop(InflightSlot::new(
            Arc::clone(&inflight),
            key.to_string(),
            producer_notify,
        ));

        tokio::time::timeout(std::time::Duration::from_secs(5), notified)
            .await
            .expect("waiter must be woken when the producer releases the slot");
    }

    /// `claim_inflight` must admit exactly one producer per key even when many
    /// threads claim it at the same instant: `pin()` is only an epoch guard, so
    /// a `get`-then-`insert` pair can hand `Produce` to two callers.
    #[test]
    fn test_claim_inflight_admits_one_producer_under_concurrent_claims() {
        use std::sync::Barrier;
        use std::sync::atomic::{AtomicUsize, Ordering};

        const THREADS: usize = 8;
        const KEYS: usize = 400;

        let inflight: Arc<papaya::HashMap<String, Arc<tokio::sync::Notify>>> =
            Arc::new(papaya::HashMap::new());
        let barrier = Arc::new(Barrier::new(THREADS));
        let producers = Arc::new(AtomicUsize::new(0));

        std::thread::scope(|scope| {
            for _ in 0..THREADS {
                let inflight = Arc::clone(&inflight);
                let barrier = Arc::clone(&barrier);
                let producers = Arc::clone(&producers);
                scope.spawn(move || {
                    for i in 0..KEYS {
                        let key = format!("key-{i}");
                        // All threads race on the same key at the same moment.
                        barrier.wait();
                        if let InflightClaim::Produce(_) = claim_inflight(&inflight, &key) {
                            producers.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
            }
        });

        assert_eq!(
            producers.load(Ordering::Relaxed),
            KEYS,
            "each key must be claimed by exactly one producer"
        );
    }

    /// An abandoned HLS generation must not leave the key stuck `InProgress`:
    /// that state has no TTL and is never evicted, so every later request would
    /// block for `HLS_WAIT_TIMEOUT` and then 404 until restart.
    #[cfg(feature = "media-metadata")]
    #[tokio::test]
    async fn test_hls_generation_slot_released_when_future_dropped() {
        use crate::video_transcode::TranscodeTarget;
        use crate::video_transcode_cache::{HlsCacheKey, HlsCacheStartResult};

        let cache = HlsCache::new(1024 * 1024);
        let key = HlsCacheKey::segment(
            PathBuf::from("/videos/clip.mp4"),
            TranscodeTarget::Resolution720p,
            3,
        );

        {
            let key = key.clone();
            let producing = async {
                let notify = match cache.start_generation(key.clone()) {
                    HlsCacheStartResult::Started(notify) => notify,
                    _ => panic!("first start_generation must start"),
                };
                let _slot = HlsGenerationSlot::new(&cache, key, notify);
                std::future::pending::<()>().await;
            };
            assert!(
                tokio::time::timeout(std::time::Duration::ZERO, producing)
                    .await
                    .is_err()
            );
        }

        // The slot is settled (not `InProgress`), so the next request gets an
        // immediate answer instead of waiting on a generation nobody is running.
        assert!(
            !matches!(
                cache.start_generation(key),
                HlsCacheStartResult::AlreadyInProgress(_)
            ),
            "cancelled generation must not leave the key wedged in progress"
        );
    }

    // ===== live reload broadcast handling =====

    /// A lagging live-reload client must keep receiving events. Consuming the
    /// broadcast with a refutable `Ok(..)` pattern inside `select!` disables the
    /// branch permanently on the first `Lagged`, silently killing live reload
    /// for that tab.
    #[tokio::test]
    async fn test_live_reload_action_keeps_forwarding_after_lag() {
        use crate::watcher::{ChangeEventType, FileChangeEvent};

        let event = |name: &str| FileChangeEvent {
            path: format!("/repo/{name}"),
            relative_path: name.to_string(),
            event: ChangeEventType::Modified,
        };

        let (tx, mut rx) = broadcast::channel::<FileChangeEvent>(2);
        // Overflow the channel so the receiver falls behind.
        for i in 0..5 {
            tx.send(event(&format!("a{i}.md"))).unwrap();
        }

        assert!(
            matches!(live_reload_action(rx.recv().await), LiveReloadAction::Skip),
            "an overflowed receiver must report lag, not terminate"
        );

        // Everything still queued, and everything sent afterwards, is forwarded.
        assert!(matches!(
            live_reload_action(rx.recv().await),
            LiveReloadAction::Forward(_)
        ));
        tx.send(event("after-lag.md")).unwrap();
        while let LiveReloadAction::Forward(forwarded) = live_reload_action(rx.recv().await) {
            if forwarded.relative_path == "after-lag.md" {
                return;
            }
        }
        panic!("event sent after the lag was never forwarded");
    }

    /// When the sender is dropped (server shutting down) the loop closes.
    #[tokio::test]
    async fn test_live_reload_action_closes_when_sender_dropped() {
        let (tx, mut rx) = broadcast::channel::<crate::watcher::FileChangeEvent>(2);
        drop(tx);
        assert!(matches!(
            live_reload_action(rx.recv().await),
            LiveReloadAction::Close
        ));
    }

    // ===== derived cache invalidation =====

    /// A file change must drop the link caches too, not just sibling nav:
    /// `LinkCache` has no TTL, so a survivor serves pre-edit links forever.
    /// The directory-listing subdirectories and the serialized site.json body
    /// are derived from the same data and must go with them.
    #[test]
    fn test_invalidate_derived_caches_clears_link_caches() {
        use crate::link_index::{InboundLink, OutboundLink};

        let listing_caches = ListingCaches {
            sibling_nav_cache: Arc::new(papaya::HashMap::new()),
            subdir_cache: Arc::new(papaya::HashMap::new()),
            site_json_cache: Arc::new(parking_lot::RwLock::new(SiteJsonCache {
                generation: 0,
                body: Some(axum::body::Bytes::from_static(b"{\"markdown_files\":[]}")),
            })),
        };
        listing_caches
            .sibling_nav_cache
            .pin()
            .insert(PathBuf::from("docs"), Arc::new(vec![serde_json::json!({})]));
        listing_caches
            .subdir_cache
            .pin()
            .insert(PathBuf::new(), Arc::new(vec![serde_json::json!({})]));

        let link_cache = LinkCache::new(1024 * 1024);
        link_cache.insert(
            "/docs/guide/".to_string(),
            vec![OutboundLink {
                to: "/docs/other/".to_string(),
                text: "Other".to_string(),
                anchor: None,
                internal: true,
            }],
        );

        let inbound_link_cache = InboundLinkCache::new(1024 * 1024, 300);
        inbound_link_cache.insert(
            "/docs/other/".to_string(),
            vec![InboundLink {
                from: "/docs/guide/".to_string(),
                text: "Other".to_string(),
                anchor: None,
            }],
        );

        invalidate_derived_caches(&listing_caches, &link_cache, &inbound_link_cache);

        assert!(listing_caches.sibling_nav_cache.pin().is_empty());
        assert!(
            listing_caches.subdir_cache.pin().is_empty(),
            "directory subdirectory lists must be dropped on file change"
        );
        assert!(
            listing_caches.site_json_cache.read().body.is_none(),
            "the cached site.json body must be dropped on file change"
        );
        assert!(
            link_cache.get("/docs/guide/").is_none(),
            "outbound link cache must be dropped on file change"
        );
        assert!(
            inbound_link_cache.get("/docs/other/").is_none(),
            "inbound link cache must be dropped on file change"
        );
    }

    /// A site.json rebuild that started before a file change must not publish
    /// its pre-change snapshot afterwards: the invalidation already ran, so the
    /// stale body would be served until the *next* change.
    #[test]
    fn test_site_json_cache_rejects_store_from_a_raced_rebuild() {
        let cache = parking_lot::RwLock::new(SiteJsonCache::default());

        // A request finds the slot empty and starts rebuilding.
        let (generation, cached) = cache.read().snapshot();
        assert!(cached.is_none());

        // A file change lands while that rebuild is in flight.
        cache.write().invalidate();

        // The rebuild finishes and tries to publish its stale snapshot.
        cache
            .write()
            .store(generation, axum::body::Bytes::from_static(b"stale"));
        assert!(
            cache.read().body.is_none(),
            "a body built before the invalidation must not be published"
        );

        // A rebuild started after the change publishes normally.
        let (generation, _) = cache.read().snapshot();
        cache
            .write()
            .store(generation, axum::body::Bytes::from_static(b"fresh"));
        assert_eq!(cache.read().body.as_deref(), Some(&b"fresh"[..]));
    }

    /// The media metadata cache must not inherit the oembed *text* cache
    /// budget: disabling link previews with `--oembed-cache-size 0` used to
    /// disable cover/chapter/caption caching along with it.
    #[cfg(feature = "media-metadata")]
    #[test]
    fn test_media_cache_size_is_independent_of_oembed_cache_size() {
        use crate::video_metadata_cache::{CachedMetadata, VideoMetadataCache};

        let config = crate::config::Config {
            oembed_cache_size: 0,
            ..Default::default()
        };
        let server_config = ServerConfig::from(&config);
        assert_eq!(server_config.oembed_cache_size, 0);
        assert_eq!(server_config.media_cache_size, 64 * 1024 * 1024);

        // The cache built from that budget still caches a cover payload.
        let cache = VideoMetadataCache::new(server_config.media_cache_size);
        let key = "videos/clip.mp4::cover::mtime=1".to_string();
        cache.insert(key.clone(), CachedMetadata::Cover(vec![0u8; 256 * 1024]));
        assert!(
            cache.get(&key).is_some(),
            "media caching must stay enabled when the oembed cache is disabled"
        );

        // And the default budget holds many covers, not a couple of dozen.
        let cache = VideoMetadataCache::new(server_config.media_cache_size);
        for i in 0..100 {
            cache.insert(
                format!("videos/clip-{i}.mp4::cover::mtime=1"),
                CachedMetadata::Cover(vec![0u8; 256 * 1024]),
            );
        }
        assert_eq!(
            cache.len(),
            100,
            "100 covers of 256 KB must fit in the default media cache"
        );
    }

    /// `announce_listening` routes the startup banner on `Server.gui_mode`:
    /// stdout in server mode, `tracing::info!` in GUI mode. That only works if
    /// `init` actually carries the flag off `ServerConfig` onto `Server` — if
    /// it were dropped and defaulted to `false`, the GUI would silently go back
    /// to printing to the console window it is trying to stay out of, and every
    /// existing test would still pass.
    #[tokio::test]
    async fn test_server_init_carries_gui_mode() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("index.md"), "# Test").unwrap();

        for gui_mode in [true, false] {
            let config = crate::config::Config {
                root_dir: temp.path().to_path_buf(),
                ..Default::default()
            };
            let server = Server::init(ServerConfig::from(&config).with_gui_mode(gui_mode))
                .expect("server init should succeed over a temp repo");

            assert_eq!(
                server.gui_mode, gui_mode,
                "Server::init must carry gui_mode through to Server; \
                 announce_listening reads it to decide stdout vs tracing"
            );
        }
    }

    /// Call-site smoke test for the metadata cache path: a cached cover produces
    /// a JPEG response, and a negative marker falls through to `None` (404).
    #[cfg(feature = "media-metadata")]
    #[test]
    fn test_metadata_response_from_cache_variants() {
        use crate::video_metadata_cache::CachedMetadata;

        let jpg = Server::metadata_response_from_cache(CachedMetadata::Cover(vec![0xFF, 0xD8]));
        assert!(jpg.is_some());
        assert_eq!(
            jpg.unwrap().headers().get(header::CONTENT_TYPE).unwrap(),
            "image/jpeg"
        );

        let vtt =
            Server::metadata_response_from_cache(CachedMetadata::Captions("WEBVTT".to_string()));
        assert!(vtt.is_some());

        assert!(Server::metadata_response_from_cache(CachedMetadata::NotAvailable).is_none());
    }

    // ===== resolve_new_target_path (file-management path safety) =====

    #[test]
    fn test_resolve_new_target_rejects_parent_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        assert!(matches!(
            resolve_new_target_path(&base, "../escape.md"),
            Err(FileOpError::Traversal)
        ));
        assert!(matches!(
            resolve_new_target_path(&base, "docs/../../escape.md"),
            Err(FileOpError::Traversal)
        ));
        // Nothing was written outside the root.
        assert!(!base.join("../escape.md").exists());
    }

    #[test]
    fn test_resolve_new_target_rejects_empty_or_root() {
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        assert!(matches!(
            resolve_new_target_path(&base, ""),
            Err(FileOpError::Traversal)
        ));
        assert!(matches!(
            resolve_new_target_path(&base, "/"),
            Err(FileOpError::Traversal)
        ));
    }

    #[test]
    fn test_resolve_new_target_accepts_valid_new_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let resolved = resolve_new_target_path(&base, "docs/new.md").expect("valid path");
        assert_eq!(resolved, base.join("docs/new.md"));
    }

    #[test]
    fn test_resolve_new_target_normalizes_leading_slash() {
        // A leading slash is treated as repo-root-relative, never an escape.
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let resolved = resolve_new_target_path(&base, "/docs/new.md").expect("valid path");
        assert_eq!(resolved, base.join("docs/new.md"));
    }

    // ===== sanitize_upload_name (upload filename validation) =====

    fn md_exts() -> Vec<String> {
        vec!["md".to_string(), "markdown".to_string()]
    }

    #[test]
    fn test_sanitize_upload_name_basename_and_trim() {
        assert_eq!(
            sanitize_upload_name("image.png", &md_exts()).as_deref(),
            Some("image.png")
        );
        // Surrounding whitespace is trimmed.
        assert_eq!(
            sanitize_upload_name("  pic.jpeg  ", &md_exts()).as_deref(),
            Some("pic.jpeg")
        );
        // Multiple dots: only the final segment is the extension.
        assert_eq!(
            sanitize_upload_name("clip.final.cut.mp4", &md_exts()).as_deref(),
            Some("clip.final.cut.mp4")
        );
    }

    #[test]
    fn test_sanitize_upload_name_enforces_media_allowlist() {
        // Executable/site-controlling types must never be uploadable: these are
        // what turned the media uploader into a page-hijacking primitive.
        for name in [
            "index.html",
            "mbr-components.min.js",
            "theme.css",
            "config.toml",
            "evil.svg",
            "archive.tar.gz",
            "payload.sh",
        ] {
            assert_eq!(
                sanitize_upload_name(name, &md_exts()),
                None,
                "{name} must not be uploadable"
            );
        }
        // Media stays uploadable, extension case-insensitively.
        for name in ["pic.png", "PIC.PNG", "clip.mp4", "song.mp3", "doc.pdf"] {
            assert_eq!(
                sanitize_upload_name(name, &md_exts()).as_deref(),
                Some(name),
                "{name} must be uploadable"
            );
        }
    }

    #[test]
    fn test_sanitize_upload_name_rejects_separators_and_traversal() {
        assert_eq!(sanitize_upload_name("../secret.png", &md_exts()), None);
        assert_eq!(sanitize_upload_name("notes/pic.png", &md_exts()), None);
        assert_eq!(sanitize_upload_name("a\\b.png", &md_exts()), None);
        assert_eq!(sanitize_upload_name("..", &md_exts()), None);
        assert_eq!(sanitize_upload_name(".", &md_exts()), None);
        assert_eq!(sanitize_upload_name("my..pic.png", &md_exts()), None);
    }

    #[test]
    fn test_sanitize_upload_name_requires_stem_and_extension() {
        // No extension at all.
        assert_eq!(sanitize_upload_name("noext", &md_exts()), None);
        // Dotfile with no other extension → no extension.
        assert_eq!(sanitize_upload_name(".png", &md_exts()), None);
        // Trailing dot → empty extension.
        assert_eq!(sanitize_upload_name("pic.", &md_exts()), None);
        // Empty / whitespace-only.
        assert_eq!(sanitize_upload_name("", &md_exts()), None);
        assert_eq!(sanitize_upload_name("   ", &md_exts()), None);
    }

    #[test]
    fn test_sanitize_upload_name_rejects_markdown_extensions() {
        // Markdown files must be created via /.mbr/create, not uploaded.
        assert_eq!(sanitize_upload_name("note.md", &md_exts()), None);
        assert_eq!(sanitize_upload_name("note.markdown", &md_exts()), None);
        // Case-insensitive.
        assert_eq!(sanitize_upload_name("note.MD", &md_exts()), None);
        // A non-markdown extension is still accepted.
        assert_eq!(
            sanitize_upload_name("note.pdf", &md_exts()).as_deref(),
            Some("note.pdf")
        );
    }

    // ===== Host header validation (DNS-rebinding defense) =====

    fn host_headers(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn test_host_header_hostname_strips_port_and_brackets() {
        assert_eq!(host_header_hostname("127.0.0.1:5200"), "127.0.0.1");
        assert_eq!(host_header_hostname("localhost:5200"), "localhost");
        assert_eq!(host_header_hostname("localhost"), "localhost");
        assert_eq!(host_header_hostname("[::1]:5200"), "::1");
        assert_eq!(host_header_hostname("[::1]"), "::1");
        assert_eq!(host_header_hostname("::1"), "::1");
        assert_eq!(
            host_header_hostname(" evil.example.com "),
            "evil.example.com"
        );
    }

    #[test]
    fn test_host_header_is_allowed_accepts_local_names() {
        let loopback = [127, 0, 0, 1];
        for value in [
            "localhost",
            "LocalHost:5200",
            "127.0.0.1:5200",
            "127.0.0.53",
            "[::1]:5200",
        ] {
            assert!(
                host_header_is_allowed(&host_headers(value), loopback),
                "{value} should be an allowed Host"
            );
        }
        // A non-loopback bind address is allowed under its own IP literal.
        assert!(host_header_is_allowed(
            &host_headers("192.168.1.5:5200"),
            [192, 168, 1, 5]
        ));
    }

    #[test]
    fn test_host_header_is_allowed_rejects_rebinding_names() {
        let loopback = [127, 0, 0, 1];
        // A DNS-rebinding host resolves to 127.0.0.1 but must not be trusted,
        // and neither may a name that merely embeds an allowed one.
        for value in [
            "evil.example.com",
            "evil.example.com:5200",
            "localhost.evil.example.com",
            "127.0.0.1.evil.example.com",
            "192.168.1.5",
        ] {
            assert!(
                !host_header_is_allowed(&host_headers(value), loopback),
                "{value} must be rejected"
            );
        }
        // A missing Host header is rejected (HTTP/1.1 requires one).
        assert!(!host_header_is_allowed(&HeaderMap::new(), loopback));
    }

    // ===== /.mbr asset allowlist =====

    #[test]
    fn test_is_servable_mbr_asset_allows_known_asset_types() {
        for path in [
            "/theme.css",
            "/components/mbr-components.min.js",
            "/components/mbr-graph.min.js.map",
            "/favicon.png",
            "/fonts/KaTeX_Main-Bold.woff2",
            "/index.html",
        ] {
            assert!(is_servable_mbr_asset(path), "{path} should be servable");
        }
    }

    #[test]
    fn test_is_servable_mbr_asset_rejects_config_and_dotfiles() {
        // config.toml carries the Argon2 edit_token_hash.
        for path in [
            "/config.toml",
            "/config.TOML",
            "/.env",
            "/secrets/.env.local",
            "/notes.md",
            "/id_rsa",
            "/",
        ] {
            assert!(!is_servable_mbr_asset(path), "{path} must not be servable");
        }
    }

    // ===== upload destination guard =====

    #[test]
    fn test_is_template_folder_path_blocks_mbr_and_template_folder() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(base.join(".mbr/components")).unwrap();
        let template_folder = base.join("custom-templates");
        std::fs::create_dir_all(&template_folder).unwrap();

        // Targets inside .mbr/ (which do not exist yet) are rejected.
        for rel in [".mbr/index.html", ".mbr/components/mbr-components.min.js"] {
            assert!(
                is_template_folder_path(&base.join(rel), &base, Some(&base), None),
                "{rel} must be blocked"
            );
        }
        // So are targets inside an explicit --template-folder.
        assert!(is_template_folder_path(
            &template_folder.join("index.html"),
            &base,
            Some(&base),
            Some(&template_folder)
        ));
        // Ordinary note folders are unaffected.
        assert!(!is_template_folder_path(
            &base.join("notes/pic.png"),
            &base,
            Some(&base),
            Some(&template_folder)
        ));
        // A folder that merely starts with the same characters is not `.mbr`.
        assert!(!is_template_folder_path(
            &base.join(".mbrx/pic.png"),
            &base,
            Some(&base),
            None
        ));
    }

    // ===== served-path containment (symlink escape defense in depth) =====

    #[test]
    fn test_is_within_served_roots_accepts_repo_and_static_overlay() {
        let parent = tempfile::tempdir().unwrap();
        let repo = parent.path().join("content");
        let static_dir = parent.path().join("static");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&static_dir).unwrap();
        let inside = repo.join("note.md");
        std::fs::write(&inside, "# hi").unwrap();
        let overlay = static_dir.join("logo.png");
        std::fs::write(&overlay, "png").unwrap();
        let canonical_repo = repo.canonicalize().unwrap();

        assert!(is_within_served_roots(
            &inside,
            &repo,
            Some(&canonical_repo),
            "static"
        ));
        assert!(is_within_served_roots(
            &repo,
            &repo,
            Some(&canonical_repo),
            ""
        ));
        // The static_folder overlay may legitimately live outside the root.
        assert!(is_within_served_roots(
            &overlay,
            &repo,
            Some(&canonical_repo),
            "../static"
        ));
    }

    #[test]
    fn test_is_within_served_roots_rejects_symlink_escape() {
        let parent = tempfile::tempdir().unwrap();
        let repo = parent.path().join("content");
        std::fs::create_dir_all(&repo).unwrap();
        let outside = parent.path().join("secret.txt");
        std::fs::write(&outside, "top secret").unwrap();
        let canonical_repo = repo.canonicalize().unwrap();

        // A path that is lexically inside the repo but canonicalizes outside it
        // (a symlink) must be rejected.
        let link = repo.join("secret.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        #[cfg(windows)]
        let _ = std::os::windows::fs::symlink_file(&outside, &link);

        if link.exists() {
            assert!(!is_within_served_roots(
                &link,
                &repo,
                Some(&canonical_repo),
                "static"
            ));
        }
        // A path that does not exist at all is never servable.
        assert!(!is_within_served_roots(
            &repo.join("missing.txt"),
            &repo,
            Some(&canonical_repo),
            "static"
        ));
    }

    // ===== dedupe_name (collision suffixing) =====

    #[test]
    fn test_dedupe_name_no_collision() {
        let dir = Path::new("/repo/notes");
        let chosen = dedupe_name(dir, "pic", "png", |_| false);
        assert_eq!(chosen, dir.join("pic.png"));
    }

    #[test]
    fn test_dedupe_name_first_collision() {
        let dir = Path::new("/repo");
        let taken: std::collections::HashSet<PathBuf> = [dir.join("a.txt")].into_iter().collect();
        let chosen = dedupe_name(dir, "a", "txt", |p| taken.contains(p));
        assert_eq!(chosen, dir.join("a-1.txt"));
    }

    #[test]
    fn test_dedupe_name_suffix_sequence() {
        let dir = Path::new("/repo/notes");
        let taken: std::collections::HashSet<PathBuf> = [
            dir.join("pic.png"),
            dir.join("pic-1.png"),
            dir.join("pic-2.png"),
        ]
        .into_iter()
        .collect();
        let chosen = dedupe_name(dir, "pic", "png", |p| taken.contains(p));
        assert_eq!(chosen, dir.join("pic-3.png"));
    }

    // --- Compression predicate --------------------------------------------

    fn headers_with_content_type(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(value).unwrap());
        headers
    }

    /// Builds a response big enough to clear tower-http's 32-byte `SizeAbove`
    /// floor, so the content-type rule is what decides the outcome.
    fn response_with_content_type(value: &str) -> Response<Body> {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, value)
            .header(header::CONTENT_LENGTH, 1_000_000)
            .body(Body::from(vec![0u8; 64]))
            .unwrap()
    }

    #[test]
    fn video_and_audio_content_types_are_incompressible() {
        // Regression: gzipping video makes tower-http drop `content-length`
        // and `accept-ranges`, which breaks seeking and duration in WebKit.
        for content_type in [
            "video/mp4",
            "video/quicktime",
            "video/mp4; charset=binary",
            "audio/mpeg",
            "application/pdf",
            "application/zip",
            "application/gzip",
            "application/x-gzip",
            "application/octet-stream",
        ] {
            assert!(
                is_incompressible_content_type(&headers_with_content_type(content_type)),
                "{content_type} must bypass compression"
            );
        }
    }

    #[test]
    fn text_content_types_stay_compressible() {
        for content_type in [
            "text/html; charset=utf-8",
            "text/css",
            "application/json",
            "application/javascript",
            "image/svg+xml",
        ] {
            assert!(
                !is_incompressible_content_type(&headers_with_content_type(content_type)),
                "{content_type} should still be compressed"
            );
        }
    }

    #[test]
    fn missing_content_type_stays_compressible() {
        assert!(!is_incompressible_content_type(&HeaderMap::new()));
    }

    #[test]
    fn compression_predicate_skips_media_and_keeps_defaults() {
        use tower_http::compression::predicate::Predicate;

        let predicate = compression_predicate();

        assert!(!predicate.should_compress(&response_with_content_type("video/mp4")));
        assert!(!predicate.should_compress(&response_with_content_type("audio/mpeg")));
        // DefaultPredicate exclusions must survive the composition.
        assert!(!predicate.should_compress(&response_with_content_type("image/png")));
        assert!(!predicate.should_compress(&response_with_content_type("text/event-stream")));
        // ...and normal text still compresses.
        assert!(predicate.should_compress(&response_with_content_type("text/html")));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    // Strategy for valid path component names
    fn path_component_strategy() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_-]{1,15}"
    }

    proptest! {
        /// Breadcrumb count: Home + all components except the last (current dir)
        /// For 0 components: [] = 0 (root page, no breadcrumbs to avoid "Home > Home")
        /// For 1 component: [Home] = 1 (last component is current dir, not a link)
        /// For 2+ components: [Home, c1, c2, ...] = components.len()
        #[test]
        fn prop_breadcrumb_count_matches_path_depth(
            components in proptest::collection::vec(path_component_strategy(), 0..5)
        ) {
            let path_str = components.join("/");
            let path = Path::new(&path_str);
            let breadcrumbs = generate_breadcrumbs(path);

            // Breadcrumbs = "Home" + all components except the last (which is current dir)
            // For empty path (root), return empty to avoid "Home > Home"
            let expected_count = if components.is_empty() {
                0  // Empty for root page
            } else {
                components.len()  // Home + all but last = components.len()
            };
            prop_assert_eq!(
                breadcrumbs.len(),
                expected_count,
                "Path {:?} should have {} breadcrumbs, got {}",
                path,
                expected_count,
                breadcrumbs.len()
            );
        }

        /// For non-empty paths, first breadcrumb is always "Home" with url "/"
        #[test]
        fn prop_first_breadcrumb_is_home(
            components in proptest::collection::vec(path_component_strategy(), 1..5)
        ) {
            let path_str = components.join("/");
            let path = Path::new(&path_str);
            let breadcrumbs = generate_breadcrumbs(path);

            prop_assert!(!breadcrumbs.is_empty(), "Non-root paths should have at least Home breadcrumb");
            prop_assert_eq!(&breadcrumbs[0].name, "Home");
            prop_assert_eq!(&breadcrumbs[0].url, "/");
        }

        /// For 2+ components, last breadcrumb is second-to-last path component
        #[test]
        fn prop_last_breadcrumb_matches_parent_component(
            components in proptest::collection::vec(path_component_strategy(), 2..5)
        ) {
            let path_str = components.join("/");
            let path = Path::new(&path_str);
            let breadcrumbs = generate_breadcrumbs(path);

            let last_breadcrumb = breadcrumbs.last().unwrap();
            // The second-to-last component is the parent dir
            let parent_component = &components[components.len() - 2];
            prop_assert_eq!(
                &last_breadcrumb.name,
                parent_component,
                "Last breadcrumb should be {:?}, got {:?}",
                parent_component,
                last_breadcrumb.name
            );
        }

        /// All breadcrumb URLs end with /
        #[test]
        fn prop_breadcrumb_urls_end_with_slash(
            components in proptest::collection::vec(path_component_strategy(), 0..5)
        ) {
            let path_str = components.join("/");
            let path = Path::new(&path_str);
            let breadcrumbs = generate_breadcrumbs(path);

            for bc in &breadcrumbs {
                prop_assert!(
                    bc.url.ends_with('/'),
                    "Breadcrumb URL {:?} should end with /",
                    bc.url
                );
            }
        }

        /// get_current_dir_name returns the last path component
        #[test]
        fn prop_current_dir_name_is_last_component(
            components in proptest::collection::vec(path_component_strategy(), 1..5)
        ) {
            let path_str = components.join("/");
            let path = Path::new(&path_str);
            let name = get_current_dir_name(path);

            let expected = components.last().unwrap();
            prop_assert_eq!(
                &name,
                expected,
                "Current dir name should be {:?}, got {:?}",
                expected,
                name
            );
        }

        /// get_parent_path returns None for root, Some for others
        #[test]
        fn prop_parent_path_behavior(
            components in proptest::collection::vec(path_component_strategy(), 0..5)
        ) {
            let path_str = components.join("/");
            let path = Path::new(&path_str);
            let parent = get_parent_path(path);

            if components.is_empty() {
                prop_assert!(parent.is_none(), "Root should have no parent");
            } else {
                prop_assert!(parent.is_some(), "Non-root should have parent");
                let parent_str = parent.unwrap();
                prop_assert!(
                    parent_str.ends_with('/'),
                    "Parent path should end with /: {:?}",
                    parent_str
                );
            }
        }

        /// Parent path is shorter than original path (fewer characters)
        #[test]
        fn prop_parent_path_shorter_than_original(
            components in proptest::collection::vec(path_component_strategy(), 2..5)
        ) {
            // Need at least 2 components - for single component, parent is "/"
            // which is hard to compare meaningfully
            let path_str = components.join("/");
            let path = Path::new(&path_str);

            if let Some(parent) = get_parent_path(path) {
                // Parent path should be shorter in character length
                // (excluding the trailing slash we add)
                let parent_trimmed = parent.trim_end_matches('/');
                prop_assert!(
                    parent_trimmed.len() < path_str.len(),
                    "Parent {:?} should be shorter than {:?}",
                    parent_trimmed,
                    path_str
                );
            }
        }

        // ==================== validate_media_path Property Tests ====================

        /// Any path containing ".." should be rejected
        #[test]
        fn prop_validate_media_path_rejects_dotdot(
            prefix in "[a-zA-Z0-9_-]{0,10}",
            suffix in "[a-zA-Z0-9_-]{0,10}"
        ) {
            let temp_dir = tempfile::tempdir().unwrap();
            // Test various ".." patterns
            let test_paths = vec![
                format!("{}/../{}", prefix, suffix),
                format!("../{}/{}", prefix, suffix),
                format!("{}/{}/..", prefix, suffix),
                format!("{}%2F..%2F{}", prefix, suffix), // URL-encoded /
            ];

            for path in test_paths {
                // Any path with ".." should be rejected as directory traversal
                // Note: URL-decoded path is what matters
                if path.contains("..") {
                    let result = validate_media_path(&path, temp_dir.path(), "");
                    // Path either doesn't exist or is rejected as traversal
                    prop_assert!(
                        result.is_err(),
                        "Path containing '..' should be rejected: {:?}",
                        path
                    );
                }
            }
        }

        /// validate_media_path is deterministic - same input always gives same output
        #[test]
        fn prop_validate_media_path_deterministic(
            path in "[a-zA-Z0-9_/-]{1,30}"
        ) {
            let temp_dir = tempfile::tempdir().unwrap();
            let result1 = validate_media_path(&path, temp_dir.path(), "");
            let result2 = validate_media_path(&path, temp_dir.path(), "");

            // Both should be the same (both errors or both same Ok value)
            match (&result1, &result2) {
                (Ok(p1), Ok(p2)) => prop_assert_eq!(p1, p2),
                (Err(_), Err(_)) => (), // Both errors is fine
                _ => prop_assert!(false, "Results should be consistent: {:?} vs {:?}", result1, result2),
            }
        }

        /// URL-encoded paths decode correctly
        #[test]
        fn prop_validate_media_path_decodes_url_encoding(
            filename in "[a-zA-Z0-9]{1,15}"
        ) {
            let temp_dir = tempfile::tempdir().unwrap();

            // Create a test file
            let test_file = temp_dir.path().join(&filename);
            std::fs::write(&test_file, "test").unwrap();

            // Test with URL-encoded path (spaces as %20)
            let encoded = format!("%20{}", filename); // Leading space encoded
            let result = validate_media_path(&encoded, temp_dir.path(), "");

            // The decoded path " filename" doesn't exist, so should fail
            prop_assert!(result.is_err(), "Encoded path with non-existent target should fail");

            // Test with the actual filename - should succeed
            let result = validate_media_path(&filename, temp_dir.path(), "");
            prop_assert!(result.is_ok(), "Valid path should succeed: {:?}", filename);
        }

        /// Valid paths within repo root succeed
        #[test]
        fn prop_validate_media_path_valid_paths_succeed(
            filename in "[a-zA-Z0-9_-]{1,15}"
        ) {
            let temp_dir = tempfile::tempdir().unwrap();

            // Create a test file
            let test_file = temp_dir.path().join(&filename);
            std::fs::write(&test_file, "test content").unwrap();

            // Validate the path
            let result = validate_media_path(&filename, temp_dir.path(), "");
            prop_assert!(result.is_ok(), "Valid file path should succeed: {:?}", filename);

            // Result should be the canonical path to the file
            if let Ok(canonical) = result {
                let expected_canonical = test_file.canonicalize().unwrap();
                prop_assert_eq!(canonical, expected_canonical);
            }
        }

        /// Paths with leading slash are handled correctly
        #[test]
        fn prop_validate_media_path_handles_leading_slash(
            filename in "[a-zA-Z0-9_-]{1,15}"
        ) {
            let temp_dir = tempfile::tempdir().unwrap();

            // Create a test file
            let test_file = temp_dir.path().join(&filename);
            std::fs::write(&test_file, "test content").unwrap();

            // Test with leading slash
            let path_with_slash = format!("/{}", filename);
            let result = validate_media_path(&path_with_slash, temp_dir.path(), "");
            prop_assert!(result.is_ok(), "Path with leading slash should work: {:?}", path_with_slash);

            // Test without leading slash
            let result_no_slash = validate_media_path(&filename, temp_dir.path(), "");
            prop_assert!(result_no_slash.is_ok(), "Path without leading slash should work: {:?}", filename);

            // Both should resolve to the same canonical path
            if let (Ok(p1), Ok(p2)) = (result, result_no_slash) {
                prop_assert_eq!(p1, p2, "Leading slash should not change result");
            }
        }
    }
}
