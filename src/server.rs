use axum::{
    Router,
    body::Body,
    extract::{self, OriginalUri, State, ws::WebSocketUpgrade},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use percent_encoding::percent_decode_str;
use std::{net::SocketAddr, path::Path, path::PathBuf, sync::Arc};
use tokio::sync::broadcast;

use crate::config::{SortField, TagSource};
use crate::embedded_katex;
use crate::embedded_pico;
use crate::errors::{MbrError, ServerError};
use crate::link_grep::InboundLinkCache;
use crate::link_index::{LinkCache, resolve_outbound_links};
use crate::link_transform::LinkTransformConfig;
use crate::oembed_cache::OembedCache;
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
use tower::ServiceExt;
use tower_http::{compression::CompressionLayer, services::ServeFile, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

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
#[cfg(feature = "media-metadata")]
fn validate_path_containment(file_path: &Path, base_dir: &Path) -> Option<PathBuf> {
    // Early rejection of obvious path traversal in the path string
    if file_path.to_string_lossy().contains("..") {
        tracing::warn!("Path traversal attempt blocked: {}", file_path.display());
        return None;
    }

    let canonical_base = base_dir.canonicalize().ok()?;
    let canonical_file = file_path.canonicalize().ok()?;

    if canonical_file.starts_with(&canonical_base) && canonical_file.is_file() {
        Some(canonical_file)
    } else {
        None
    }
}

pub struct Server {
    pub router: Router,
    pub port: u16,
    pub ip: [u8; 4],
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
    pub template_folder: Option<std::path::PathBuf>,
    pub sort: Vec<SortField>,
    pub gui_mode: bool,
    pub theme: String,
    pub log_filter: Option<String>,
    pub link_tracking: bool,
    pub tag_sources: Vec<TagSource>,
    pub sidebar_style: String,
    pub sidebar_max_items: usize,
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
            template_folder: config.template_folder.clone(),
            sort: config.sort.clone(),
            gui_mode: false, // Default to server mode
            theme: config.theme.clone(),
            log_filter: None, // Set via with_log_filter()
            link_tracking: config.link_tracking,
            tag_sources: config.tag_sources.clone(),
            sidebar_style: config.sidebar_style.clone(),
            sidebar_max_items: config.sidebar_max_items,
            #[cfg(feature = "media-metadata")]
            transcode_enabled: config.transcode,
        }
    }
}

#[derive(Clone)]
pub struct ServerState {
    pub base_dir: std::path::PathBuf,
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
    /// Whether bidirectional link tracking is enabled
    pub link_tracking: bool,
    /// Cache for outbound links extracted during page renders
    pub link_cache: Arc<LinkCache>,
    /// Cache for inbound links discovered via grep
    pub inbound_link_cache: Arc<InboundLinkCache>,
    /// Tag sources for frontmatter extraction
    pub tag_sources: Vec<TagSource>,
    /// Sidebar navigation style ("panel" for mbr-browse, "single" for mbr-browse-single)
    pub sidebar_style: String,
    /// Maximum items per section in sidebar navigation
    pub sidebar_max_items: usize,
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
            template_folder,
            sort,
            gui_mode,
            theme,
            log_filter,
            link_tracking,
            tag_sources,
            sidebar_style,
            sidebar_max_items,
            #[cfg(feature = "media-metadata")]
            transcode_enabled,
        } = config;

        let oembed_cache = Arc::new(OembedCache::new(oembed_cache_size));

        // Initialize video metadata cache with same size as oembed cache
        #[cfg(feature = "media-metadata")]
        let video_metadata_cache = Arc::new(VideoMetadataCache::new(oembed_cache_size));

        // Initialize HLS cache (200MB default size for playlists and segments)
        #[cfg(feature = "media-metadata")]
        let hls_cache = Arc::new(HlsCache::new(200 * 1024 * 1024));

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
        ));

        // Create a broadcast channel for file changes - watcher will be initialized in background
        let (file_change_tx, _rx) =
            tokio::sync::broadcast::channel::<crate::watcher::FileChangeEvent>(100);
        let tx_for_watcher = file_change_tx.clone();

        // Initialize file watcher in background to avoid blocking server startup
        // PollWatcher's recursive scan can take 10+ seconds for large directories
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
        let template_folder_for_reload = template_folder.clone();
        let mut template_change_rx = file_change_tx.subscribe();
        tokio::spawn(async move {
            while let Ok(event) = template_change_rx.recv().await {
                // Only reload for .html files
                if !event.path.ends_with(".html") {
                    continue;
                }

                // If we have a template folder, only reload for changes in that folder
                // Otherwise, only reload for changes in .mbr folder
                let should_reload = if let Some(ref tf) = template_folder_for_reload {
                    event.path.starts_with(&tf.to_string_lossy().to_string())
                } else {
                    event.path.contains("/.mbr/")
                };

                if should_reload {
                    tracing::debug!("Template file changed: {}", event.path);
                    if let Err(e) = templates_for_reload.reload() {
                        tracing::error!("Failed to reload templates: {}", e);
                    }
                }
            }
        });

        // Spawn background task to invalidate repo cache when files change
        let repo_for_invalidation = Arc::clone(&repo);
        let markdown_extensions_for_invalidation = markdown_extensions.clone();
        let mut repo_change_rx = file_change_tx.subscribe();
        tokio::spawn(async move {
            while let Ok(event) = repo_change_rx.recv().await {
                // Invalidate cache for:
                // - Created files (new file added)
                // - Deleted files (file removed)
                // - Modified markdown files (frontmatter may have changed)
                let should_invalidate = match event.event {
                    crate::watcher::ChangeEventType::Created => true,
                    crate::watcher::ChangeEventType::Deleted => true,
                    crate::watcher::ChangeEventType::Modified => {
                        // Only invalidate for markdown files (frontmatter changes)
                        markdown_extensions_for_invalidation
                            .iter()
                            .any(|ext| event.relative_path.ends_with(&format!(".{}", ext)))
                    }
                };

                if should_invalidate {
                    tracing::debug!(
                        "Invalidating repo cache due to file change: {:?}",
                        event.relative_path
                    );
                    repo_for_invalidation.clear();
                }
            }
        });

        // Initialize link caches (use same size strategy as oembed cache)
        // 2MB for outbound links, 1MB for inbound with 60 second TTL
        let link_cache = Arc::new(LinkCache::new(2 * 1024 * 1024));
        let inbound_link_cache = Arc::new(InboundLinkCache::new(1024 * 1024, 60));

        let state = ServerState {
            base_dir,
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
            link_tracking,
            link_cache,
            inbound_link_cache,
            tag_sources,
            sidebar_style,
            sidebar_max_items,
        };

        let router = Router::new()
            .route("/", get(Self::home_page))
            .route("/.mbr/site.json", get(Self::get_site_info))
            .route("/.mbr/search", post(Self::search_handler))
            .route("/.mbr/ws/changes", get(Self::websocket_handler))
            // Media viewer routes - must be before the catch-all /.mbr/{*path}
            .route("/.mbr/videos/", get(Self::serve_media_viewer))
            .route("/.mbr/pdfs/", get(Self::serve_media_viewer))
            .route("/.mbr/audio/", get(Self::serve_media_viewer))
            .route("/.mbr/images/", get(Self::serve_media_viewer))
            .route("/.mbr/{*path}", get(Self::serve_mbr_assets))
            .route("/{*path}", get(Self::handle))
            .layer(CompressionLayer::new())
            .layer(TraceLayer::new_for_http())
            .with_state(state);

        Ok(Server {
            router,
            ip,
            port,
            _watcher_handle: watcher_handle,
        })
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
        tracing::debug!("listening on {}", local_addr);
        println!("Server running at http://{}/", local_addr);

        // Signal that server is ready before starting to serve
        if let Some(tx) = ready_tx
            && tx.send(()).is_err()
        {
            tracing::debug!("Ready signal receiver dropped (shutdown in progress)");
        }

        axum::serve(listener, self.router.clone())
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
                    tracing::debug!("listening on {}", local_addr);
                    println!("Server running at http://{}/", local_addr);

                    // Signal that server is ready with the actual port
                    if let Some(tx) = ready_tx
                        && tx.send(self.port).is_err()
                    {
                        tracing::debug!("Port signal receiver dropped (shutdown in progress)");
                    }

                    axum::serve(listener, self.router.clone())
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
    pub async fn websocket_handler(
        ws: WebSocketUpgrade,
        State(config): State<ServerState>,
    ) -> impl IntoResponse {
        ws.on_upgrade(|socket| Self::handle_websocket(socket, config))
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
                // Forward file change events to the client
                Ok(change_event) = rx.recv() => {
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

    pub async fn get_site_info(
        State(config): State<ServerState>,
    ) -> Result<impl IntoResponse, StatusCode> {
        config
            .repo
            .scan_all()
            .inspect_err(|e| tracing::error!("Error scanning repo: {e}"))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // Build combined response with repo data and config
        let mut response = serde_json::to_value(&*config.repo)
            .inspect_err(|e| tracing::error!("Error creating json: {e}"))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // Add sort config to the response
        if let Some(obj) = response.as_object_mut() {
            obj.insert(
                "sort".to_string(),
                serde_json::to_value(&config.sort).unwrap_or(serde_json::Value::Array(vec![])),
            );
            // Add sidebar navigation configuration
            obj.insert(
                "sidebar_style".to_string(),
                serde_json::Value::String(config.sidebar_style.clone()),
            );
            obj.insert(
                "sidebar_max_items".to_string(),
                serde_json::json!(config.sidebar_max_items),
            );
        }

        let resp = Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(
                serde_json::to_string(&response)
                    .inspect_err(|e| tracing::error!("Error serializing json: {e}"))
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
            )
            .inspect_err(|e| tracing::error!("Error rendering site file: {e}"))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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

        // Ensure repo is scanned (may already be from background scan)
        if let Err(e) = config.repo.scan_all() {
            tracing::error!("Error scanning repo for search: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to scan repository: {}", e),
                    "query": query.q,
                    "total_matches": 0,
                    "results": [],
                    "duration_ms": 0
                })),
            );
        }

        // Create search engine and execute search
        let engine = SearchEngine::new(config.repo.clone(), config.base_dir.clone());

        let mut response = match engine.search(&query) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("Search error: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": format!("Search failed: {}", e),
                        "query": query.q,
                        "total_matches": 0,
                        "results": [],
                        "duration_ms": 0
                    })),
                );
            }
        };

        // If searching all filetypes or non-markdown, also search other files
        if query.filetype.as_deref() == Some("all")
            || (query.filetype.is_some() && query.filetype.as_deref() != Some("markdown"))
        {
            let other_results = search_other_files(
                &config.repo,
                &query.q,
                query.folder.as_deref(),
                query.filetype.as_deref(),
                query.limit,
            );

            // Merge and re-sort
            response.results.extend(other_results);
            response.results.sort_by(|a, b| b.score.cmp(&a.score));
            response.results.truncate(query.limit);
            response.total_matches = response.results.len();
        }

        tracing::debug!(
            "Search completed: {} results in {}ms",
            response.total_matches,
            response.duration_ms
        );

        (
            StatusCode::OK,
            Json(serde_json::to_value(response).unwrap()),
        )
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
                    );
                }
            };

        // Extract title from filename
        let title = validated_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Media Viewer")
            .to_string();

        // Calculate relative path for display and parent path for navigation
        let relative_path = validated_path
            .strip_prefix(&config.base_dir)
            .unwrap_or(&validated_path);

        // Generate breadcrumbs from the media file's directory path
        let breadcrumbs =
            generate_breadcrumbs(relative_path.parent().unwrap_or(std::path::Path::new("")));
        let breadcrumbs_json: Vec<_> = breadcrumbs
            .iter()
            .map(|b| json!({"name": b.name, "url": b.url}))
            .collect();

        // Get parent path for back navigation
        let parent_path = relative_path.parent().and_then(|p| p.to_str()).map(|p| {
            if p.is_empty() {
                "/".to_string()
            } else {
                format!("/{}/", p)
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
        context.insert("server_mode".to_string(), json!(true));
        context.insert("gui_mode".to_string(), json!(config.gui_mode));
        context.insert("relative_base".to_string(), json!("/.mbr/"));
        context.insert("sidebar_style".to_string(), json!(config.sidebar_style));
        context.insert(
            "sidebar_max_items".to_string(),
            json!(config.sidebar_max_items),
        );

        // Render the media viewer template
        match config.templates.render_media_viewer(context) {
            Ok(html) => {
                let etag = generate_etag(html.as_bytes());
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                    .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_STORE)
                    .header(header::ETAG, etag)
                    .body(Body::from(html))
                    .unwrap_or_else(|_| {
                        Response::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .body(Body::from("Internal Server Error"))
                            .unwrap()
                    })
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
    /// that resolved paths remain within the intended directory.
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
        let mbr_dir = config.base_dir.join(".mbr");
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
    ) -> Response<Body> {
        use std::collections::HashMap;

        let mut context: HashMap<String, serde_json::Value> = HashMap::new();
        context.insert(
            "error_code".to_string(),
            serde_json::Value::Number(status_code.as_u16().into()),
        );
        context.insert(
            "error_title".to_string(),
            serde_json::Value::String(error_title.to_string()),
        );
        if let Some(msg) = error_message {
            context.insert(
                "error_message".to_string(),
                serde_json::Value::String(msg.to_string()),
            );
        }
        context.insert(
            "requested_url".to_string(),
            serde_json::Value::String(requested_url.to_string()),
        );
        // Server mode uses absolute paths
        context.insert("server_mode".to_string(), serde_json::Value::Bool(true));
        context.insert("gui_mode".to_string(), serde_json::Value::Bool(gui_mode));
        // Sidebar navigation configuration
        context.insert(
            "sidebar_style".to_string(),
            serde_json::Value::String(sidebar_style.to_string()),
        );
        context.insert(
            "sidebar_max_items".to_string(),
            serde_json::Value::Number(sidebar_max_items.into()),
        );

        match templates.render_error(context) {
            Ok(html) => Response::builder()
                .status(status_code)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .body(Body::from(html))
                .unwrap_or_else(|_| {
                    Response::builder()
                        .status(status_code)
                        .body(Body::from(error_title.to_string()))
                        .unwrap()
                }),
            Err(e) => {
                tracing::error!("Failed to render error page: {}", e);
                Response::builder()
                    .status(status_code)
                    .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                    .body(Body::from(format!(
                        "{} {}",
                        status_code.as_u16(),
                        error_title
                    )))
                    .unwrap()
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
            static_folder: &config.static_folder,
            markdown_extensions: &config.markdown_extensions,
            index_file: &config.index_file,
            tag_sources: &tag_url_sources,
        };

        match resolve_request_path(&resolver_config, &path) {
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
            ResolvedPath::Redirect(canonical_url) => {
                tracing::debug!("redirecting to canonical URL: {}", &canonical_url);
                Ok(Response::builder()
                    .status(StatusCode::MOVED_PERMANENTLY)
                    .header(header::LOCATION, &canonical_url)
                    .body(Body::empty())
                    .unwrap())
            }
            ResolvedPath::NotFound => {
                // Try to serve HLS content (playlist or segment) for transcoded variants
                #[cfg(feature = "media-metadata")]
                if config.transcode_enabled
                    && let Some(response) = Self::try_serve_hls_content(&path, &config).await
                {
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

    /// Try to serve dynamically generated video metadata (cover, chapters, captions).
    ///
    /// Returns Some(Response) if the request was for video metadata and we successfully
    /// generated it, None otherwise (fall through to 404).
    #[cfg(feature = "media-metadata")]
    async fn try_serve_video_metadata(path: &str, config: &ServerState) -> Option<Response<Body>> {
        use crate::video_metadata::{
            MetadataType, extract_captions, extract_chapters, extract_cover, parse_metadata_request,
        };
        use crate::video_metadata_cache::{CachedMetadata, cache_key};

        // Check if this is a video metadata request
        let (video_url_path, metadata_type) = parse_metadata_request(path)?;

        let cache_type_str = match metadata_type {
            MetadataType::Cover => "cover",
            MetadataType::Chapters => "chapters",
            MetadataType::Captions => "captions",
        };

        // Check cache first
        let key = cache_key(video_url_path, cache_type_str);
        if let Some(cached) = config.video_metadata_cache.get(&key) {
            return match cached {
                CachedMetadata::Cover(bytes) => Some(Self::build_jpg_response(bytes)),
                CachedMetadata::Chapters(vtt) | CachedMetadata::Captions(vtt) => {
                    Some(Self::build_vtt_response(vtt))
                }
                CachedMetadata::NotAvailable => None, // Cached negative result
            };
        }

        // Try to resolve the video file path with path traversal protection
        // First, try the direct path, then try with static_folder prefix
        let video_file = {
            let direct = config.base_dir.join(video_url_path);
            // Validate path stays within base_dir (defense in depth)
            if let Some(validated) = validate_path_containment(&direct, &config.base_dir) {
                validated
            } else {
                let static_dir = config.base_dir.join(&config.static_folder);
                let with_static = static_dir.join(video_url_path);
                // Validate path stays within static folder
                if let Some(validated) = validate_path_containment(&with_static, &static_dir) {
                    validated
                } else {
                    tracing::debug!(
                        "Video file not found for metadata generation: {}",
                        video_url_path
                    );
                    return None;
                }
            }
        };

        tracing::debug!(
            "Generating {} for: {}",
            cache_type_str,
            video_file.display()
        );

        // Generate the metadata
        match metadata_type {
            MetadataType::Cover => match extract_cover(&video_file) {
                Ok(bytes) => {
                    config
                        .video_metadata_cache
                        .insert(key, CachedMetadata::Cover(bytes.clone()));
                    Some(Self::build_jpg_response(bytes))
                }
                Err(e) => {
                    tracing::debug!("Failed to extract cover: {}", e);
                    config
                        .video_metadata_cache
                        .insert(key, CachedMetadata::NotAvailable);
                    None
                }
            },
            MetadataType::Chapters => match extract_chapters(&video_file) {
                Ok(vtt) => {
                    config
                        .video_metadata_cache
                        .insert(key, CachedMetadata::Chapters(vtt.clone()));
                    Some(Self::build_vtt_response(vtt))
                }
                Err(e) => {
                    tracing::debug!("Failed to extract chapters: {}", e);
                    config
                        .video_metadata_cache
                        .insert(key, CachedMetadata::NotAvailable);
                    None
                }
            },
            MetadataType::Captions => match extract_captions(&video_file) {
                Ok(vtt) => {
                    config
                        .video_metadata_cache
                        .insert(key, CachedMetadata::Captions(vtt.clone()));
                    Some(Self::build_vtt_response(vtt))
                }
                Err(e) => {
                    tracing::debug!("Failed to extract captions: {}", e);
                    config
                        .video_metadata_cache
                        .insert(key, CachedMetadata::NotAvailable);
                    None
                }
            },
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
        use crate::video_metadata_cache::{CachedMetadata, cache_key};

        // Check if this is a PDF cover request
        let pdf_url_path = parse_pdf_cover_request(url_path)?;

        // Build cache key
        let key = cache_key(pdf_url_path, "pdf_cover");

        // Check memory cache first
        if let Some(cached) = config.video_metadata_cache.get(&key) {
            return match cached {
                CachedMetadata::Cover(bytes) => Some(Self::build_jpg_response(bytes)),
                CachedMetadata::NotAvailable => None, // Cached negative result
                _ => None,                            // Other types not relevant for PDF covers
            };
        }

        // Find the PDF file path (corresponding to this sidecar)
        // The sidecar is at {pdf_path}.cover.jpg, so remove .cover.jpg to get pdf_path
        let pdf_file = {
            let sidecar_str = sidecar_file_path.to_str()?;
            let pdf_path_str = sidecar_str.strip_suffix(".cover.jpg")?;
            std::path::PathBuf::from(pdf_path_str)
        };

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
        use crate::video_metadata_cache::{CachedMetadata, cache_key};

        // Check if this is a PDF cover request
        let pdf_url_path = parse_pdf_cover_request(path)?;

        // Build cache key
        let key = cache_key(pdf_url_path, "pdf_cover");

        // Check memory cache first
        if let Some(cached) = config.video_metadata_cache.get(&key) {
            return match cached {
                CachedMetadata::Cover(bytes) => Some(Self::build_jpg_response(bytes)),
                CachedMetadata::NotAvailable => None, // Cached negative result
                _ => None,                            // Other types not relevant for PDF covers
            };
        }

        // Try to resolve the PDF file path with path traversal protection
        // First, try the direct path, then try with static_folder prefix
        let pdf_file = {
            let direct = config.base_dir.join(pdf_url_path);
            // Validate path stays within base_dir (defense in depth)
            if let Some(validated) = validate_path_containment(&direct, &config.base_dir) {
                validated
            } else {
                let static_dir = config.base_dir.join(&config.static_folder);
                let with_static = static_dir.join(pdf_url_path);
                // Validate path stays within static folder
                if let Some(validated) = validate_path_containment(&with_static, &static_dir) {
                    validated
                } else {
                    tracing::debug!("PDF file not found for cover generation: {}", pdf_url_path);
                    return None;
                }
            }
        };

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
        use crate::link_grep::find_inbound_links;
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
        // e.g., "docs/guide/links.json" -> "/docs/guide/"
        let page_path = path.strip_suffix("links.json")?;
        let page_url_path = if page_path.is_empty() || page_path == "/" {
            "/".to_string()
        } else {
            let normalized = page_path.trim_end_matches('/');
            format!("{}/", normalized)
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
                    )
                    .await
                    {
                        Ok(render_result) => {
                            // Resolve relative URLs to absolute before caching
                            let resolved_links = resolve_outbound_links(
                                &page_url_path,
                                render_result.outbound_links,
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

        // Get inbound links from cache or grep
        let inbound = if let Some(cached) = config.inbound_link_cache.get(&page_url_path) {
            cached
        } else {
            // Grep for inbound links
            let links = find_inbound_links(
                &page_url_path,
                &config.base_dir,
                &config.markdown_extensions,
                &config.ignore_dirs,
                &config.ignore_globs,
            );
            // Cache the result
            config
                .inbound_link_cache
                .insert(page_url_path.clone(), links.clone());
            links
        };

        let page_links = PageLinks { inbound, outbound };

        let json = match serde_json::to_string(&page_links) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!("Failed to serialize links.json: {}", e);
                return None;
            }
        };

        Some(
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_CACHE)
                .body(Body::from(json))
                .unwrap(),
        )
    }

    /// Build a JPEG image response.
    #[cfg(feature = "media-metadata")]
    fn build_jpg_response(bytes: Vec<u8>) -> Response<Body> {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "image/jpeg")
            .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_CACHE)
            .body(Body::from(bytes))
            .unwrap()
    }

    /// Build a WebVTT response.
    #[cfg(feature = "media-metadata")]
    fn build_vtt_response(vtt: String) -> Response<Body> {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/vtt; charset=utf-8")
            .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_CACHE)
            .body(Body::from(vtt))
            .unwrap()
    }

    /// Build an HLS playlist response.
    #[cfg(feature = "media-metadata")]
    fn build_hls_playlist_response(playlist: Arc<Vec<u8>>) -> Response<Body> {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")
            .header(header::CONTENT_LENGTH, playlist.len())
            .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_CACHE)
            .body(Body::from(playlist.as_ref().clone()))
            .unwrap()
    }

    /// Build an HLS segment response.
    #[cfg(feature = "media-metadata")]
    fn build_hls_segment_response(segment: Arc<Vec<u8>>) -> Response<Body> {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "video/mp2t")
            .header(header::CONTENT_LENGTH, segment.len())
            .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_CACHE)
            .body(Body::from(segment.as_ref().clone()))
            .unwrap()
    }

    /// Try to serve HLS content (playlist or segment) for transcoded video variants.
    ///
    /// Returns Some(Response) if the request was for HLS content and we
    /// successfully served it, None otherwise (fall through to other handlers).
    #[cfg(feature = "media-metadata")]
    async fn try_serve_hls_content(path: &str, config: &ServerState) -> Option<Response<Body>> {
        use crate::video_transcode::{
            HlsRequest, TranscodeError, generate_hls_playlist, parse_hls_request,
            probe_video_resolution, should_transcode, transcode_segment,
        };
        use crate::video_transcode_cache::{HlsCacheKey, HlsCacheStartResult, HlsCacheState};

        // Helper to build error response for transcode errors
        fn build_transcode_error_response(error: &TranscodeError) -> Option<Response<Body>> {
            match error {
                TranscodeError::SourceTooSmall {
                    source_height,
                    target_height,
                } => Some(
                    Response::builder()
                        .status(StatusCode::UNPROCESSABLE_ENTITY)
                        .header(header::CONTENT_TYPE, "text/plain")
                        .body(Body::from(format!(
                            "Cannot transcode: source ({}p) not larger than target ({}p)",
                            source_height, target_height
                        )))
                        .unwrap(),
                ),
                TranscodeError::SegmentOutOfRange {
                    segment_index,
                    video_duration,
                } => Some(
                    Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .header(header::CONTENT_TYPE, "text/plain")
                        .body(Body::from(format!(
                            "Segment {} is out of range (video duration: {:.1}s)",
                            segment_index, video_duration
                        )))
                        .unwrap(),
                ),
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

        // Resolve the original video file path
        let video_file = {
            let direct = config.base_dir.join(&video_path);
            if direct.is_file() {
                direct
            } else {
                let with_static = config
                    .base_dir
                    .join(&config.static_folder)
                    .join(&video_path);
                if with_static.is_file() {
                    with_static
                } else {
                    tracing::debug!("Original video file not found for HLS: {}", video_path);
                    return None;
                }
            }
        };

        // Check if we should transcode (only downscale, never upscale)
        let resolution = probe_video_resolution(&video_file).ok()?;
        if !should_transcode(resolution.height, target) {
            tracing::debug!(
                "Video already at or below target resolution: {}x{} <= {}",
                resolution.width,
                resolution.height,
                target.height()
            );
            // Return 422 instead of None (404) with helpful message
            return Some(
                Response::builder()
                    .status(StatusCode::UNPROCESSABLE_ENTITY)
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from(format!(
                        "Cannot transcode: source ({}p) not larger than target ({}p)",
                        resolution.height,
                        target.height()
                    )))
                    .unwrap(),
            );
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
                                notify.notify_waiters();
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
                    HlsCacheStartResult::AlreadyInProgress(notify) => {
                        tracing::debug!("Waiting for in-progress playlist generation");
                        notify.notified().await;

                        match config.hls_cache.get_state(&cache_key) {
                            Some(HlsCacheState::Complete(data)) => {
                                return Some(Self::build_hls_playlist_response(data));
                            }
                            _ => return None,
                        }
                    }
                    HlsCacheStartResult::AlreadyComplete(data) => {
                        tracing::debug!("Serving cached playlist");
                        return Some(Self::build_hls_playlist_response(data));
                    }
                    HlsCacheStartResult::PreviouslyFailed(msg) => {
                        tracing::debug!("Previous playlist generation failed: {}", msg);
                        // Return 422 with cached error message instead of None (404)
                        return Some(
                            Response::builder()
                                .status(StatusCode::UNPROCESSABLE_ENTITY)
                                .header(header::CONTENT_TYPE, "text/plain")
                                .body(Body::from(format!("Transcode failed: {}", msg)))
                                .unwrap(),
                        );
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
                                notify.notify_waiters();
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
                    HlsCacheStartResult::AlreadyInProgress(notify) => {
                        tracing::debug!("Waiting for in-progress segment transcode");
                        notify.notified().await;

                        match config.hls_cache.get_state(&cache_key) {
                            Some(HlsCacheState::Complete(data)) => {
                                return Some(Self::build_hls_segment_response(data));
                            }
                            _ => return None,
                        }
                    }
                    HlsCacheStartResult::AlreadyComplete(data) => {
                        tracing::debug!("Serving cached segment");
                        return Some(Self::build_hls_segment_response(data));
                    }
                    HlsCacheStartResult::PreviouslyFailed(msg) => {
                        tracing::debug!("Previous segment transcode failed: {}", msg);
                        // Return 422 with cached error message instead of None (404)
                        return Some(
                            Response::builder()
                                .status(StatusCode::UNPROCESSABLE_ENTITY)
                                .header(header::CONTENT_TYPE, "text/plain")
                                .body(Body::from(format!("Transcode failed: {}", msg)))
                                .unwrap(),
                        );
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
    ) -> Result<Response<Body>, Box<dyn std::error::Error>> {
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
        )
        .await
        .inspect_err(|e| tracing::error!("Error rendering markdown: {e}"))?;
        let mut frontmatter = render_result.frontmatter;
        let headings = render_result.headings;
        let inner_html_output = render_result.html;
        let outbound_links = render_result.outbound_links;
        let has_h1 = render_result.has_h1;
        let word_count = render_result.word_count;
        // Use relative path for markdown_source so live reload can match it
        let relative_md_path =
            pathdiff::diff_paths(md_path, root_path).unwrap_or_else(|| md_path.to_path_buf());
        frontmatter.insert(
            "markdown_source".into(),
            relative_md_path.to_string_lossy().into(),
        );
        // Indicate server mode for frontend search functionality
        frontmatter.insert("server_mode".into(), "true".into());
        // Indicate GUI mode for native window detection
        frontmatter.insert(
            "gui_mode".into(),
            if config.gui_mode { "true" } else { "" }.into(),
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

        // Cache outbound links for links.json endpoint if link tracking is enabled
        if config.link_tracking && !outbound_links.is_empty() {
            let url_path_str = format!("/{}/", url_path_buf.display()).replace("//", "/");
            // Resolve relative URLs to absolute before caching
            let resolved_links = resolve_outbound_links(&url_path_str, outbound_links);
            config.link_cache.insert(url_path_str, resolved_links);
        }

        let breadcrumbs = generate_breadcrumbs(&url_path_buf);
        let breadcrumbs_json: Vec<_> = breadcrumbs
            .iter()
            .map(|b| serde_json::json!({"name": b.name, "url": b.url}))
            .collect();
        let current_dir_name = get_current_dir_name(&url_path_buf);

        // Build extra context for navigation elements, heading TOC, and config
        let mut extra_context = std::collections::HashMap::new();
        extra_context.insert(
            "breadcrumbs".to_string(),
            serde_json::json!(breadcrumbs_json),
        );
        extra_context.insert(
            "current_dir_name".to_string(),
            serde_json::json!(current_dir_name),
        );
        extra_context.insert("headings".to_string(), serde_json::json!(headings));
        extra_context.insert("has_h1".to_string(), serde_json::json!(has_h1));

        // Pass tag sources configuration for frontend tag linking
        // Pre-serialize as JSON string for safe template rendering in JavaScript context
        let tag_sources_json = serde_json::to_string(
            &config
                .tag_sources
                .iter()
                .map(|ts| {
                    serde_json::json!({
                        "field": ts.field,
                        "urlSource": ts.url_source(),
                        "label": ts.singular_label(),
                        "labelPlural": ts.plural_label()
                    })
                })
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_string());
        extra_context.insert(
            "tag_sources".to_string(),
            serde_json::json!(tag_sources_json),
        );

        // Pass word count and reading time (200 words per minute)
        let reading_time_minutes = word_count.div_ceil(200);
        extra_context.insert("word_count".to_string(), serde_json::json!(word_count));
        extra_context.insert(
            "reading_time_minutes".to_string(),
            serde_json::json!(reading_time_minutes),
        );

        // Pass file path (relative to root) for reference
        extra_context.insert(
            "file_path".to_string(),
            serde_json::json!(relative_md_path.to_string_lossy()),
        );
        // Pass sidebar navigation configuration
        extra_context.insert(
            "sidebar_style".to_string(),
            serde_json::json!(config.sidebar_style),
        );
        extra_context.insert(
            "sidebar_max_items".to_string(),
            serde_json::json!(config.sidebar_max_items),
        );

        // Pass modified date from file metadata
        let modified_info = tokio::fs::metadata(md_path)
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
        if let Some(duration) = modified_info {
            extra_context.insert(
                "modified_timestamp".to_string(),
                serde_json::json!(duration.as_secs()),
            );
        }

        // Compute prev/next sibling pages for navigation
        let current_url = format!("/{}/", url_path_buf.display()).replace("//", "/");
        let parent_dir = relative_md_path.parent().unwrap_or(Path::new(""));

        // Get sibling markdown files in the same directory
        let mut siblings: Vec<_> = config
            .repo
            .markdown_files
            .pin()
            .iter()
            .filter_map(|(_, info)| {
                let file_parent = info.raw_path.parent()?;
                if file_parent == parent_dir {
                    Some(markdown_file_to_json(info))
                } else {
                    None
                }
            })
            .collect();

        // Sort siblings using configured sort order
        sort_files(&mut siblings, &config.sort);

        // Find current position and get prev/next
        if let Some(current_idx) = siblings.iter().position(|f| {
            f.get("url_path")
                .and_then(|v| v.as_str())
                .is_some_and(|p| p == current_url)
        }) {
            if current_idx > 0
                && let Some(prev) = siblings.get(current_idx - 1)
            {
                extra_context.insert(
                    "prev_page".to_string(),
                    serde_json::json!({
                        "url": prev.get("url_path"),
                        "title": prev.get("title").and_then(|v| v.as_str()).unwrap_or("Previous")
                    }),
                );
            }
            if let Some(next) = siblings.get(current_idx + 1) {
                extra_context.insert(
                    "next_page".to_string(),
                    serde_json::json!({
                        "url": next.get("url_path"),
                        "title": next.get("title").and_then(|v| v.as_str()).unwrap_or("Next")
                    }),
                );
            }
        }

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
            .map_err(|e| e.into())
    }

    async fn directory_to_html(
        dir_path: &Path,
        templates: &crate::templates::Templates,
        root_path: &Path,
        config: &ServerState,
    ) -> Result<Response<Body>, Box<dyn std::error::Error>> {
        use serde_json::json;

        // Create a temporary repo instance to scan this directory
        let temp_repo = Repo::init(
            root_path,
            &config.static_folder,
            &config.markdown_extensions,
            &config.ignore_dirs,
            &config.ignore_globs,
            &config.index_file,
            &config.tag_sources,
        );

        // Calculate relative path from root
        let relative_path = pathdiff::diff_paths(dir_path, root_path)
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        // Scan this directory only (non-recursive)
        temp_repo
            .scan_folder(&relative_path)
            .inspect_err(|e| tracing::error!("Error scanning directory: {e}"))?;

        // Extract markdown files and transform to JSON using helper
        let mut files: Vec<_> = temp_repo
            .markdown_files
            .pin()
            .iter()
            .map(|(_, file_info)| markdown_file_to_json(file_info))
            .collect();

        // Sort files using configurable sort order
        sort_files(&mut files, &config.sort);

        // Extract subdirectories
        let subdirs: Vec<_> = temp_repo
            .queued_folders
            .pin()
            .iter()
            .filter_map(|(abs_path, rel_path)| {
                // Only include immediate children
                let parent = abs_path.parent()?;
                if parent == dir_path {
                    let name = abs_path.file_name()?.to_str()?.to_string();
                    let mut url_path = rel_path.to_str()?.to_string();
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

        // Use helper functions for navigation elements
        let breadcrumbs = generate_breadcrumbs(&relative_path);
        let breadcrumbs_json: Vec<_> = breadcrumbs
            .iter()
            .map(|b| json!({"name": b.name, "url": b.url}))
            .collect();

        let current_dir_name = get_current_dir_name(&relative_path);
        let parent_path = get_parent_path(&relative_path);

        // Build context
        let mut context = std::collections::HashMap::new();
        context.insert("files".to_string(), json!(files));
        context.insert("subdirs".to_string(), json!(subdirs));
        context.insert("breadcrumbs".to_string(), json!(breadcrumbs_json));
        context.insert("current_dir_name".to_string(), json!(current_dir_name));
        context.insert(
            "current_path".to_string(),
            json!(relative_path.to_string_lossy()),
        );
        if let Some(parent) = parent_path {
            context.insert("parent_path".to_string(), json!(parent));
        }
        // Indicate server mode for frontend search functionality
        context.insert("server_mode".to_string(), json!(true));
        // Indicate GUI mode for native window detection
        context.insert("gui_mode".to_string(), json!(config.gui_mode));

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
        // Pre-serialize as JSON string for safe template rendering in JavaScript context
        let tag_sources_json = serde_json::to_string(
            &config
                .tag_sources
                .iter()
                .map(|ts| {
                    json!({
                        "field": ts.field,
                        "urlSource": ts.url_source(),
                        "label": ts.singular_label(),
                        "labelPlural": ts.plural_label()
                    })
                })
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_string());
        context.insert("tag_sources".to_string(), json!(tag_sources_json));

        // Pass sidebar navigation configuration
        context.insert("sidebar_style".to_string(), json!(config.sidebar_style));
        context.insert(
            "sidebar_max_items".to_string(),
            json!(config.sidebar_max_items),
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
            .map_err(|e| e.into())
    }

    /// Renders a tag page showing all pages with a specific tag value.
    async fn tag_page_to_html(
        source: &str,
        value: &str,
        config: &ServerState,
    ) -> Result<Response<Body>, Box<dyn std::error::Error>> {
        use serde_json::json;

        // Find the TagSource config to get labels
        let tag_source = config.tag_sources.iter().find(|s| s.url_source() == source);

        let (label, label_plural) = if let Some(ts) = tag_source {
            (ts.singular_label(), ts.plural_label())
        } else {
            // Fallback to capitalized source name
            let capitalized = source
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default()
                + &source[1..];
            (capitalized.clone(), format!("{}s", capitalized))
        };

        // Get pages with this tag from the index
        let pages = config.repo.tag_index.get_pages(source, value);

        // Get display value for the tag
        let display_value = config
            .repo
            .tag_index
            .get_tag_display(source, value)
            .unwrap_or_else(|| value.to_string());

        // Convert pages to JSON objects
        let pages_json: Vec<serde_json::Value> = pages
            .iter()
            .map(|page| {
                json!({
                    "url_path": page.url_path,
                    "title": page.title,
                    "description": page.description,
                })
            })
            .collect();

        // Build template context
        let mut context = std::collections::HashMap::new();
        context.insert("tag_source".to_string(), json!(source));
        context.insert("tag_display_value".to_string(), json!(display_value));
        context.insert("tag_label".to_string(), json!(label));
        context.insert("tag_label_plural".to_string(), json!(label_plural));
        context.insert("pages".to_string(), json!(pages_json));
        context.insert("page_count".to_string(), json!(pages.len()));
        context.insert("server_mode".to_string(), json!(true));
        context.insert("relative_base".to_string(), json!("/.mbr/"));
        // Pass sidebar navigation configuration
        context.insert("sidebar_style".to_string(), json!(config.sidebar_style));
        context.insert(
            "sidebar_max_items".to_string(),
            json!(config.sidebar_max_items),
        );

        let html_output = config.templates.render_tag(context)?;

        let etag = generate_etag(html_output.as_bytes());

        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_STORE)
            .header(header::ETAG, etag)
            .body(Body::from(html_output))
            .map_err(|e| e.into())
    }

    /// Renders a tag source index showing all tags from a source.
    async fn tag_source_index_to_html(
        source: &str,
        config: &ServerState,
    ) -> Result<Response<Body>, Box<dyn std::error::Error>> {
        use serde_json::json;

        // Find the TagSource config to get labels
        let tag_source = config.tag_sources.iter().find(|s| s.url_source() == source);

        let (label, label_plural) = if let Some(ts) = tag_source {
            (ts.singular_label(), ts.plural_label())
        } else {
            // Fallback to capitalized source name
            let capitalized = source
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default()
                + &source[1..];
            (capitalized.clone(), format!("{}s", capitalized))
        };

        // Get all tags for this source
        let tags = config.repo.tag_index.get_all_tags(source);

        // Convert tags to JSON objects
        let tags_json: Vec<serde_json::Value> = tags
            .iter()
            .map(|tag| {
                json!({
                    "url_value": tag.normalized,
                    "display_value": tag.display,
                    "page_count": tag.count,
                })
            })
            .collect();

        // Build template context
        let mut context = std::collections::HashMap::new();
        context.insert("tag_source".to_string(), json!(source));
        context.insert("tag_label".to_string(), json!(label));
        context.insert("tag_label_plural".to_string(), json!(label_plural));
        context.insert("tags".to_string(), json!(tags_json));
        context.insert("tag_count".to_string(), json!(tags.len()));
        context.insert("server_mode".to_string(), json!(true));
        context.insert("relative_base".to_string(), json!("/.mbr/"));
        // Pass sidebar navigation configuration
        context.insert("sidebar_style".to_string(), json!(config.sidebar_style));
        context.insert(
            "sidebar_max_items".to_string(),
            json!(config.sidebar_max_items),
        );

        let html_output = config.templates.render_tag_index(context)?;

        let etag = generate_etag(html_output.as_bytes());

        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, CACHE_CONTROL_NO_STORE)
            .header(header::ETAG, etag)
            .body(Body::from(html_output))
            .map_err(|e| e.into())
    }

    /// Handler for the root path "/" - renders the home page using the same
    /// logic as other directories but with the home.html template.
    async fn home_page(State(config): State<ServerState>) -> Result<impl IntoResponse, StatusCode> {
        tracing::debug!("home_page handler");

        let tag_url_sources = crate::config::tag_sources_to_url_sources(&config.tag_sources);
        let resolver_config = PathResolverConfig {
            base_dir: config.base_dir.as_path(),
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
        let partial_path: std::path::PathBuf = path_components.iter().take(idx + 1).collect();
        let url = format!("/{}/", partial_path.to_string_lossy());
        let name = path_components[idx].to_string();
        breadcrumbs.push(Breadcrumb::new(name, url));
    }

    breadcrumbs
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
        let parent: std::path::PathBuf = path_components
            .iter()
            .take(path_components.len() - 1)
            .collect();
        Some(format!("/{}/", parent.to_string_lossy()))
    } else if !path_components.is_empty() {
        Some("/".to_string())
    } else {
        None
    }
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

    let modified_date = chrono::DateTime::from_timestamp(file_info.modified as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    json!({
        "title": title,
        "url_path": file_info.url_path,
        "description": description,
        "tags": tags,
        "modified_date": modified_date,
        "modified": file_info.modified,
        "name": file_info.raw_path.file_name().and_then(|s| s.to_str()).unwrap_or(""),
    })
}

// ============================================================================
// Cache header helpers (extracted for testability and reuse)
// ============================================================================

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
        "/hljs.dark.css",
        include_bytes!("../templates/hljs.dark.11.11.1.css"),
        "text/css",
    ),
    (
        "/hljs.js",
        include_bytes!("../templates/hljs.11.11.1.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.css.js",
        include_bytes!("../templates/hljs.lang.css.11.11.1.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.javascript.js",
        include_bytes!("../templates/hljs.lang.javascript.11.11.1.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.typescript.js",
        include_bytes!("../templates/hljs.lang.typescript.11.11.1.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.rust.js",
        include_bytes!("../templates/hljs.lang.rust.11.11.1.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.python.js",
        include_bytes!("../templates/hljs.lang.python.11.11.1.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.bash.js",
        include_bytes!("../templates/hljs.lang.bash.11.11.1.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.java.js",
        include_bytes!("../templates/hljs.lang.java.11.11.1.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.scala.js",
        include_bytes!("../templates/hljs.lang.scala.11.11.1.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.go.js",
        include_bytes!("../templates/hljs.lang.go.11.11.1.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.ruby.js",
        include_bytes!("../templates/hljs.lang.ruby.11.11.1.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.nix.js",
        include_bytes!("../templates/hljs.lang.nix.11.11.1.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.json.js",
        include_bytes!("../templates/hljs.lang.json.11.11.1.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.yaml.js",
        include_bytes!("../templates/hljs.lang.yaml.11.11.1.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.xml.js",
        include_bytes!("../templates/hljs.lang.xml.11.11.1.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.sql.js",
        include_bytes!("../templates/hljs.lang.sql.11.11.1.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.dockerfile.js",
        include_bytes!("../templates/hljs.lang.dockerfile.11.11.1.js"),
        "application/javascript",
    ),
    (
        "/hljs.lang.markdown.js",
        include_bytes!("../templates/hljs.lang.markdown.11.11.1.js"),
        "application/javascript",
    ),
    (
        "/mermaid.min.js",
        include_bytes!("../templates/mermaid.11.12.2.min.js"),
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
    use std::collections::HashMap;
    use std::path::PathBuf;

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
    fn test_markdown_file_to_json_with_frontmatter() {
        let mut frontmatter = HashMap::new();
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
            raw_path: PathBuf::from("/root/test.md"),
            url_path: "/test/".to_string(),
            frontmatter: Some(frontmatter),
            created: 1699000000,
            modified: 1700000000,
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
            raw_path: PathBuf::from("/root/my-document.md"),
            url_path: "/my-document/".to_string(),
            frontmatter: None,
            created: 1699000000,
            modified: 1700000000,
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
        let mut frontmatter = HashMap::new();
        frontmatter.insert(
            "title".to_string(),
            serde_json::Value::String("Only Title".to_string()),
        );
        // No description or tags

        let file_info = MarkdownInfo {
            raw_path: PathBuf::from("/root/partial.md"),
            url_path: "/partial/".to_string(),
            frontmatter: Some(frontmatter),
            created: 1699000000,
            modified: 1700000000,
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
