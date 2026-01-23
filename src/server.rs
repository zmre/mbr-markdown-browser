use axum::{
    Router,
    body::Body,
    extract::{self, State, ws::WebSocketUpgrade},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use std::{net::SocketAddr, path::Path, sync::Arc};
use tokio::sync::broadcast;

use crate::config::{SortField, TagSource};
use crate::embedded_katex;
use crate::embedded_pico;
use crate::errors::ServerError;
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

pub struct Server {
    pub router: Router,
    pub port: u16,
    pub ip: [u8; 4],
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
}

impl Server {
    #[allow(clippy::too_many_arguments)]
    pub fn init<S: Into<String>, P: Into<std::path::PathBuf>>(
        ip: [u8; 4],
        port: u16,
        base_dir: P,
        static_folder: S,
        markdown_extensions: &[String],
        ignore_dirs: &[String],
        ignore_globs: &[String],
        watcher_ignore_dirs: &[String],
        index_file: S,
        oembed_timeout_ms: u64,
        oembed_cache_size: usize,
        template_folder: Option<std::path::PathBuf>,
        sort: Vec<SortField>,
        gui_mode: bool,
        theme: S,
        log_filter: Option<&str>,
        link_tracking: bool,
        tag_sources: &[TagSource],
        #[cfg(feature = "media-metadata")] transcode_enabled: bool,
    ) -> Result<Self, ServerError> {
        let base_dir = base_dir.into();
        let static_folder = static_folder.into();
        let index_file = index_file.into();
        let theme = theme.into();
        let oembed_cache = Arc::new(OembedCache::new(oembed_cache_size));

        // Initialize video metadata cache with same size as oembed cache
        #[cfg(feature = "media-metadata")]
        let video_metadata_cache = Arc::new(VideoMetadataCache::new(oembed_cache_size));

        // Initialize HLS cache (200MB default size for playlists and segments)
        #[cfg(feature = "media-metadata")]
        let hls_cache = Arc::new(HlsCache::new(200 * 1024 * 1024));

        // Use try_init to allow multiple server instances in tests
        // RUST_LOG env var takes precedence, then CLI flag, then default (warn)
        let default_filter = log_filter.unwrap_or("mbr=warn,tower_http=warn");
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
            markdown_extensions,
            ignore_dirs,
            ignore_globs,
            &index_file,
            tag_sources,
        ));

        // Create a broadcast channel for file changes - watcher will be initialized in background
        let (file_change_tx, _rx) =
            tokio::sync::broadcast::channel::<crate::watcher::FileChangeEvent>(100);
        let tx_for_watcher = file_change_tx.clone();

        // Initialize file watcher in background to avoid blocking server startup
        // PollWatcher's recursive scan can take 10+ seconds for large directories
        let base_dir_for_watcher = base_dir.clone();
        let template_folder_for_watcher = template_folder.clone();
        let watcher_ignore_dirs = watcher_ignore_dirs.to_vec();
        let ignore_globs_for_watcher = ignore_globs.to_vec();
        std::thread::spawn(move || {
            match crate::watcher::FileWatcher::new_with_sender(
                &base_dir_for_watcher,
                template_folder_for_watcher.as_deref(),
                &watcher_ignore_dirs,
                &ignore_globs_for_watcher,
                tx_for_watcher,
            ) {
                Ok(watcher) => {
                    tracing::info!("File watcher initialized successfully (background)");
                    // Keep the watcher alive by leaking it (it runs in background thread)
                    std::mem::forget(watcher);
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

        // Initialize link caches (use same size strategy as oembed cache)
        // 2MB for outbound links, 1MB for inbound with 60 second TTL
        let link_cache = Arc::new(LinkCache::new(2 * 1024 * 1024));
        let inbound_link_cache = Arc::new(InboundLinkCache::new(1024 * 1024, 60));

        let config = ServerState {
            base_dir,
            static_folder,
            markdown_extensions: markdown_extensions.to_owned(),
            ignore_dirs: ignore_dirs.to_owned(),
            ignore_globs: ignore_globs.to_owned(),
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
            tag_sources: tag_sources.to_vec(),
        };

        let router = Router::new()
            .route("/", get(Self::home_page))
            .route("/.mbr/site.json", get(Self::get_site_info))
            .route("/.mbr/search", post(Self::search_handler))
            .route("/.mbr/ws/changes", get(Self::websocket_handler))
            .route("/.mbr/{*path}", get(Self::serve_mbr_assets))
            .route("/{*path}", get(Self::handle))
            .layer(CompressionLayer::new())
            .layer(TraceLayer::new_for_http())
            .with_state(config);

        Ok(Server { router, ip, port })
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
        if let Some(tx) = ready_tx {
            let _ = tx.send(());
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
                    if let Some(tx) = ready_tx {
                        let _ = tx.send(self.port);
                    }

                    axum::serve(listener, self.router.clone())
                        .await
                        .map_err(ServerError::StartFailed)?;
                    return Ok(());
                }
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && attempts < max_retries => {
                    let old_port = self.port;
                    self.port = self.port.saturating_add(1);
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
            let _ = sender
                .send(axum::extract::ws::Message::Text(
                    r#"{"error":"File watcher not available"}"#.to_string().into(),
                ))
                .await;
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

    /// Serves assets from /.mbr/* path.
    ///
    /// Priority:
    /// 1. If template_folder is set, serve from there (js/ for components, rest from root)
    /// 2. Otherwise, check .mbr/ directory in base_dir
    /// 3. Fall back to compiled-in DEFAULT_FILES
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

        // Try template_folder first if set
        if let Some(ref template_folder) = config.template_folder {
            // Map components/* -> js/* in template folder
            let file_path = if asset_path.starts_with("/components/") {
                let component_name = asset_path
                    .strip_prefix("/components/")
                    .unwrap_or(&asset_path);
                template_folder.join("components-js").join(component_name)
            } else {
                // Strip leading slash for joining
                template_folder.join(asset_path.trim_start_matches('/'))
            };

            tracing::trace!("Checking template folder: {}", file_path.display());

            if file_path.is_file() {
                return Self::serve_file_from_path(&file_path).await;
            }
        }

        // Try .mbr/ directory in base_dir
        let mbr_dir = config.base_dir.join(".mbr");
        let file_path = mbr_dir.join(asset_path.trim_start_matches('/'));
        tracing::trace!("Checking .mbr dir: {}", file_path.display());

        if file_path.is_file() {
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
    fn render_error_page(
        templates: &templates::Templates,
        status_code: StatusCode,
        error_title: &str,
        error_message: Option<&str>,
        requested_url: &str,
        gui_mode: bool,
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
                CachedMetadata::Cover(bytes) => Some(Self::build_png_response(bytes)),
                CachedMetadata::Chapters(vtt) | CachedMetadata::Captions(vtt) => {
                    Some(Self::build_vtt_response(vtt))
                }
                CachedMetadata::NotAvailable => None, // Cached negative result
            };
        }

        // Try to resolve the video file path
        // First, try the direct path, then try with static_folder prefix
        let video_file = {
            let direct = config.base_dir.join(video_url_path);
            if direct.is_file() {
                direct
            } else {
                let with_static = config
                    .base_dir
                    .join(&config.static_folder)
                    .join(video_url_path);
                if with_static.is_file() {
                    with_static
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
                    Some(Self::build_png_response(bytes))
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
                        Ok((_frontmatter, _headings, _html, outbound_links, _has_h1)) => {
                            // Resolve relative URLs to absolute before caching
                            let resolved_links =
                                resolve_outbound_links(&page_url_path, outbound_links);
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

    /// Build a PNG image response.
    #[cfg(feature = "media-metadata")]
    fn build_png_response(bytes: Vec<u8>) -> Response<Body> {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "image/png")
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
        let (mut frontmatter, headings, inner_html_output, outbound_links, has_h1) =
            markdown::render_with_cache(
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

        let full_html_output = config
            .templates
            .render_markdown(&inner_html_output, frontmatter, extra_context)
            .await
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

        // Detect if we're at the root directory
        let is_root =
            relative_path.as_os_str().is_empty() || relative_path == std::path::Path::new(".");

        // Add is_home to context for template conditional rendering
        context.insert("is_home".to_string(), json!(is_root));

        let full_html_output = if is_root {
            templates
                .render_home(context)
                .await
                .inspect_err(|e| tracing::error!("Error rendering home template: {e}"))?
        } else {
            templates
                .render_section(context)
                .await
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
            file_info
                .raw_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled")
                .to_string()
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
        frontmatter.insert("title".to_string(), "My Title".to_string());
        frontmatter.insert("description".to_string(), "My description".to_string());
        frontmatter.insert("tags".to_string(), "rust, testing".to_string());

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
        assert_eq!(json["tags"], "rust, testing");
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
        frontmatter.insert("title".to_string(), "Only Title".to_string());
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
    }
}
