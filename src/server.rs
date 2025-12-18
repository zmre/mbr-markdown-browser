use axum::{
    body::Body,
    extract::{self, State},
    handler::HandlerWithoutStateExt,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use std::{
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
};

use crate::errors::ServerError;
use crate::path_resolver::{resolve_request_path, PathResolverConfig, ResolvedPath};
use crate::repo::MarkdownInfo;
use crate::templates;
use crate::{markdown, repo::Repo};
use tower::ServiceExt;
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
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
    pub index_file: String,
    pub templates: crate::templates::Templates,
    pub repo: Arc<Mutex<Repo>>,
    pub oembed_timeout_ms: u64,
}

impl Server {
    pub fn init<S: Into<String>, P: Into<std::path::PathBuf>>(
        ip: [u8; 4],
        port: u16,
        base_dir: P,
        static_folder: S,
        markdown_extensions: &[String],
        ignore_dirs: &[String],
        ignore_globs: &[String],
        index_file: S,
        oembed_timeout_ms: u64,
    ) -> Result<Self, ServerError> {
        let base_dir = base_dir.into();
        let static_folder = static_folder.into();
        let index_file = index_file.into();

        // Use try_init to allow multiple server instances in tests
        let _ = tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                    format!("{}=debug,tower_http=debug", env!("CARGO_CRATE_NAME")).into()
                }),
            )
            .with(tracing_subscriber::fmt::layer())
            .try_init();

        let templates = templates::Templates::new(base_dir.as_path())
            .map_err(ServerError::TemplateInit)?;

        let repo = Arc::new(Mutex::new(Repo::init(
            &base_dir,
            &static_folder,
            markdown_extensions,
            ignore_dirs,
            ignore_globs,
            &index_file,
        )));

        let config = ServerState {
            base_dir,
            static_folder,
            markdown_extensions: markdown_extensions.to_owned(),
            index_file,
            templates,
            repo,
            oembed_timeout_ms,
        };

        let mbr_builtins = Self::serve_default_mbr.into_service();
        // let mbr_builtins = get(Self::serve_default_mbr);
        let serve_mbr =
            ServeDir::new(config.base_dir.as_path().join(".mbr")).fallback(mbr_builtins);

        let router = Router::new()
            // .route("/favicon.ico", ServeFile::new())
            .route("/", get(Self::home_page))
            .route("/.mbr/site.json", get(Self::get_site_info))
            .nest_service("/.mbr", serve_mbr)
            .route("/{*path}", get(Self::handle))
            // .fallback_service(handle_static)
            .layer(TraceLayer::new_for_http())
            .with_state(config);

        Ok(Server { router, ip, port })
    }

    pub async fn start(&self) -> Result<(), ServerError> {
        let addr = SocketAddr::from((self.ip, self.port));
        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
            ServerError::BindFailed {
                addr: addr.to_string(),
                source: e,
            }
        })?;
        let local_addr = listener.local_addr().map_err(ServerError::LocalAddrFailed)?;
        tracing::debug!("listening on {}", local_addr);
        axum::serve(listener, self.router.clone())
            .await
            .map_err(ServerError::StartFailed)?;
        Ok(())
    }

    pub async fn get_site_info(
        State(config): State<ServerState>,
    ) -> Result<impl IntoResponse, StatusCode> {
        let repo = config
            .repo
            .lock()
            .inspect_err(|e| tracing::error!("Lock issue with config.repo: {e}"))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        repo.scan_all()
            .inspect_err(|e| tracing::error!("Error scanning repo: {e}"))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let resp = Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(
                repo.to_json()
                    .inspect_err(|e| tracing::error!("Error creating json: {e}"))
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
            )
            .inspect_err(|e| tracing::error!("Error rendering site file: {e}"))
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        Ok(resp.into_response())
    }

    // This is the fallback if the file isn't in the runtime .mbr dir
    pub async fn serve_default_mbr(
        request: extract::Request,
    ) -> Result<impl IntoResponse, StatusCode> {
        tracing::debug!("handle_mb4_404");
        let path = request.uri().path().replace("/.mbr", "");
        if let Some((_name, bytes, mime)) = DEFAULT_FILES.iter().find(|(name, _, _)| {
            tracing::debug!("Comparing path ({}) to name ({})", path, name);
            path.as_str() == *name
        }) {
            tracing::debug!("found default");
            let resp = Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", *mime)
                .body(axum::body::Body::from(*bytes))
                .inspect_err(|e| tracing::error!("Error rendering default file: {e}"))
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok(resp.into_response())
        } else {
            tracing::debug!("no default found");
            // let resp = Response::builder()
            //     .status(StatusCode::NOT_FOUND)
            //     .body(axum::body::Body::from("404 Not Found in fallback"))
            //     .unwrap();
            // resp.into_response()
            Err(StatusCode::NOT_FOUND)
        }
    }

    async fn handle(
        extract::Path(path): extract::Path<String>,
        State(config): State<ServerState>,
        req: extract::Request<Body>,
    ) -> Result<impl IntoResponse, StatusCode> {
        tracing::debug!("handle: {}", &path);

        let resolver_config = PathResolverConfig {
            base_dir: config.base_dir.as_path(),
            static_folder: &config.static_folder,
            markdown_extensions: &config.markdown_extensions,
            index_file: &config.index_file,
        };

        match resolve_request_path(&resolver_config, &path) {
            ResolvedPath::StaticFile(file_path) => {
                tracing::debug!("serving static file: {:?}", &file_path);
                Self::serve_static_file(file_path, req).await
            }
            ResolvedPath::MarkdownFile(md_path) => {
                tracing::debug!("rendering markdown: {:?}", &md_path);
                Self::markdown_to_html(
                    &md_path,
                    &config.templates,
                    config.base_dir.as_path(),
                    config.oembed_timeout_ms,
                )
                .await
                .map(|html| html.into_response())
                .map_err(|e| {
                    tracing::error!("Error rendering markdown: {e}");
                    StatusCode::INTERNAL_SERVER_ERROR
                })
            }
            ResolvedPath::DirectoryListing(dir_path) => {
                tracing::debug!("generating directory listing: {:?}", &dir_path);
                Self::directory_to_html(&dir_path, &config.templates, config.base_dir.as_path(), &config)
                    .await
                    .map(|html| html.into_response())
                    .map_err(|e| {
                        tracing::error!("Error generating directory listing: {e}");
                        StatusCode::INTERNAL_SERVER_ERROR
                    })
            }
            ResolvedPath::NotFound => {
                tracing::debug!("resource not found: {}", &path);
                Err(StatusCode::NOT_FOUND)
            }
        }
    }

    /// Serves a static file using tower's ServeFile service.
    async fn serve_static_file(
        file_path: std::path::PathBuf,
        req: extract::Request<Body>,
    ) -> Result<Response, StatusCode> {
        let static_service = ServeFile::new(file_path);
        static_service
            .oneshot(req)
            .await
            .map(|r| r.into_response())
            .map_err(|e| {
                tracing::error!("Error serving static file: {e}");
                StatusCode::INTERNAL_SERVER_ERROR
            })
    }

    async fn markdown_to_html(
        md_path: &Path,
        templates: &crate::templates::Templates,
        root_path: &Path,
        oembed_timeout_ms: u64,
    ) -> Result<Html<String>, Box<dyn std::error::Error>> {
        let (mut frontmatter, inner_html_output) =
            markdown::render(md_path.to_path_buf(), root_path, oembed_timeout_ms)
                .await
                .inspect_err(|e| eprintln!("Error rendering markdown: {e}"))?;
        frontmatter.insert("markdown_source".into(), md_path.to_string_lossy().into());
        let full_html_output = templates
            .render_markdown(&inner_html_output, frontmatter)
            .await
            .inspect_err(|e| eprintln!("Error rendering template: {e}"))?;
        tracing::debug!("generated the html");
        Ok(Html(full_html_output))
    }

    async fn directory_to_html(
        dir_path: &Path,
        templates: &crate::templates::Templates,
        root_path: &Path,
        config: &ServerState,
    ) -> Result<Html<String>, Box<dyn std::error::Error>> {
        use serde_json::json;

        // Create a temporary repo instance to scan this directory
        let ignore_dirs = vec![
            "target".to_string(),
            "result".to_string(),
            "build".to_string(),
            "node_modules".to_string(),
            "ci".to_string(),
        ];
        let ignore_globs = vec![
            "*.log".to_string(),
            "*.bak".to_string(),
            "*.lock".to_string(),
        ];
        let temp_repo = Repo::init(
            root_path,
            &config.static_folder,
            &config.markdown_extensions,
            &ignore_dirs,
            &ignore_globs,
            &config.index_file,
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

        // Sort by modified timestamp, newest first
        files.sort_by(|a, b| {
            let a_modified = a["modified"].as_u64().unwrap_or(0);
            let b_modified = b["modified"].as_u64().unwrap_or(0);
            b_modified.cmp(&a_modified)
        });

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

        let full_html_output = templates
            .render_section(context)
            .await
            .inspect_err(|e| eprintln!("Error rendering section template: {e}"))?;

        tracing::debug!("generated directory listing html");
        Ok(Html(full_html_output))
    }

    async fn home_page() -> impl IntoResponse {
        // TODO: look for index.{markdown extensions} then index.html then finally fall back to some hard coded html maybe with a list of markdown files in the same dir and immediate children?
        tracing::debug!("home");
        "Home".to_string()
    }
}

// ============================================================================
// Pure helper functions for directory listing (extracted for testability)
// ============================================================================

/// A breadcrumb entry for navigation.
#[derive(Debug, Clone, PartialEq, Eq)]
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

    // Start with Home
    let mut breadcrumbs = vec![Breadcrumb::new("Home", "/")];

    // Add all but the last component (last is current directory)
    for (idx, _) in path_components.iter().enumerate().take(path_components.len().saturating_sub(1)) {
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
        let parent: std::path::PathBuf = path_components.iter().take(path_components.len() - 1).collect();
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

pub const DEFAULT_FILES: &[(&str, &[u8], &str)] = &[
    (
        "/theme.css",
        include_bytes!("../templates/theme.css"),
        "text/css",
    ),
    (
        "/user.css",
        &[], // the idea of this is for users to override and for us to leave blank
        "text/css",
    ),
    (
        "/pico.min.css",
        include_bytes!("../templates/pico.min.css"),
        "text/css",
    ),
    (
        "/vid.js",
        include_bytes!("../templates/vid.js"),
        "application/javascript",
    ),
    (
        "/vidstack.player.css",
        include_bytes!("../templates/vidstack.player.1.11.21.css"),
        "text/css",
    ),
    (
        "/vidstack.plyr.css",
        include_bytes!("../templates/vidstack.plyr.1.11.21.css"),
        "text/css",
    ),
    (
        "/vidstack.player.js",
        include_bytes!("../templates/vidstack.player.1.12.13.js"),
        "application/javascript",
    ),
    (
        "/components/mbr-components.js",
        include_bytes!("../components/dist/mbr-components.js"),
        "application/javascript",
    ),
    (
        "/components/mbr-components.css",
        include_bytes!("../components/dist/mbr-components.css"),
        "application/javascript",
    ),
    // (
    //     "/components/legacy.js",
    //     include_bytes!("../templates/components/legacy.js"),
    //     "application/javascript",
    // ),
    // (
    //     "/components/disclose-version.js",
    //     include_bytes!("../templates/components/disclose-version.js"),
    //     "application/javascript",
    // ),
    // (
    //     "/components/mbr-browse.es.js",
    //     include_bytes!("../templates/components/mbr-browse.es.js"),
    //     "application/javascript",
    // ),
    // (
    //     "/components/mbr-jump.es.js",
    //     include_bytes!("../templates/components/mbr-jump.es.js"),
    //     "application/javascript",
    // ),
    // (
    //     "/components/mbr-info.es.js",
    //     include_bytes!("../templates/components/mbr-info.es.js"),
    //     "application/javascript",
    // ),
    // (
    //     "/components/mbr-search.es.js",
    //     include_bytes!("../templates/components/mbr-search.es.js"),
    //     "application/javascript",
    // ),
    // (
    //     "/components/mbr-navloader.es.js",
    //     include_bytes!("../templates/components/mbr-navloader.es.js"),
    //     "application/javascript",
    // ),
    // (
    //     "/components/svelte.js",
    //     include_bytes!("../templates/components/svelte.js"),
    //     "application/javascript",
    // ),
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

        assert_eq!(breadcrumbs.len(), 1);
        assert_eq!(breadcrumbs[0], Breadcrumb::new("Home", "/"));
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
        let path = Path::new("a/b/c/d");
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
        /// For 0 components: [Home] = 1
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
            let expected_count = if components.is_empty() {
                1  // Just Home
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

        /// First breadcrumb is always "Home" with url "/"
        #[test]
        fn prop_first_breadcrumb_is_home(
            components in proptest::collection::vec(path_component_strategy(), 0..5)
        ) {
            let path_str = components.join("/");
            let path = Path::new(&path_str);
            let breadcrumbs = generate_breadcrumbs(path);

            prop_assert!(!breadcrumbs.is_empty(), "Should always have at least Home breadcrumb");
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
