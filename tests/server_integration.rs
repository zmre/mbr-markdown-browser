//! Integration tests for the mbr server.

mod common;

use common::{TestRepo, assert_html_contains, assert_html_not_contains, find_available_port};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Create a test ServerConfig with sensible defaults for integration tests.
fn test_server_config(port: u16, root_dir: PathBuf) -> mbr::server::ServerConfig {
    mbr::server::ServerConfig {
        ip: [127, 0, 0, 1],
        port,
        base_dir: root_dir,
        static_folder: "static".to_string(),
        markdown_extensions: vec!["md".to_string()],
        ignore_dirs: vec!["target".to_string(), "node_modules".to_string()],
        ignore_globs: vec!["*.log".to_string()],
        watcher_ignore_dirs: vec![
            ".direnv".to_string(),
            ".git".to_string(),
            "result".to_string(),
            "target".to_string(),
            "build".to_string(),
        ],
        index_file: "index.md".to_string(),
        oembed_timeout_ms: 100,
        oembed_cache_size: 2 * 1024 * 1024,
        #[cfg(feature = "media-metadata")]
        media_cache_size: 64 * 1024 * 1024,
        template_folder: None,
        sort: mbr::config::default_sort_config(),
        gui_mode: false,
        theme: "default".to_string(),
        log_filter: None,
        link_tracking: true,
        relationship_tracking: true,
        relationship_types: mbr::config::default_relationship_types(),
        tag_sources: mbr::config::default_tag_sources(),
        sidebar_style: "panel".to_string(),
        sidebar_max_items: 100,
        graph_depth: 2,
        title_prefix: String::new(),
        title_suffix: String::new(),
        mark_incomplete: true,
        incomplete_markers: mbr::config::default_incomplete_markers(),
        tasks_enabled: true,
        tasks_stamp_done: true,
        edit_enabled: false,
        edit_require_token_on_loopback: false,
        edit_token_hash: None,
        upload_max_bytes: 25 * 1024 * 1024,
        #[cfg(feature = "media-metadata")]
        transcode_enabled: false,
    }
}

/// Helper to start a test server and make requests.
struct TestServer {
    port: u16,
    client: reqwest::Client,
    _handle: tokio::task::JoinHandle<()>,
}

impl TestServer {
    async fn start(repo: &TestRepo) -> Self {
        let port = find_available_port();
        let root_dir = repo.path().to_path_buf();

        let handle = tokio::spawn(async move {
            let config = test_server_config(port, root_dir);
            let server = mbr::server::Server::init(config).expect("Failed to initialize server");

            // Start server (will run until task is dropped)
            server.start().await.expect("test server failed to start");
        });

        // Give server time to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        let client = mbr::http_client(Duration::from_secs(5));

        Self {
            port,
            client,
            _handle: handle,
        }
    }

    async fn start_with_config_fn(
        repo: &TestRepo,
        config_fn: impl FnOnce(&mut mbr::server::ServerConfig) + Send + 'static,
    ) -> Self {
        Self::start_at_path_with(repo.path().to_path_buf(), config_fn).await
    }

    /// Starts a server rooted at an arbitrary on-disk path (used for the
    /// committed genealogy fixture), applying an optional config tweak.
    async fn start_at_path_with(
        root_dir: PathBuf,
        config_fn: impl FnOnce(&mut mbr::server::ServerConfig) + Send + 'static,
    ) -> Self {
        let port = find_available_port();

        let handle = tokio::spawn(async move {
            let mut config = test_server_config(port, root_dir);
            config_fn(&mut config);
            let server = mbr::server::Server::init(config).expect("Failed to initialize server");
            server.start().await.expect("test server failed to start");
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let client = mbr::http_client(Duration::from_secs(5));

        Self {
            port,
            client,
            _handle: handle,
        }
    }

    /// Starts a server rooted at an arbitrary on-disk path with default config.
    async fn start_at_path(root_dir: PathBuf) -> Self {
        Self::start_at_path_with(root_dir, |_| {}).await
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }

    async fn get(&self, path: &str) -> reqwest::Response {
        self.client
            .get(self.url(path))
            .send()
            .await
            .expect("Request failed")
    }

    async fn get_text(&self, path: &str) -> String {
        self.get(path)
            .await
            .text()
            .await
            .expect("Failed to get response text")
    }

    async fn post_json(&self, path: &str, body: &str) -> reqwest::Response {
        self.client
            .post(self.url(path))
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .expect("Request failed")
    }

    /// Wait for the background repo scan to complete.
    ///
    /// The `/.mbr/site.json` endpoint blocks until the scan is finished,
    /// so fetching it serves as a reliable synchronization point.
    async fn wait_for_scan(&self) {
        let _ = self.get("/.mbr/site.json").await;
    }
}

#[tokio::test]
async fn test_serve_markdown_file() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello World\n\nThis is a test.");

    let server = TestServer::start(&repo).await;
    let response = server.get("/readme/").await;

    assert_eq!(response.status(), 200);

    let html = response.text().await.unwrap();
    assert_html_contains(&html, "<h1 id=\"hello-world\">Hello World</h1>");
    assert_html_contains(&html, "This is a test.");
}

#[tokio::test]
async fn test_serve_markdown_file_with_dotted_name() {
    // Regression: a markdown file whose name contains a period is served at a
    // canonical URL that strips only the final extension (e.g.
    // `patrick-walsh-b.2010-03-03.md` -> `/patrick-walsh-b.2010-03-03/`). It must
    // resolve rather than 404.
    let repo = TestRepo::new();
    repo.create_markdown(
        "patrick-walsh-b.2010-03-03.md",
        "# Patrick Walsh\n\nBorn 2010.",
    );

    let server = TestServer::start(&repo).await;
    let response = server.get("/patrick-walsh-b.2010-03-03/").await;

    assert_eq!(response.status(), 200);

    let html = response.text().await.unwrap();
    assert_html_contains(&html, "Patrick Walsh");
    assert_html_contains(&html, "Born 2010.");
}

#[tokio::test]
async fn test_head_config_includes_graph_depth() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello\n\nBody.");

    // Default config surfaces graphDepth: 2 in the __MBR_CONFIG__ head script.
    let server = TestServer::start(&repo).await;
    let html = server.get_text("/readme/").await;
    assert_html_contains(&html, "graphDepth: 2");

    // A graph_depth override flows through to the template.
    let server = TestServer::start_with_config_fn(&repo, |c| {
        c.graph_depth = 4;
    })
    .await;
    let html = server.get_text("/readme/").await;
    assert_html_contains(&html, "graphDepth: 4");
}

#[tokio::test]
async fn test_gui_mode_emits_find_bar() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello\n\nBody.");

    // Plain server mode: the browser has its own find, so the bar must not ship.
    let server = TestServer::start(&repo).await;
    let html = server.get_text("/readme/").await;
    assert_html_not_contains(&html, "<mbr-find-bar>");
    assert_html_contains(&html, "guiMode: false");

    // GUI mode: the bar ships, because the webview has no find of its own.
    let server = TestServer::start_with_config_fn(&repo, |c| {
        c.gui_mode = true;
    })
    .await;
    let html = server.get_text("/readme/").await;
    assert_html_contains(&html, "<mbr-find-bar></mbr-find-bar>");
    // The element and window.__MBR_CONFIG__.guiMode come from the same Tera
    // variable; asserting both together keeps them from drifting apart.
    assert_html_contains(&html, "guiMode: true");
}

#[tokio::test]
async fn test_server_marks_incomplete_blocks_by_default() {
    // Server/GUI default for mark_incomplete is true; rendered HTML should
    // wrap blocks starting with TK/TODO/FIXME/XXX in an mbr-incomplete span.
    let repo = TestRepo::new();
    repo.create_markdown("drafts.md", "# Drafts\n\nTK rewrite this paragraph.");

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/drafts/").await;
    assert!(
        html.contains(r#"<span class="mbr-incomplete">"#),
        "Server should highlight TK paragraph by default: {html}"
    );
}

#[tokio::test]
async fn test_server_no_incomplete_spans_when_disabled() {
    // When the user disables mark_incomplete, the spans must not appear.
    let repo = TestRepo::new();
    repo.create_markdown("drafts.md", "# Drafts\n\nTK rewrite this paragraph.");

    let server = TestServer::start_with_config_fn(&repo, |c| c.mark_incomplete = false).await;
    let html = server.get_text("/drafts/").await;
    assert!(
        !html.contains("mbr-incomplete"),
        "Server should not emit spans when mark_incomplete=false: {html}"
    );
}

// NOTE: Root path "/" is handled by a placeholder home_page() function.
// This test verifies index.md works in subdirectories instead.
// TODO: Update when home_page() is implemented to handle index.md
#[tokio::test]
async fn test_serve_index_at_subdirectory() {
    let repo = TestRepo::new();
    repo.create_dir("home");
    repo.create_markdown("home/index.md", "# Home Page");

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/home/").await;

    assert_html_contains(&html, "<h1 id=\"home-page\">Home Page</h1>");
}

#[tokio::test]
async fn test_serve_directory_index() {
    let repo = TestRepo::new();
    repo.create_dir("docs");
    repo.create_markdown("docs/index.md", "# Documentation");

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/docs/").await;

    assert_html_contains(&html, "<h1 id=\"documentation\">Documentation</h1>");
}

#[tokio::test]
async fn test_serve_nested_markdown() {
    let repo = TestRepo::new();
    repo.create_markdown("blog/posts/first.md", "# First Post\n\nContent.");

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/blog/posts/first/").await;

    assert_html_contains(&html, "<h1 id=\"first-post\">First Post</h1>");
}

#[tokio::test]
async fn test_directory_listing() {
    let repo = TestRepo::new();
    repo.create_dir("articles");
    repo.create_markdown("articles/one.md", "# Article One");
    repo.create_markdown("articles/two.md", "# Article Two");

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/articles/").await;

    // Should show a directory listing with links to both articles
    assert_html_contains(&html, "one");
    assert_html_contains(&html, "two");
}

#[tokio::test]
async fn test_static_file_serving() {
    let repo = TestRepo::new();
    repo.create_static_file("image.txt", b"Hello from static file");

    let server = TestServer::start(&repo).await;
    let response = server.get("/image.txt").await;

    assert_eq!(response.status(), 200);
    let text = response.text().await.unwrap();
    assert_eq!(text, "Hello from static file");
}

// ============================================================================
// Static Folder Serving Tests
// ============================================================================

#[tokio::test]
async fn test_static_folder_file_serving() {
    // Create temp repo with static folder structure
    let repo = TestRepo::new();
    repo.create_dir("static/images");
    repo.create_static_file("static/images/test.png", b"PNG data");

    let server = TestServer::start(&repo).await;

    // Request /images/test.png should find static/images/test.png
    let response = server.get("/images/test.png").await;

    assert_eq!(response.status(), 200);
    let bytes = response.bytes().await.unwrap();
    assert_eq!(bytes.as_ref(), b"PNG data");
}

#[tokio::test]
async fn test_static_folder_nested_path_serving() {
    // Test deeply nested paths through actual HTTP requests
    let repo = TestRepo::new();
    repo.create_dir("static/images/blog/2024");
    repo.create_static_file("static/images/blog/2024/photo.jpg", b"JPEG");

    let server = TestServer::start(&repo).await;

    let response = server.get("/images/blog/2024/photo.jpg").await;

    assert_eq!(response.status(), 200);
    let bytes = response.bytes().await.unwrap();
    assert_eq!(bytes.as_ref(), b"JPEG");
}

#[tokio::test]
async fn test_static_folder_deeply_nested_path() {
    // Test 5+ levels of nesting through HTTP
    let repo = TestRepo::new();
    repo.create_dir("static/a/b/c/d/e");
    repo.create_static_file("static/a/b/c/d/e/deep.txt", b"deep content");

    let server = TestServer::start(&repo).await;

    let response = server.get("/a/b/c/d/e/deep.txt").await;

    assert_eq!(response.status(), 200);
    let bytes = response.bytes().await.unwrap();
    assert_eq!(bytes.as_ref(), b"deep content");
}

#[tokio::test]
async fn test_static_folder_precedence_base_dir_wins() {
    // When file exists in BOTH base_dir and static folder, base_dir should win
    let repo = TestRepo::new();
    repo.create_static_file("image.png", b"from base_dir");
    repo.create_dir("static");
    repo.create_static_file("static/image.png", b"from static folder");

    let server = TestServer::start(&repo).await;

    let response = server.get("/image.png").await;

    assert_eq!(response.status(), 200);
    let bytes = response.bytes().await.unwrap();
    assert_eq!(
        bytes.as_ref(),
        b"from base_dir",
        "Should serve file from base_dir, not static folder"
    );
}

#[tokio::test]
async fn test_static_folder_fallback_when_not_in_base() {
    // When file ONLY exists in static folder, it should be served
    let repo = TestRepo::new();
    // Note: NOT creating base_dir/images/
    repo.create_dir("static/images");
    repo.create_static_file("static/images/only-here.png", b"static only");

    let server = TestServer::start(&repo).await;

    let response = server.get("/images/only-here.png").await;

    assert_eq!(response.status(), 200);
    let bytes = response.bytes().await.unwrap();
    assert_eq!(bytes.as_ref(), b"static only");
}

#[tokio::test]
async fn test_static_folder_with_spaces_in_path() {
    // Test URL-encoded spaces in static folder paths
    let repo = TestRepo::new();
    repo.create_dir("static/my images");
    repo.create_static_file("static/my images/photo file.jpg", b"spaced content");

    let server = TestServer::start(&repo).await;

    // URL-encoded spaces
    let response = server.get("/my%20images/photo%20file.jpg").await;

    assert_eq!(response.status(), 200);
    let bytes = response.bytes().await.unwrap();
    assert_eq!(bytes.as_ref(), b"spaced content");
}

/// End-to-end cover for the `repo/content` + `repo/static` layout: the markdown
/// root holds `.mbr/`, and the assets live in a *peer* directory named as
/// `static_folder = "../static"`.
///
/// Regression. Tightening `static_folder` to "no `..` ever" made this layout
/// fail to boot at all, which surfaced to the user as "video media doesn't seem
/// to play in gui mode". The range request is the interesting half: that is how
/// a webview actually streams video, and it goes down the same static-overlay
/// resolution path as a plain GET.
#[tokio::test]
async fn test_peer_static_folder_serves_assets_and_ranges() {
    let project = tempfile::tempdir().unwrap();
    let content = project.path().join("content");
    std::fs::create_dir_all(content.join(".mbr")).unwrap();
    std::fs::write(content.join("readme.md"), "# Peer layout").unwrap();

    let videos = project.path().join("static/videos");
    std::fs::create_dir_all(&videos).unwrap();
    let video_bytes: &[u8] = b"\x00\x00\x00\x20ftypisom fake mp4 payload";
    std::fs::write(videos.join("demo.mp4"), video_bytes).unwrap();
    std::fs::write(project.path().join("static/pic.png"), b"PNG bytes").unwrap();
    std::fs::write(project.path().join("secret.txt"), b"not servable").unwrap();

    let server = TestServer::start_at_path_with(content, |config| {
        config.static_folder = "../static".to_string();
    })
    .await;

    let response = server.get("/videos/demo.mp4").await;
    assert_eq!(
        response.status(),
        200,
        "a video in a peer static folder must be served"
    );
    assert_eq!(response.bytes().await.unwrap().as_ref(), video_bytes);

    let response = server.get("/pic.png").await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.bytes().await.unwrap().as_ref(), b"PNG bytes");

    let response = server
        .client
        .get(server.url("/videos/demo.mp4"))
        .header("Range", "bytes=0-3")
        .send()
        .await
        .expect("range request failed");
    assert_eq!(
        response.status(),
        206,
        "a peer static folder must support the range requests a webview streams with"
    );
    assert_eq!(response.bytes().await.unwrap().as_ref(), &video_bytes[0..4]);

    assert_eq!(server.get("/").await.status(), 200);

    // Containment still holds: widening the overlay to a peer did not widen it
    // to the peer's *parent*. `project/secret.txt` sits next to both `content`
    // and `static`, so it is under neither served root and must 404. Asserted
    // with a plain name rather than a `..` path on purpose — reqwest normalizes
    // `..` (and `%2e%2e`) out of the URL before it is ever sent, so a `..`
    // assertion here would pass without testing anything. Request-path traversal
    // is covered directly in `path_resolver`'s unit tests.
    assert_eq!(
        server.get("/secret.txt").await.status(),
        404,
        "a file beside the peer static folder must not be served"
    );
}

// Only the macOS and Linux canonicalize() behaviors are asserted below, so the
// test is gated to those platforms rather than silently passing elsewhere.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn test_static_folder_trailing_slash_platform_behavior() {
    // Behavior is platform-dependent:
    // - macOS: canonicalize() tolerates trailing slashes on file paths (200)
    // - Linux: canonicalize() rejects trailing slashes on file paths (404)
    let repo = TestRepo::new();
    repo.create_dir("static/images");
    repo.create_static_file("static/images/photo.png", b"image");

    let server = TestServer::start(&repo).await;
    let response = server.get("/images/photo.png/").await;

    #[cfg(target_os = "macos")]
    {
        assert_eq!(
            response.status(),
            200,
            "macOS: trailing slash on file path should serve file"
        );
        let bytes = response.bytes().await.unwrap();
        assert_eq!(bytes.as_ref(), b"image");
    }

    #[cfg(target_os = "linux")]
    {
        assert_eq!(
            response.status(),
            404,
            "Linux: trailing slash on file path should return 404"
        );
    }
}

#[tokio::test]
async fn test_404_for_missing_file() {
    let repo = TestRepo::new();

    let server = TestServer::start(&repo).await;
    let response = server.get("/nonexistent/").await;

    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn test_non_canonical_index_url_redirects() {
    let repo = TestRepo::new();
    repo.create_dir("docs");
    repo.create_markdown("docs/index.md", "# Docs");

    let server = TestServer::start(&repo).await;

    // Use a client that doesn't follow redirects
    let client = mbr::http_client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    // Request /docs/index/ should get 301 redirect to /docs/
    let response = client.get(server.url("/docs/index/")).send().await.unwrap();

    assert_eq!(response.status(), 301);
    assert_eq!(response.headers().get("location").unwrap(), "/docs/");
}

#[tokio::test]
async fn test_root_index_url_redirects() {
    let repo = TestRepo::new();
    repo.create_markdown("index.md", "# Home");

    let server = TestServer::start(&repo).await;

    // Use a client that doesn't follow redirects
    let client = mbr::http_client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    // Request /index/ should get 301 redirect to /
    let response = client.get(server.url("/index/")).send().await.unwrap();

    assert_eq!(response.status(), 301);
    assert_eq!(response.headers().get("location").unwrap(), "/");
}

#[tokio::test]
async fn test_markdown_with_frontmatter() {
    let repo = TestRepo::new();
    let mut frontmatter = HashMap::new();
    frontmatter.insert("title", "My Custom Title");

    repo.create_markdown_with_frontmatter("page.md", &frontmatter, "Page content here.");

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/page/").await;

    // The title from frontmatter should be used in the page
    assert_html_contains(&html, "My Custom Title");
    assert_html_contains(&html, "Page content here.");
}

#[tokio::test]
async fn test_site_json_endpoint() {
    let repo = TestRepo::new();
    repo.create_markdown("test.md", "# Test");

    let server = TestServer::start(&repo).await;
    let response = server.get("/.mbr/site.json").await;

    assert_eq!(response.status(), 200);
    let content_type = response.headers().get("content-type").unwrap();
    assert!(content_type.to_str().unwrap().contains("application/json"));
}

#[tokio::test]
async fn test_default_css_served() {
    let repo = TestRepo::new();

    let server = TestServer::start(&repo).await;
    let response = server.get("/.mbr/theme.css").await;

    assert_eq!(response.status(), 200);
    let content_type = response.headers().get("content-type").unwrap();
    assert!(content_type.to_str().unwrap().contains("text/css"));
}

/// The print stylesheet hides chrome with an allowlist -- it hides *every*
/// child of `<body>` that is not `<main>` (see the "Print Styles" section of
/// `templates/theme.css`). That is only safe while the page templates actually
/// put the rendered content in a `<main>` element, and nothing in the compiler
/// or the template engine enforces that coupling: renaming the container to a
/// `<div>` would still serve a perfectly good page that prints as a blank
/// sheet. Pin both halves together here so the drift fails loudly.
#[tokio::test]
async fn test_print_allowlist_matches_content_container() {
    let repo = TestRepo::new();
    repo.create_markdown("test.md", "# Printable\n\nBody text.");

    let server = TestServer::start(&repo).await;

    let css = server.get_text("/.mbr/theme.css").await;
    assert!(
        css.contains("body > *:not(main, mbr-genealogy, .mbr-print-keep)"),
        "print styles should hide body children via an allowlist"
    );

    let page = server.get_text("/test/").await;
    assert!(
        page.contains("<main id=\"wrapper\""),
        "rendered content must live in <main>, or the print allowlist hides it"
    );
    assert!(
        page.contains("Body text."),
        "sanity: the page should contain the rendered markdown"
    );
}

// ============================================================================
// Search endpoint tests
// ============================================================================

#[tokio::test]
async fn test_search_endpoint_returns_json() {
    let repo = TestRepo::new();
    repo.create_markdown("test.md", "# Test Page\n\nSome searchable content.");

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;
    let response = server.post_json("/.mbr/search", r#"{"q": "test"}"#).await;

    assert_eq!(response.status(), 200);
    let content_type = response.headers().get("content-type").unwrap();
    assert!(content_type.to_str().unwrap().contains("application/json"));
}

#[tokio::test]
async fn test_search_finds_by_title() {
    let repo = TestRepo::new();
    let mut frontmatter = HashMap::new();
    frontmatter.insert("title", "Unique Search Title");
    repo.create_markdown_with_frontmatter("findme.md", &frontmatter, "Some content.");

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;
    let response = server
        .post_json("/.mbr/search", r#"{"q": "Unique Search"}"#)
        .await;

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();

    assert!(body["total_matches"].as_i64().unwrap() >= 1);
    let results = body["results"].as_array().unwrap();
    assert!(!results.is_empty());

    // Check that our file was found
    let found = results
        .iter()
        .any(|r| r["url_path"].as_str().unwrap().contains("findme"));
    assert!(found, "Expected to find 'findme' in results: {:?}", results);
}

#[tokio::test]
async fn test_search_with_scope_metadata() {
    let repo = TestRepo::new();
    let mut frontmatter = HashMap::new();
    frontmatter.insert("title", "Metadata Only Title");
    repo.create_markdown_with_frontmatter("meta.md", &frontmatter, "Body text without match.");

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;
    let response = server
        .post_json(
            "/.mbr/search",
            r#"{"q": "Metadata Only", "scope": "metadata"}"#,
        )
        .await;

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();

    assert!(body["total_matches"].as_i64().unwrap() >= 1);
}

#[tokio::test]
async fn test_search_with_limit() {
    let repo = TestRepo::new();
    // Create multiple files
    for i in 1..=5 {
        repo.create_markdown(&format!("file{}.md", i), &format!("# File {} content", i));
    }

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;
    let response = server
        .post_json("/.mbr/search", r#"{"q": "file", "limit": 2}"#)
        .await;

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();

    let results = body["results"].as_array().unwrap();
    assert!(
        results.len() <= 2,
        "Expected at most 2 results, got {}",
        results.len()
    );
}

#[tokio::test]
async fn test_search_includes_duration() {
    let repo = TestRepo::new();
    repo.create_markdown("test.md", "# Test");

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;
    let response = server.post_json("/.mbr/search", r#"{"q": "test"}"#).await;

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();

    assert!(
        body["duration_ms"].is_number(),
        "Expected duration_ms in response"
    );
    assert!(
        body["query"].as_str().unwrap() == "test",
        "Expected query echo in response"
    );
}

// ==================== Link Transformation Tests ====================

#[tokio::test]
async fn test_link_transform_regular_markdown() {
    // Regular markdown file (not index) - relative links get ../ prefix
    let repo = TestRepo::new();
    repo.create_dir("docs");
    repo.create_markdown("docs/guide.md", "# Guide\n\n[Other Doc](other.md)");

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/docs/guide/").await;

    // guide.md becomes /docs/guide/, so link to other.md should be ../other/
    assert!(
        html.contains(r#"href="../other/""#),
        "Regular markdown should transform other.md to ../other/. Got: {}",
        html
    );
}

#[tokio::test]
async fn test_link_transform_index_file() {
    // Index file - relative links do NOT get ../ prefix
    let repo = TestRepo::new();
    repo.create_dir("docs");
    repo.create_markdown("docs/index.md", "# Docs Index\n\n[Guide](guide.md)");

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/docs/").await;

    // index.md becomes /docs/, so link to guide.md should be guide/
    assert!(
        html.contains(r#"href="guide/""#),
        "Index file should transform guide.md to guide/ (no ../). Got: {}",
        html
    );
}

#[tokio::test]
async fn test_link_transform_root_index() {
    // Root index.md - links to subdirectory files
    let repo = TestRepo::new();
    repo.create_markdown("index.md", "# Home\n\n[Docs Guide](docs/guide.md)");
    repo.create_dir("docs");
    repo.create_markdown("docs/guide.md", "# Guide");

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/").await;

    // Root index.md - link should be docs/guide/
    assert!(
        html.contains(r#"href="docs/guide/""#),
        "Root index should transform docs/guide.md to docs/guide/. Got: {}",
        html
    );
}

#[tokio::test]
async fn test_link_transform_preserves_anchors() {
    let repo = TestRepo::new();
    repo.create_dir("docs");
    repo.create_markdown(
        "docs/page.md",
        "# Page\n\n[Other Section](other.md#section)",
    );

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/docs/page/").await;

    assert!(
        html.contains(r#"href="../other/#section""#),
        "Anchors should be preserved. Got: {}",
        html
    );
}

#[tokio::test]
async fn test_link_transform_preserves_absolute_urls() {
    let repo = TestRepo::new();
    repo.create_markdown(
        "page.md",
        "# Page\n\n[External](https://example.com)\n\n[Root](/about/)",
    );

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/page/").await;

    assert!(
        html.contains(r#"href="https://example.com""#),
        "Absolute URLs should remain unchanged. Got: {}",
        html
    );
    assert!(
        html.contains(r#"href="/about/""#),
        "Root-relative URLs should remain unchanged. Got: {}",
        html
    );
}

#[tokio::test]
async fn test_link_transform_static_files() {
    // Static file links (images, etc.) should also be transformed
    let repo = TestRepo::new();
    repo.create_dir("docs");
    repo.create_markdown("docs/page.md", "# Page\n\n![Image](images/photo.jpg)");

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/docs/page/").await;

    assert!(
        html.contains(r#"src="../images/photo.jpg""#),
        "Image paths should be transformed. Got: {}",
        html
    );
}

#[tokio::test]
async fn test_link_transform_index_collapse() {
    // Link to folder/index.md should collapse to folder/
    let repo = TestRepo::new();
    repo.create_dir("docs");
    repo.create_markdown("docs/page.md", "# Page\n\n[Subfolder](subfolder/index.md)");

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/docs/page/").await;

    assert!(
        html.contains(r#"href="../subfolder/""#),
        "Links to index.md should collapse to folder/. Got: {}",
        html
    );
}

// ==================== Faceted Search Tests ====================

#[tokio::test]
async fn test_search_with_facet() {
    let repo = TestRepo::new();
    let mut frontmatter = HashMap::new();
    frontmatter.insert("category", "programming");
    frontmatter.insert("title", "Rust Guide");
    repo.create_markdown_with_frontmatter("rust.md", &frontmatter, "Learn Rust programming.");

    let mut other_fm = HashMap::new();
    other_fm.insert("category", "cooking");
    other_fm.insert("title", "Recipe Book");
    repo.create_markdown_with_frontmatter("recipe.md", &other_fm, "Cooking recipes.");

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    // Search with facet should only find matching file
    let response = server
        .post_json("/.mbr/search", r#"{"q": "category:programming"}"#)
        .await;
    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.unwrap();
    let results = body["results"].as_array().unwrap();

    // Should find rust.md but not recipe.md
    assert!(
        results
            .iter()
            .any(|r| r["url_path"].as_str().unwrap().contains("rust")),
        "Expected to find 'rust' in results: {:?}",
        results
    );
    assert!(
        !results
            .iter()
            .any(|r| r["url_path"].as_str().unwrap().contains("recipe")),
        "Should not find 'recipe' in results: {:?}",
        results
    );
}

#[tokio::test]
async fn test_search_facet_contains_match() {
    let repo = TestRepo::new();
    let mut frontmatter = HashMap::new();
    frontmatter.insert("category", "systems programming");
    repo.create_markdown_with_frontmatter("systems.md", &frontmatter, "Low-level code.");

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    // Facet should use contains match
    let response = server
        .post_json("/.mbr/search", r#"{"q": "category:programming"}"#)
        .await;
    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.unwrap();
    let results = body["results"].as_array().unwrap();

    assert!(
        !results.is_empty(),
        "Should find file with 'systems programming' category"
    );
}

#[tokio::test]
async fn test_search_facet_case_insensitive() {
    let repo = TestRepo::new();
    let mut frontmatter = HashMap::new();
    frontmatter.insert("language", "RUST");
    repo.create_markdown_with_frontmatter("code.md", &frontmatter, "Some code.");

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    // Facet should be case-insensitive
    let response = server
        .post_json("/.mbr/search", r#"{"q": "language:rust"}"#)
        .await;
    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.unwrap();
    assert!(
        body["total_matches"].as_i64().unwrap() >= 1,
        "Should find file with case-insensitive facet match"
    );
}

#[tokio::test]
async fn test_search_with_folder_scope() {
    let repo = TestRepo::new();
    repo.create_dir("docs");
    repo.create_dir("blog");
    repo.create_markdown("docs/guide.md", "# Guide\n\nDocumentation guide.");
    repo.create_markdown("blog/post.md", "# Post\n\nBlog post about guides.");

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    // Search everywhere
    let response = server
        .post_json(
            "/.mbr/search",
            r#"{"q": "guide", "folder_scope": "everywhere"}"#,
        )
        .await;
    let body: serde_json::Value = response.json().await.unwrap();
    let all_results = body["results"].as_array().unwrap().len();

    // Search in docs folder only
    let response = server
        .post_json(
            "/.mbr/search",
            r#"{"q": "guide", "folder": "/docs/", "folder_scope": "current"}"#,
        )
        .await;
    let body: serde_json::Value = response.json().await.unwrap();
    let docs_results = body["results"].as_array().unwrap();

    // Docs-only search should return fewer results
    assert!(
        docs_results.len() < all_results || all_results == 1,
        "Folder-scoped search should be more specific"
    );

    // Docs-only search should only contain /docs/ paths
    for r in docs_results {
        assert!(
            r["url_path"].as_str().unwrap().starts_with("/docs/"),
            "Result should be in /docs/: {}",
            r["url_path"]
        );
    }
}

#[tokio::test]
async fn test_search_arbitrary_frontmatter_field() {
    let repo = TestRepo::new();
    let mut frontmatter = HashMap::new();
    frontmatter.insert("author", "Alice Smith");
    frontmatter.insert("title", "An Article");
    repo.create_markdown_with_frontmatter("article.md", &frontmatter, "Content.");

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    // Should be able to search custom frontmatter fields
    let response = server
        .post_json("/.mbr/search", r#"{"q": "author:alice"}"#)
        .await;
    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.unwrap();
    assert!(
        body["total_matches"].as_i64().unwrap() >= 1,
        "Should find file by custom frontmatter field 'author'"
    );
}

#[tokio::test]
async fn test_search_mixed_terms_and_facets() {
    let repo = TestRepo::new();
    let mut frontmatter = HashMap::new();
    frontmatter.insert("category", "tutorial");
    frontmatter.insert("title", "Rust Async Tutorial");
    repo.create_markdown_with_frontmatter("async.md", &frontmatter, "Learn async in Rust.");

    let mut other_fm = HashMap::new();
    other_fm.insert("category", "tutorial");
    other_fm.insert("title", "Python Basics");
    repo.create_markdown_with_frontmatter("python.md", &other_fm, "Learn Python basics.");

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    // Search with both term and facet
    let response = server
        .post_json("/.mbr/search", r#"{"q": "rust category:tutorial"}"#)
        .await;
    let body: serde_json::Value = response.json().await.unwrap();
    let results = body["results"].as_array().unwrap();

    // Should find rust tutorial but not python tutorial
    assert!(
        results
            .iter()
            .any(|r| r["url_path"].as_str().unwrap().contains("async")),
        "Expected to find Rust tutorial: {:?}",
        results
    );
}

// ============================================================================
// Template Folder Tests
// ============================================================================

/// Helper to start a test server with template_folder option.
struct TestServerWithTemplates {
    port: u16,
    client: reqwest::Client,
    _handle: tokio::task::JoinHandle<()>,
}

impl TestServerWithTemplates {
    async fn start(repo: &TestRepo, template_folder: Option<std::path::PathBuf>) -> Self {
        let port = find_available_port();
        let root_dir = repo.path().to_path_buf();

        let handle = tokio::spawn(async move {
            let mut config = test_server_config(port, root_dir);
            config.template_folder = template_folder;
            let server = mbr::server::Server::init(config).expect("Failed to initialize server");

            server.start().await.expect("test server failed to start");
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let client = mbr::http_client(Duration::from_secs(5));

        Self {
            port,
            client,
            _handle: handle,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }

    async fn get(&self, path: &str) -> reqwest::Response {
        self.client
            .get(self.url(path))
            .send()
            .await
            .expect("Failed to make request")
    }

    async fn get_text(&self, path: &str) -> String {
        self.get(path)
            .await
            .text()
            .await
            .expect("Failed to get text")
    }
}

#[tokio::test]
async fn test_template_folder_serves_css() {
    let repo = TestRepo::new();

    // Create a custom template folder with a custom CSS file
    let template_dir = repo.path().join("custom-templates");
    std::fs::create_dir_all(&template_dir).unwrap();
    std::fs::write(
        template_dir.join("theme.css"),
        "/* Custom theme CSS */\nbody { color: red; }",
    )
    .unwrap();

    let server = TestServerWithTemplates::start(&repo, Some(template_dir)).await;
    let response = server.get("/.mbr/theme.css").await;

    assert_eq!(response.status(), 200);
    let body = response.text().await.unwrap();
    assert!(
        body.contains("Custom theme CSS"),
        "Should serve custom theme.css from template folder"
    );
}

#[tokio::test]
async fn test_template_folder_serves_js_from_js_subdir() {
    let repo = TestRepo::new();

    // Create a custom template folder with components-js/ subdirectory for components
    let template_dir = repo.path().join("custom-templates");
    std::fs::create_dir_all(template_dir.join("components-js")).unwrap();
    std::fs::write(
        template_dir.join("components-js/mbr-components.min.js"),
        "// Custom components JS",
    )
    .unwrap();

    let server = TestServerWithTemplates::start(&repo, Some(template_dir)).await;
    let response = server.get("/.mbr/components/mbr-components.min.js").await;

    assert_eq!(response.status(), 200);
    let body = response.text().await.unwrap();
    assert!(
        body.contains("Custom components JS"),
        "Should serve components from template_folder/components-js/"
    );
}

#[tokio::test]
async fn test_template_folder_falls_back_to_defaults() {
    let repo = TestRepo::new();

    // Create an empty template folder
    let template_dir = repo.path().join("custom-templates");
    std::fs::create_dir_all(&template_dir).unwrap();

    let server = TestServerWithTemplates::start(&repo, Some(template_dir)).await;

    // Request a file that's NOT in the template folder - should fall back to compiled default
    let response = server.get("/.mbr/pico.min.css").await;

    assert_eq!(response.status(), 200);
    let body = response.text().await.unwrap();
    // pico.min.css is a compiled-in default, should be served
    assert!(
        !body.is_empty(),
        "Should fall back to compiled default for missing files"
    );
}

#[tokio::test]
async fn test_template_folder_overrides_html_templates() {
    let repo = TestRepo::new();
    repo.create_markdown("test.md", "# Test Page");

    // Create a custom template folder with custom HTML
    let template_dir = repo.path().join("custom-templates");
    std::fs::create_dir_all(&template_dir).unwrap();
    std::fs::write(
        template_dir.join("index.html"),
        r#"<!DOCTYPE html>
<html>
<head><title>Custom Template</title></head>
<body>
<div class="custom-wrapper">{{ markdown | safe }}</div>
</body>
</html>"#,
    )
    .unwrap();

    let server = TestServerWithTemplates::start(&repo, Some(template_dir)).await;
    let html = server.get_text("/test/").await;

    assert!(
        html.contains("Custom Template"),
        "Should use custom HTML template"
    );
    assert!(
        html.contains("custom-wrapper"),
        "Should render with custom wrapper"
    );
    assert!(
        html.contains("<h1 id=\"test-page\">Test Page</h1>"),
        "Should still render markdown content"
    );
}

// ============================================================================
// Server Mode Configuration Tests
// ============================================================================

#[tokio::test]
async fn test_server_mode_sets_server_mode_true() {
    let repo = TestRepo::new();
    repo.create_markdown("test.md", "# Test");

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/test/").await;

    // Server mode should have serverMode: true
    assert!(
        html.contains("serverMode: true"),
        "Expected serverMode: true in server mode. Got: {}",
        &html[..std::cmp::min(2000, html.len())]
    );
    assert!(
        !html.contains("serverMode: false"),
        "Should not have serverMode: false in server mode"
    );
}

#[tokio::test]
async fn test_server_mode_includes_components() {
    let repo = TestRepo::new();
    repo.create_markdown("test.md", "# Test");

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/test/").await;

    // Should include the components script
    assert!(
        html.contains("mbr-components.min.js"),
        "Expected mbr-components.min.js script reference in HTML"
    );
}

#[tokio::test]
async fn test_site_json_returns_valid_structure() {
    let repo = TestRepo::new();
    repo.create_markdown("one.md", "# One");
    repo.create_markdown("two.md", "# Two");

    let server = TestServer::start(&repo).await;
    let response = server.get("/.mbr/site.json").await;

    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.unwrap();

    // Should have markdown_files array
    assert!(
        body["markdown_files"].is_array(),
        "Expected markdown_files array in site.json"
    );

    let files = body["markdown_files"].as_array().unwrap();
    assert!(
        files.len() >= 2,
        "Expected at least 2 files in markdown_files"
    );

    // Each file should have required fields
    for file in files {
        assert!(file["url_path"].is_string(), "Expected url_path in file");
        assert!(file["raw_path"].is_string(), "Expected raw_path in file");
        assert!(
            file["created"].is_number(),
            "Expected created timestamp in file"
        );
        assert!(
            file["modified"].is_number(),
            "Expected modified timestamp in file"
        );
    }
}

/// `raw_path` is a public customization API (`file.raw_path.split('/').pop()`),
/// so it must be repo-relative and `/`-separated on every platform, and must
/// never leak the host's absolute directory layout into a published site.
#[tokio::test]
async fn test_site_json_raw_path_is_relative_and_slash_separated() {
    let repo = TestRepo::new();
    repo.create_markdown("top.md", "# Top");
    repo.create_markdown("docs/guide.md", "# Guide");
    repo.create_markdown("docs/deep/nested/page.md", "# Nested");
    repo.create_markdown("docs/index.md", "# Docs Index");

    let server = TestServer::start(&repo).await;
    let body: serde_json::Value = server.get("/.mbr/site.json").await.json().await.unwrap();

    // The absolute root, in the form it would appear if it leaked.
    let root_str = repo.path().to_string_lossy().to_string();

    let raw_paths: Vec<String> = body["markdown_files"]
        .as_array()
        .expect("markdown_files array")
        .iter()
        .map(|f| {
            f["raw_path"]
                .as_str()
                .expect("raw_path should be a string")
                .to_string()
        })
        .collect();

    assert!(!raw_paths.is_empty(), "expected markdown files");

    for raw in &raw_paths {
        assert!(
            !raw.contains('\\'),
            "raw_path must be `/`-separated, got {raw}"
        );
        assert!(
            !raw.contains(&root_str),
            "raw_path must not leak the absolute root {root_str}, got {raw}"
        );
        // Relative, and (for this fixture, whose files all live under the root)
        // without upward traversal.
        assert!(
            !raw.starts_with('/') && !raw.starts_with(".."),
            "raw_path must be repo-relative, got {raw}"
        );
        // Windows absolute paths start with a drive letter rather than `/`.
        assert!(
            !raw.contains(':'),
            "raw_path must not contain a drive prefix, got {raw}"
        );
    }

    let mut sorted = raw_paths.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        vec![
            "docs/deep/nested/page.md".to_string(),
            "docs/guide.md".to_string(),
            "docs/index.md".to_string(),
            "top.md".to_string(),
        ],
        "raw_path values should be exactly the repo-relative source paths"
    );

    // The documented customization idiom must still recover the file name.
    let names: Vec<&str> = raw_paths
        .iter()
        .map(|p| p.rsplit('/').next().unwrap())
        .collect();
    assert!(
        names.contains(&"index.md"),
        "index detection must still work"
    );
}

/// Creates a repo containing one of every payload-reachable file kind, so a
/// whole-document leak scan actually exercises the `other_files` /
/// `StaticFileMetadata` surface rather than only `markdown_files`.
///
/// An earlier version of the leak test used a markdown-only fixture, which left
/// `other_files` empty and missed an absolute path in `metadata.path`.
fn repo_with_all_file_kinds() -> TestRepo {
    let repo = TestRepo::new();
    repo.create_markdown("note.md", "# Note");
    repo.create_markdown("docs/guide.md", "# Guide");
    repo.create_markdown("docs/index.md", "# Docs");
    repo.create_static_file("images/pic.png", b"\x89PNG\r\n\x1a\nfake");
    repo.create_static_file("images/nested/deep.jpg", b"\xff\xd8\xfffake");
    repo.create_static_file("docs/report.pdf", b"%PDF-1.4 fake");
    repo.create_static_file("videos/clip.mp4", b"\x00\x00\x00\x18ftypmp42");
    repo.create_static_file("audio/sound.mp3", b"ID3fake");
    repo.create_static_file("data/notes.txt", b"some searchable text");
    repo.create_static_file("data/blob.bin", b"\x00\x01\x02");
    repo
}

/// Asserts no value anywhere in `json` contains the repo's absolute root.
///
/// Walks the parsed document rather than substring-scanning the raw text so a
/// failure names the exact JSON path, and checks both the plain and
/// JSON-escaped forms (a Windows path is embedded with doubled backslashes).
fn assert_no_absolute_paths(label: &str, body: &str, root: &std::path::Path) {
    let root_str = root.to_string_lossy().to_string();
    let escaped = root_str.replace('\\', "\\\\");

    let parsed: serde_json::Value =
        serde_json::from_str(body).unwrap_or_else(|e| panic!("{label} should be valid JSON: {e}"));

    fn walk(node: &serde_json::Value, path: &str, needle: &str, hits: &mut Vec<String>) {
        match node {
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    walk(v, &format!("{path}.{k}"), needle, hits);
                }
            }
            serde_json::Value::Array(items) => {
                for (i, v) in items.iter().enumerate() {
                    walk(v, &format!("{path}[{i}]"), needle, hits);
                }
            }
            serde_json::Value::String(s) if s.contains(needle) => {
                hits.push(format!("{path} = {s}"));
            }
            _ => {}
        }
    }

    let mut hits = Vec::new();
    walk(&parsed, "", &root_str, &mut hits);
    assert!(
        hits.is_empty(),
        "{label} leaked the absolute repo root at:\n  {}",
        hits.join("\n  ")
    );

    assert!(
        !body.contains(&escaped),
        "{label} leaked the escaped absolute repo root {escaped}"
    );
}

/// No absolute host path may appear anywhere in `site.json`. Guards the whole
/// document, including the `other_files` / `StaticFileMetadata` surface.
#[tokio::test]
async fn test_site_json_contains_no_absolute_host_paths() {
    let repo = repo_with_all_file_kinds();
    let server = TestServer::start(&repo).await;
    let body = server.get("/.mbr/site.json").await.text().await.unwrap();

    assert_no_absolute_paths("site.json", &body, repo.path());
}

/// Same guarantee for `media.json`, which is where server mode serves
/// `other_files` (site.json strips them). This is the payload that actually
/// carries `StaticFileMetadata`, so without this test the struct is unchecked.
#[tokio::test]
async fn test_media_json_contains_no_absolute_host_paths() {
    let repo = repo_with_all_file_kinds();
    let server = TestServer::start(&repo).await;
    let body = server.get("/.mbr/media.json").await.text().await.unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    let other = parsed["other_files"]
        .as_array()
        .expect("media.json should expose other_files");
    assert!(
        !other.is_empty(),
        "fixture must actually produce other_files, otherwise this test proves nothing"
    );

    assert_no_absolute_paths("media.json", &body, repo.path());
}

/// Prev/next sibling navigation is computed by matching each file's parent
/// directory against the current page's. Both sides must use the same
/// representation; when `raw_path` was absolute and the current page's parent
/// was repo-relative they never matched and the links silently vanished.
#[tokio::test]
async fn test_sibling_navigation_links_are_populated() {
    let repo = TestRepo::new();
    repo.create_markdown("docs/a.md", "# Alpha");
    repo.create_markdown("docs/b.md", "# Bravo");
    repo.create_markdown("docs/c.md", "# Charlie");
    // A file in a different folder must not appear as a sibling.
    repo.create_markdown("other/z.md", "# Zulu");

    let server = TestServer::start(&repo).await;
    let html = server.get("/docs/b/").await.text().await.unwrap();

    assert!(
        html.contains("prevPage") || html.contains("nextPage"),
        "expected sibling navigation to be emitted for a page with siblings"
    );
    assert!(
        html.contains("/docs/a/") && html.contains("/docs/c/"),
        "expected both siblings to be linked, got:\n{html}"
    );
    assert!(
        !html.contains("/other/z/"),
        "a file in another folder must not be treated as a sibling"
    );
}

#[tokio::test]
async fn test_site_json_includes_frontmatter() {
    let repo = TestRepo::new();
    let mut frontmatter = std::collections::HashMap::new();
    frontmatter.insert("title", "My Title");
    frontmatter.insert("tags", "rust, web");
    repo.create_markdown_with_frontmatter("tagged.md", &frontmatter, "Content here.");

    let server = TestServer::start(&repo).await;
    let body: serde_json::Value = server.get("/.mbr/site.json").await.json().await.unwrap();

    let files = body["markdown_files"].as_array().unwrap();
    let tagged_file = files
        .iter()
        .find(|f| f["url_path"].as_str().unwrap().contains("tagged"));

    assert!(
        tagged_file.is_some(),
        "Expected to find tagged.md in site.json"
    );

    let tagged = tagged_file.unwrap();
    assert!(
        tagged["frontmatter"].is_object(),
        "Expected frontmatter object"
    );
    assert_eq!(tagged["frontmatter"]["title"].as_str(), Some("My Title"));
}

#[tokio::test]
async fn test_site_json_other_files_kind_structure() {
    // Test T013: Verify other_files include metadata.kind with type structure
    // Note: other_files are served via /.mbr/media.json (not site.json) in server mode
    let repo = TestRepo::new();

    // Create a video file (will be classified as video type)
    repo.create_dir("videos");
    repo.create_static_file("videos/demo.mp4", b"fake video data");

    // Create an image file
    repo.create_dir("images");
    repo.create_static_file("images/photo.jpg", b"fake jpg data");

    // Create a PDF file
    repo.create_static_file("document.pdf", b"%PDF-1.4 fake pdf");

    let server = TestServer::start(&repo).await;

    // Verify site.json no longer contains other_files in server mode
    let site_body: serde_json::Value = server.get("/.mbr/site.json").await.json().await.unwrap();
    assert!(
        site_body["other_files"].is_null(),
        "site.json should NOT contain other_files in server mode"
    );

    // Fetch media.json which has other_files
    let body: serde_json::Value = server.get("/.mbr/media.json").await.json().await.unwrap();

    // Should have other_files array
    assert!(
        body["other_files"].is_array(),
        "Expected other_files array in media.json"
    );

    let other_files = body["other_files"].as_array().unwrap();
    assert!(
        !other_files.is_empty(),
        "Expected at least one file in other_files"
    );

    // Find the video file
    let video_file = other_files
        .iter()
        .find(|f| f["url_path"].as_str().unwrap_or("").contains("demo.mp4"));

    assert!(
        video_file.is_some(),
        "Expected to find demo.mp4 in other_files"
    );

    let video = video_file.unwrap();
    // Verify the metadata.kind structure has a "type" field
    assert!(
        video["metadata"]["kind"]["type"].is_string(),
        "Expected metadata.kind.type to be a string in video file"
    );
    assert_eq!(
        video["metadata"]["kind"]["type"].as_str(),
        Some("video"),
        "Expected video file to have kind.type = 'video'"
    );

    // Find the image file
    let image_file = other_files
        .iter()
        .find(|f| f["url_path"].as_str().unwrap_or("").contains("photo.jpg"));

    assert!(
        image_file.is_some(),
        "Expected to find photo.jpg in other_files"
    );

    let image = image_file.unwrap();
    assert_eq!(
        image["metadata"]["kind"]["type"].as_str(),
        Some("image"),
        "Expected image file to have kind.type = 'image'"
    );

    // Find the PDF file
    let pdf_file = other_files.iter().find(|f| {
        f["url_path"]
            .as_str()
            .unwrap_or("")
            .contains("document.pdf")
    });

    assert!(
        pdf_file.is_some(),
        "Expected to find document.pdf in other_files"
    );

    let pdf = pdf_file.unwrap();
    assert_eq!(
        pdf["metadata"]["kind"]["type"].as_str(),
        Some("pdf"),
        "Expected PDF file to have kind.type = 'pdf'"
    );
}

#[tokio::test]
async fn test_site_json_srt_file_classified_as_text() {
    // Test T014: Verify .srt files are classified as text type
    // Note: other_files are served via /.mbr/media.json (not site.json) in server mode
    let repo = TestRepo::new();

    // Create an .srt subtitle file
    repo.create_dir("subtitles");
    repo.create_static_file(
        "subtitles/movie.srt",
        b"1\n00:00:01,000 --> 00:00:04,000\nSubtitle text here",
    );

    // Also create a .vtt file to verify other text types
    repo.create_static_file(
        "subtitles/movie.vtt",
        b"WEBVTT\n\n00:00:01.000 --> 00:00:04.000\nSubtitle text here",
    );

    let server = TestServer::start(&repo).await;
    let body: serde_json::Value = server.get("/.mbr/media.json").await.json().await.unwrap();

    let other_files = body["other_files"].as_array().unwrap();

    // Find the .srt file
    let srt_file = other_files
        .iter()
        .find(|f| f["url_path"].as_str().unwrap_or("").contains("movie.srt"));

    assert!(
        srt_file.is_some(),
        "Expected to find movie.srt in other_files"
    );

    let srt = srt_file.unwrap();
    assert!(
        srt["metadata"]["kind"]["type"].is_string(),
        "Expected metadata.kind.type to be a string in srt file"
    );
    assert_eq!(
        srt["metadata"]["kind"]["type"].as_str(),
        Some("text"),
        "Expected .srt file to have kind.type = 'text'"
    );

    // Verify .vtt is also classified as text
    let vtt_file = other_files
        .iter()
        .find(|f| f["url_path"].as_str().unwrap_or("").contains("movie.vtt"));

    assert!(
        vtt_file.is_some(),
        "Expected to find movie.vtt in other_files"
    );

    let vtt = vtt_file.unwrap();
    assert_eq!(
        vtt["metadata"]["kind"]["type"].as_str(),
        Some("text"),
        "Expected .vtt file to have kind.type = 'text'"
    );
}

// ============================================================================
// HTTP Range Request Tests
// ============================================================================

#[tokio::test]
async fn test_range_request_partial_content() {
    let repo = TestRepo::new();
    // Create a file with known content for byte-level verification
    let content = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    repo.create_static_file("video.bin", content);

    let server = TestServer::start(&repo).await;

    // Request bytes 10-19 (inclusive)
    let response = server
        .client
        .get(server.url("/video.bin"))
        .header("Range", "bytes=10-19")
        .send()
        .await
        .expect("Request failed");

    // Should return 206 Partial Content
    assert_eq!(
        response.status(),
        206,
        "Expected 206 Partial Content for range request"
    );

    // Should have Content-Range header
    let content_range = response
        .headers()
        .get("content-range")
        .expect("Expected Content-Range header");
    assert!(
        content_range.to_str().unwrap().contains("bytes 10-19/36"),
        "Content-Range should indicate bytes 10-19 of 36. Got: {:?}",
        content_range
    );

    // Should return exactly the requested bytes
    let body = response.bytes().await.unwrap();
    assert_eq!(body.as_ref(), b"ABCDEFGHIJ", "Body should be bytes 10-19");
}

#[tokio::test]
async fn test_range_request_suffix() {
    let repo = TestRepo::new();
    let content = b"0123456789ABCDEFGHIJ";
    repo.create_static_file("data.bin", content);

    let server = TestServer::start(&repo).await;

    // Request last 5 bytes
    let response = server
        .client
        .get(server.url("/data.bin"))
        .header("Range", "bytes=-5")
        .send()
        .await
        .expect("Request failed");

    assert_eq!(response.status(), 206);

    let body = response.bytes().await.unwrap();
    assert_eq!(body.as_ref(), b"FGHIJ", "Should return last 5 bytes");
}

#[tokio::test]
async fn test_range_request_from_offset() {
    let repo = TestRepo::new();
    let content = b"0123456789ABCDEFGHIJ";
    repo.create_static_file("data.bin", content);

    let server = TestServer::start(&repo).await;

    // Request from byte 15 to end
    let response = server
        .client
        .get(server.url("/data.bin"))
        .header("Range", "bytes=15-")
        .send()
        .await
        .expect("Request failed");

    assert_eq!(response.status(), 206);

    let body = response.bytes().await.unwrap();
    assert_eq!(body.as_ref(), b"FGHIJ", "Should return bytes 15 to end");
}

#[tokio::test]
async fn test_range_request_accept_ranges_header() {
    let repo = TestRepo::new();
    repo.create_static_file("file.bin", b"content");

    let server = TestServer::start(&repo).await;

    // Regular request (no Range header) should advertise Accept-Ranges
    let response = server.get("/file.bin").await;

    assert_eq!(response.status(), 200);

    let accept_ranges = response.headers().get("accept-ranges");
    assert!(
        accept_ranges.is_some(),
        "Expected Accept-Ranges header to advertise range support"
    );
    assert_eq!(
        accept_ranges.unwrap().to_str().unwrap(),
        "bytes",
        "Accept-Ranges should be 'bytes'"
    );
}

/// Regression: `CompressionLayer::new()`'s default predicate only excludes
/// gRPC, images and SSE, so a plain `GET` of an `.mp4` with
/// `Accept-Encoding: gzip` used to come back `content-encoding: gzip` +
/// `transfer-encoding: chunked` with **no** `content-length` and **no**
/// `accept-ranges` (tower-http strips both when it compresses). That destroys
/// seeking and duration detection in any client that negotiates gzip — and
/// burns CPU gzipping already-compressed H.264.
#[tokio::test]
async fn test_video_is_never_gzipped() {
    let repo = TestRepo::new();
    // Well above the 32-byte SizeAbove floor so compression is genuinely on
    // the table for this response.
    repo.create_static_file("clip.mp4", &vec![b'A'; 64 * 1024]);

    let server = TestServer::start(&repo).await;

    let response = server
        .client
        .get(server.url("/clip.mp4"))
        .header("Accept-Encoding", "gzip")
        .send()
        .await
        .expect("Request failed");

    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers().get("content-encoding"),
        None,
        "video must not be gzipped, got headers: {:?}",
        response.headers()
    );
    assert_eq!(
        response
            .headers()
            .get("accept-ranges")
            .and_then(|v| v.to_str().ok()),
        Some("bytes"),
        "compression strips accept-ranges, which breaks seeking"
    );
    assert_eq!(
        response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok()),
        Some("65536"),
        "compression strips content-length, which breaks duration detection"
    );

    let body = response.bytes().await.unwrap();
    assert_eq!(
        body.len(),
        64 * 1024,
        "body must be the raw, unmodified file"
    );
}

/// The exclusion must be narrow: HTML still gets compressed.
#[tokio::test]
async fn test_html_is_still_gzipped() {
    let repo = TestRepo::new();
    repo.create_markdown(
        "page.md",
        &format!("# Page\n\n{}", "lorem ipsum ".repeat(500)),
    );

    let server = TestServer::start(&repo).await;

    let response = server
        .client
        .get(server.url("/page/"))
        .header("Accept-Encoding", "gzip")
        .send()
        .await
        .expect("Request failed");

    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get("content-encoding")
            .and_then(|v| v.to_str().ok()),
        Some("gzip"),
        "markdown pages should still be compressed, got headers: {:?}",
        response.headers()
    );
}

#[tokio::test]
async fn test_range_request_invalid_range() {
    let repo = TestRepo::new();
    repo.create_static_file("small.bin", b"tiny");

    let server = TestServer::start(&repo).await;

    // Request beyond file size
    let response = server
        .client
        .get(server.url("/small.bin"))
        .header("Range", "bytes=100-200")
        .send()
        .await
        .expect("Request failed");

    // Should return 416 Range Not Satisfiable
    assert_eq!(
        response.status(),
        416,
        "Expected 416 Range Not Satisfiable for invalid range"
    );
}

// ============================================================================
// Cache Headers Tests
// ============================================================================

#[tokio::test]
async fn test_cache_headers_on_markdown() {
    let repo = TestRepo::new();
    repo.create_markdown("page.md", "# Test Page");

    let server = TestServer::start(&repo).await;
    let response = server.get("/page/").await;

    assert_eq!(response.status(), 200);

    // Check Cache-Control header
    let cache_control = response.headers().get("cache-control");
    assert!(
        cache_control.is_some(),
        "Markdown pages should have Cache-Control header"
    );
    assert_eq!(cache_control.unwrap().to_str().unwrap(), "no-cache");

    // Check ETag header
    let etag = response.headers().get("etag");
    assert!(etag.is_some(), "Markdown pages should have ETag header");
    let etag_value = etag.unwrap().to_str().unwrap();
    assert!(
        etag_value.starts_with("W/\""),
        "ETag should be weak (W/\"...\")"
    );

    // Check Last-Modified header
    let last_modified = response.headers().get("last-modified");
    assert!(
        last_modified.is_some(),
        "Markdown pages should have Last-Modified header"
    );
}

#[tokio::test]
async fn test_cache_headers_on_default_assets() {
    let repo = TestRepo::new();

    let server = TestServer::start(&repo).await;
    let response = server.get("/.mbr/theme.css").await;

    assert_eq!(response.status(), 200);

    // Check Cache-Control header
    let cache_control = response.headers().get("cache-control");
    assert!(
        cache_control.is_some(),
        "Default assets should have Cache-Control header"
    );
    assert_eq!(cache_control.unwrap().to_str().unwrap(), "no-cache");

    // Check ETag header
    let etag = response.headers().get("etag");
    assert!(etag.is_some(), "Default assets should have ETag header");
}

#[tokio::test]
async fn test_cache_headers_on_static_files() {
    let repo = TestRepo::new();
    repo.create_static_file("test.txt", b"Static file content");

    let server = TestServer::start(&repo).await;
    let response = server.get("/test.txt").await;

    assert_eq!(response.status(), 200);

    // Check Cache-Control header
    let cache_control = response.headers().get("cache-control");
    assert!(
        cache_control.is_some(),
        "Static files should have Cache-Control header"
    );
    assert_eq!(cache_control.unwrap().to_str().unwrap(), "no-cache");
}

#[tokio::test]
async fn test_cache_headers_on_directory_listing() {
    let repo = TestRepo::new();
    repo.create_dir("docs");
    repo.create_markdown("docs/one.md", "# One");
    repo.create_markdown("docs/two.md", "# Two");

    let server = TestServer::start(&repo).await;
    let response = server.get("/docs/").await;

    assert_eq!(response.status(), 200);

    // Directory listings should use no-store since they're truly dynamic
    let cache_control = response.headers().get("cache-control");
    assert!(
        cache_control.is_some(),
        "Directory listings should have Cache-Control header"
    );
    assert_eq!(cache_control.unwrap().to_str().unwrap(), "no-store");

    // Should still have ETag
    let etag = response.headers().get("etag");
    assert!(etag.is_some(), "Directory listings should have ETag header");
}

#[tokio::test]
async fn test_etag_changes_with_content() {
    let repo = TestRepo::new();
    repo.create_markdown("mutable.md", "# Original Content");

    let server = TestServer::start(&repo).await;

    // Get first ETag
    let response1 = server.get("/mutable/").await;
    let etag1 = response1
        .headers()
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // Modify the file
    std::fs::write(repo.path().join("mutable.md"), "# Modified Content").unwrap();

    // Small delay to ensure file is written
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Get second ETag - should be different
    let response2 = server.get("/mutable/").await;
    let etag2 = response2
        .headers()
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    assert_ne!(etag1, etag2, "ETag should change when content changes");
}

// ==================== Video Enhancement Tests ====================

#[tokio::test]
async fn test_components_js_bundle_served() {
    let repo = TestRepo::new();

    let server = TestServer::start(&repo).await;
    let response = server.get("/.mbr/components/mbr-components.min.js").await;

    assert_eq!(
        response.status(),
        200,
        "Components JS bundle should be served at /.mbr/components/mbr-components.min.js"
    );

    let content_type = response.headers().get("content-type").unwrap();
    assert!(
        content_type.to_str().unwrap().contains("javascript"),
        "Should have javascript content type"
    );
}

#[tokio::test]
async fn test_graph_chunks_served() {
    // The lazy-loaded mini-graph, genealogy and task-panel chunks are compiled
    // into the binary (DEFAULT_FILES) and must be served alongside the main
    // bundle.
    let repo = TestRepo::new();

    let server = TestServer::start(&repo).await;

    for path in [
        "/.mbr/components/mbr-graph.min.js",
        "/.mbr/components/mbr-genealogy.min.js",
        "/.mbr/components/mbr-tasks.min.js",
    ] {
        let response = server.get(path).await;
        assert_eq!(response.status(), 200, "Chunk should be served at {path}");

        let content_type = response.headers().get("content-type").unwrap();
        assert!(
            content_type.to_str().unwrap().contains("javascript"),
            "{path} should have javascript content type"
        );
    }
}

#[tokio::test]
async fn test_components_js_bundle_no_missing_imports() {
    // This test verifies that the JS bundle doesn't try to dynamically import
    // files that aren't served, which would cause 404 errors in the browser.
    let repo = TestRepo::new();

    let server = TestServer::start(&repo).await;
    let js_content = server
        .get_text("/.mbr/components/mbr-components.min.js")
        .await;

    // Relative dynamic imports (e.g. import("./main-abc123.js")) indicate
    // unintended vite code splitting into hashed chunks that are never served.
    assert!(
        !js_content.contains(r#"import("./"#) && !js_content.contains(r#"import('./"#),
        "Components bundle contains relative dynamic imports (unintended code \
         splitting). Check vite.config.ts inlineDynamicImports."
    );

    // The `<mbr-editor>` trigger intentionally lazy-loads the heavy Crepe editor
    // chunk via a runtime dynamic import. That chunk is explicitly served (see
    // DEFAULT_FILES in server.rs), so it is allowed — but any OTHER absolute
    // `/.mbr/components/` chunk import would be an unserved code-split artifact.
    const EDITOR_CHUNK: &str = "/.mbr/components/mbr-editor.min.js";
    let stripped = js_content
        .replace(&format!(r#"import("{EDITOR_CHUNK}")"#), "")
        .replace(&format!(r#"import('{EDITOR_CHUNK}')"#), "");
    assert!(
        !stripped.contains(r#"import("/.mbr/components/"#)
            && !stripped.contains(r#"import('/.mbr/components/"#),
        "Components bundle imports an unexpected /.mbr/components/ chunk that may \
         not be served."
    );

    // If the editor chunk is referenced, confirm it is actually served.
    if js_content.contains(EDITOR_CHUNK) {
        let resp = server.get(EDITOR_CHUNK).await;
        assert_eq!(
            resp.status(),
            200,
            "The lazy-loaded editor chunk must be served at {EDITOR_CHUNK}"
        );
    }

    // The mini-graph, genealogy and task-panel chunks are lazy-loaded through
    // runtime-computed URLs (asset base + "components/<chunk>.min.js"), so
    // they never appear as literal import() targets. If the bundle references
    // any of those chunk filenames, that chunk must actually be served.
    for chunk in [
        "mbr-graph.min.js",
        "mbr-genealogy.min.js",
        "mbr-tasks.min.js",
    ] {
        if js_content.contains(chunk) {
            let path = format!("/.mbr/components/{chunk}");
            let resp = server.get(&path).await;
            assert_eq!(
                resp.status(),
                200,
                "The lazy-loaded chunk referenced by the bundle must be served at {path}"
            );
        }
    }
}

#[tokio::test]
async fn test_video_in_markdown_gets_video_tag() {
    let repo = TestRepo::new();
    // Use image syntax to embed a video (which gets converted to <video> tag)
    repo.create_markdown(
        "video-page.md",
        "# Video Page\n\n![My Video](test.mp4)\n\nSome text after.",
    );

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/video-page/").await;

    // The markdown should render a <video> element
    assert!(
        html.contains("<video"),
        "Page with video should have <video> element. Got: {}",
        html
    );
}

// ============================================================================
// Error page tests
// ============================================================================

#[tokio::test]
async fn test_404_returns_error_page_html() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello World");

    let server = TestServer::start(&repo).await;
    let response = server.get("/non-existent-page/").await;

    // Should return 404 status
    assert_eq!(response.status(), 404);

    let html = response.text().await.expect("Failed to get response text");

    // Should contain error page structure
    assert!(
        html.contains("<h1>404</h1>"),
        "Error page should display 404 code"
    );
    assert!(
        html.contains("Not Found"),
        "Error page should display 'Not Found' title"
    );
}

#[tokio::test]
async fn test_404_error_page_shows_requested_url() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello World");

    let server = TestServer::start(&repo).await;
    let html = server
        .get_text("/some/deep/path/that/does/not/exist/")
        .await;

    // Error page should show the requested URL (slashes may be HTML-encoded as &#x2F;)
    // Check for unique path segments that will be present regardless of encoding
    assert!(
        html.contains("some")
            && html.contains("deep")
            && html.contains("path")
            && html.contains("exist"),
        "Error page should show the requested URL. Got: {}",
        html
    );
}

#[tokio::test]
async fn test_404_error_page_includes_navigation() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello World");

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/missing-page/").await;

    // Error page should have navigation elements
    assert!(
        html.contains("Go Back") || html.contains("history.back"),
        "Error page should have a back button"
    );
    assert!(
        html.contains("Home") || html.contains("href=\"/\""),
        "Error page should have a home link"
    );
    assert!(
        html.contains("mbr-search") || html.contains("search"),
        "Error page should suggest using search"
    );
}

#[tokio::test]
async fn test_404_error_page_has_proper_content_type() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello World");

    let server = TestServer::start(&repo).await;
    let response = server.get("/non-existent/").await;

    // Should return HTML content type
    let content_type = response
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap_or(""));
    assert!(
        content_type.is_some_and(|ct| ct.contains("text/html")),
        "Error page should have text/html content type. Got: {:?}",
        content_type
    );
}

// ============================================================================
// Theme Configuration Tests
// ============================================================================

/// Helper to start a server with a specific theme
struct TestServerWithTheme {
    port: u16,
    client: reqwest::Client,
    _handle: tokio::task::JoinHandle<()>,
}

impl TestServerWithTheme {
    async fn start(repo: &TestRepo, theme: &str) -> Self {
        let port = find_available_port();
        let root_dir = repo.path().to_path_buf();
        let theme = theme.to_string();

        let handle = tokio::spawn(async move {
            let mut config = test_server_config(port, root_dir);
            config.theme = theme;
            let server = mbr::server::Server::init(config).expect("Failed to initialize server");

            server.start().await.expect("test server failed to start");
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let client = mbr::http_client(Duration::from_secs(5));

        Self {
            port,
            client,
            _handle: handle,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }

    async fn get(&self, path: &str) -> reqwest::Response {
        self.client
            .get(self.url(path))
            .send()
            .await
            .expect("Request failed")
    }

    #[allow(dead_code)]
    async fn get_text(&self, path: &str) -> String {
        self.get(path)
            .await
            .text()
            .await
            .expect("Failed to get response text")
    }
}

#[tokio::test]
async fn test_pico_css_default_theme() {
    let repo = TestRepo::new();

    let server = TestServerWithTheme::start(&repo, "default").await;
    let response = server.get("/.mbr/pico.min.css").await;

    assert_eq!(response.status(), 200);

    // Default theme should return the classless variant
    let css = response.text().await.unwrap();
    assert!(
        css.contains("Pico CSS") || css.len() > 1000,
        "Should return valid Pico CSS"
    );
}

#[tokio::test]
async fn test_pico_css_color_theme() {
    let repo = TestRepo::new();

    let server = TestServerWithTheme::start(&repo, "amber").await;
    let response = server.get("/.mbr/pico.min.css").await;

    assert_eq!(response.status(), 200);

    // Color theme should return CSS with different content than default
    let css = response.text().await.unwrap();
    assert!(
        css.len() > 1000,
        "Amber theme should return valid CSS content"
    );
}

#[tokio::test]
async fn test_pico_css_fluid_theme() {
    let repo = TestRepo::new();

    let server = TestServerWithTheme::start(&repo, "fluid").await;
    let response = server.get("/.mbr/pico.min.css").await;

    assert_eq!(response.status(), 200);

    let css = response.text().await.unwrap();
    assert!(css.len() > 1000, "Fluid theme should return valid CSS");
}

#[tokio::test]
async fn test_pico_css_fluid_color_theme() {
    let repo = TestRepo::new();

    let server = TestServerWithTheme::start(&repo, "fluid.jade").await;
    let response = server.get("/.mbr/pico.min.css").await;

    assert_eq!(response.status(), 200);

    let css = response.text().await.unwrap();
    assert!(css.len() > 1000, "Fluid jade theme should return valid CSS");
}

#[tokio::test]
async fn test_pico_css_invalid_theme_returns_404() {
    let repo = TestRepo::new();

    let server = TestServerWithTheme::start(&repo, "invalid-theme-name").await;
    let response = server.get("/.mbr/pico.min.css").await;

    // Invalid theme should return 404
    assert_eq!(
        response.status(),
        404,
        "Invalid theme should return 404 status"
    );
}

#[tokio::test]
async fn test_pico_css_empty_theme_uses_default() {
    let repo = TestRepo::new();

    let server = TestServerWithTheme::start(&repo, "").await;
    let response = server.get("/.mbr/pico.min.css").await;

    assert_eq!(response.status(), 200);

    let css = response.text().await.unwrap();
    assert!(css.len() > 1000, "Empty theme should use default CSS");
}

#[tokio::test]
async fn test_pico_css_has_cache_headers() {
    let repo = TestRepo::new();

    let server = TestServerWithTheme::start(&repo, "default").await;
    let response = server.get("/.mbr/pico.min.css").await;

    assert_eq!(response.status(), 200);

    // Check cache headers
    let etag = response.headers().get("etag");
    assert!(etag.is_some(), "Pico CSS response should have ETag header");

    let cache_control = response.headers().get("cache-control");
    assert!(
        cache_control.is_some(),
        "Pico CSS response should have Cache-Control header"
    );
}

// ============================================================================
// Link Tracking Tests (links.json endpoint)
// ============================================================================

#[tokio::test]
async fn test_links_json_returns_valid_structure() {
    let repo = TestRepo::new();
    repo.create_markdown("page.md", "# Test Page\n\n[Other](other/)");
    repo.create_markdown("other.md", "# Other Page");

    let server = TestServer::start(&repo).await;
    let response = server.get("/page/links.json").await;

    assert_eq!(response.status(), 200);

    let content_type = response.headers().get("content-type").unwrap();
    assert!(
        content_type.to_str().unwrap().contains("application/json"),
        "links.json should return JSON content type"
    );

    let body: serde_json::Value = response.json().await.unwrap();

    // Should have inbound and outbound arrays
    assert!(
        body["inbound"].is_array(),
        "Expected 'inbound' array in links.json"
    );
    assert!(
        body["outbound"].is_array(),
        "Expected 'outbound' array in links.json"
    );
}

#[tokio::test]
async fn test_links_json_contains_outbound_links() {
    let repo = TestRepo::new();
    repo.create_markdown(
        "source.md",
        "# Source\n\n[Link to Target](target/)\n\n[External](https://example.com)",
    );
    repo.create_markdown("target.md", "# Target Page");

    let server = TestServer::start(&repo).await;
    let body: serde_json::Value = server.get("/source/links.json").await.json().await.unwrap();

    let outbound = body["outbound"].as_array().unwrap();

    // Should have at least one internal link
    let has_internal = outbound.iter().any(|l| {
        l["to"].as_str().unwrap().contains("target") && l["internal"].as_bool() == Some(true)
    });
    assert!(
        has_internal,
        "Should have internal link to target: {:?}",
        outbound
    );

    // Should have external link
    let has_external = outbound.iter().any(|l| {
        l["to"].as_str().unwrap().contains("example.com") && l["internal"].as_bool() == Some(false)
    });
    assert!(
        has_external,
        "Should have external link to example.com: {:?}",
        outbound
    );
}

#[tokio::test]
async fn test_links_json_contains_inbound_links() {
    let repo = TestRepo::new();
    // Create source page that links to target
    repo.create_markdown("source.md", "# Source\n\n[See the Target](target/)");
    // Create target page
    repo.create_markdown("target.md", "# Target Page");

    let server = TestServer::start(&repo).await;

    // First, load the source page to populate the link cache
    let _ = server.get("/source/").await;

    // Now get links for the target page - should show inbound link from source
    let body: serde_json::Value = server.get("/target/links.json").await.json().await.unwrap();

    let inbound = body["inbound"].as_array().unwrap();

    let has_inbound_from_source = inbound
        .iter()
        .any(|l| l["from"].as_str().unwrap().contains("source"));
    assert!(
        has_inbound_from_source,
        "Target should have inbound link from source: {:?}",
        inbound
    );
}

#[tokio::test]
async fn test_links_json_outbound_includes_anchor() {
    let repo = TestRepo::new();
    repo.create_markdown(
        "page.md",
        "# Page\n\n[Section Link](other/#section-heading)",
    );
    repo.create_markdown("other.md", "# Other\n\n## Section Heading");

    let server = TestServer::start(&repo).await;
    let body: serde_json::Value = server.get("/page/links.json").await.json().await.unwrap();

    let outbound = body["outbound"].as_array().unwrap();

    let link_with_anchor = outbound
        .iter()
        .find(|l| l["to"].as_str().unwrap().contains("other"));
    assert!(
        link_with_anchor.is_some(),
        "Should have link to other page: {:?}",
        outbound
    );

    let anchor = link_with_anchor.unwrap()["anchor"].as_str();
    assert!(
        anchor.is_some() && anchor.unwrap().contains("section"),
        "Link should have anchor: {:?}",
        link_with_anchor
    );
}

#[tokio::test]
async fn test_links_json_404_for_nonexistent_page() {
    let repo = TestRepo::new();
    repo.create_markdown("exists.md", "# Exists");

    let server = TestServer::start(&repo).await;
    let response = server.get("/nonexistent/links.json").await;

    assert_eq!(
        response.status(),
        404,
        "links.json for nonexistent page should return 404"
    );
}

#[tokio::test]
async fn test_links_json_empty_for_page_with_no_links() {
    let repo = TestRepo::new();
    repo.create_markdown("isolated.md", "# Isolated Page\n\nNo links here.");

    let server = TestServer::start(&repo).await;
    let body: serde_json::Value = server
        .get("/isolated/links.json")
        .await
        .json()
        .await
        .unwrap();

    let outbound = body["outbound"].as_array().unwrap();
    let inbound = body["inbound"].as_array().unwrap();

    assert!(
        outbound.is_empty(),
        "Isolated page should have no outbound links"
    );
    assert!(
        inbound.is_empty(),
        "Isolated page should have no inbound links"
    );
}

/// Rendering a page refreshes its cached outbound links, including when the
/// edit removed the last link. `LinkCache` has no TTL and nothing else
/// overwrites an entry, so skipping the insert for an empty list left
/// links.json serving the pre-edit links for the life of the process.
#[tokio::test]
async fn test_links_json_reflects_removed_links_after_edit() {
    let repo = TestRepo::new();
    let source = repo.create_markdown("source.md", "# Source\n\n[Link to Target](target/)");
    repo.create_markdown("target.md", "# Target Page");

    let server = TestServer::start(&repo).await;

    let before: serde_json::Value = server.get("/source/links.json").await.json().await.unwrap();
    assert!(
        before["outbound"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["to"].as_str().unwrap().contains("target")),
        "precondition: the link must be cached first: {:?}",
        before["outbound"]
    );

    // Drop the only link, then re-render the page (as a browser reload would).
    std::fs::write(&source, "# Source\n\nThe link is gone.").expect("rewrite source");
    assert_eq!(server.get("/source/").await.status(), 200);

    let after: serde_json::Value = server.get("/source/links.json").await.json().await.unwrap();
    assert!(
        after["outbound"].as_array().unwrap().is_empty(),
        "links.json must not keep serving the removed link: {:?}",
        after["outbound"]
    );
}

/// An externally edited file must eventually be reflected in *other* pages'
/// links.json. The mini-graph BFS fetches links.json for neighbour pages, and
/// neither link cache re-checks mtimes, so a watcher batch that does not drop
/// them serves pre-edit links until the 300 s inbound TTL (never, for the
/// outbound cache, which has no TTL at all).
#[tokio::test]
async fn test_links_json_refreshes_after_watcher_sees_external_edit() {
    let repo = TestRepo::new();
    let source = repo.create_markdown("source.md", "# Source\n\n[See the Target](target/)");
    repo.create_markdown("target.md", "# Target Page");

    let server = TestServer::start(&repo).await;
    // The watcher is initialized on a background thread; an edit that lands
    // before it is listening is simply never seen.
    tokio::time::sleep(Duration::from_millis(750)).await;

    let before: serde_json::Value = server.get("/target/links.json").await.json().await.unwrap();
    assert!(
        before["inbound"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["from"].as_str().unwrap().contains("source")),
        "precondition: the backlink must be cached first: {:?}",
        before["inbound"]
    );

    // Edit outside the server (as an editor or `git pull` would).
    let edited = "# Source\n\nThe link is gone.";
    std::fs::write(&source, edited).expect("rewrite source");

    // Watcher event + 2 s debounce + rescan; poll rather than sleeping blind,
    // re-touching the file so a dropped first event cannot hang the test.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let body: serde_json::Value = server.get("/target/links.json").await.json().await.unwrap();
        if body["inbound"].as_array().unwrap().is_empty() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "links.json still served the stale backlink: {:?}",
            body["inbound"]
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
        std::fs::write(&source, edited).expect("re-touch source");
    }
}

/// Helper to start a server with link tracking disabled
struct TestServerNoLinkTracking {
    port: u16,
    client: reqwest::Client,
    _handle: tokio::task::JoinHandle<()>,
}

impl TestServerNoLinkTracking {
    async fn start(repo: &TestRepo) -> Self {
        let port = find_available_port();
        let root_dir = repo.path().to_path_buf();

        let handle = tokio::spawn(async move {
            let mut config = test_server_config(port, root_dir);
            config.link_tracking = false; // DISABLED
            let server = mbr::server::Server::init(config).expect("Failed to initialize server");

            server.start().await.expect("test server failed to start");
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let client = mbr::http_client(Duration::from_secs(5));

        Self {
            port,
            client,
            _handle: handle,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }

    async fn get(&self, path: &str) -> reqwest::Response {
        self.client
            .get(self.url(path))
            .send()
            .await
            .expect("Request failed")
    }
}

#[tokio::test]
async fn test_links_json_404_when_link_tracking_disabled() {
    let repo = TestRepo::new();
    repo.create_markdown("page.md", "# Page\n\n[Link](other/)");
    repo.create_markdown("other.md", "# Other");

    let server = TestServerNoLinkTracking::start(&repo).await;
    let response = server.get("/page/links.json").await;

    assert_eq!(
        response.status(),
        404,
        "links.json should return 404 when link tracking is disabled"
    );
}

// ============================================================================
// Error Scenario Tests
// ============================================================================

#[tokio::test]
async fn test_path_traversal_returns_404() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello");

    let server = TestServer::start(&repo).await;

    // Basic path traversal attempt
    let response = server.get("/../../etc/passwd").await;
    assert_eq!(
        response.status(),
        404,
        "Path traversal should return 404, not expose system files"
    );
}

#[tokio::test]
async fn test_path_traversal_url_encoded_returns_404() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello");

    let server = TestServer::start(&repo).await;

    // URL-encoded path traversal attempt
    let response = server.get("/%2e%2e%2f%2e%2e%2fetc/passwd").await;
    assert_eq!(
        response.status(),
        404,
        "URL-encoded path traversal should return 404"
    );
}

#[tokio::test]
async fn test_double_encoded_path_traversal_returns_404() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello");

    let server = TestServer::start(&repo).await;

    // Double URL-encoded path traversal
    let response = server
        .get("/%252e%252e%252f%252e%252e%252fetc/passwd")
        .await;
    assert_eq!(
        response.status(),
        404,
        "Double-encoded path traversal should return 404"
    );
}

#[tokio::test]
async fn test_malformed_frontmatter_still_renders() {
    let repo = TestRepo::new();
    // Invalid YAML: unclosed string
    repo.create_markdown(
        "malformed.md",
        "---\ntitle: \"Unclosed string\n---\n\n# Content Still Works",
    );

    let server = TestServer::start(&repo).await;
    let response = server.get("/malformed/").await;

    // Should still render (gracefully handle malformed frontmatter)
    assert_eq!(
        response.status(),
        200,
        "Malformed frontmatter should not prevent page from rendering"
    );
    let html = response.text().await.unwrap();
    assert_html_contains(&html, "Content Still Works");
}

#[tokio::test]
async fn test_invalid_yaml_frontmatter_renders() {
    let repo = TestRepo::new();
    // Invalid YAML: bad indentation
    repo.create_markdown(
        "bad-yaml.md",
        "---\ntitle: Test\n   invalid: indentation\n---\n\n# Works Anyway",
    );

    let server = TestServer::start(&repo).await;
    let response = server.get("/bad-yaml/").await;

    assert_eq!(response.status(), 200);
    let html = response.text().await.unwrap();
    assert_html_contains(&html, "Works Anyway");
}

#[tokio::test]
async fn test_file_with_spaces_in_path() {
    let repo = TestRepo::new();
    repo.create_dir("my folder");
    repo.create_markdown("my folder/my file.md", "# Spaces Work");

    let server = TestServer::start(&repo).await;

    // URL-encoded spaces
    let response = server.get("/my%20folder/my%20file/").await;
    assert_eq!(response.status(), 200);
    let html = response.text().await.unwrap();
    assert_html_contains(&html, "Spaces Work");
}

#[tokio::test]
async fn test_file_with_unicode_in_path() {
    let repo = TestRepo::new();
    repo.create_dir("文档"); // Chinese for "documents"
    repo.create_markdown("文档/测试.md", "# Unicode Works");

    let server = TestServer::start(&repo).await;

    // URL-encoded unicode path
    let encoded_path = "/%E6%96%87%E6%A1%A3/%E6%B5%8B%E8%AF%95/";
    let response = server.get(encoded_path).await;
    assert_eq!(response.status(), 200);
    let html = response.text().await.unwrap();
    assert_html_contains(&html, "Unicode Works");
}

#[tokio::test]
async fn test_nonexistent_path_returns_404() {
    let repo = TestRepo::new();
    repo.create_markdown("exists.md", "# Exists");

    let server = TestServer::start(&repo).await;

    let response = server.get("/does-not-exist/").await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn test_deep_nonexistent_path_returns_404() {
    let repo = TestRepo::new();
    repo.create_dir("real");
    repo.create_markdown("real/exists.md", "# Exists");

    let server = TestServer::start(&repo).await;

    let response = server.get("/real/fake/deep/path/").await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn test_null_byte_in_path_returns_404() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello");

    let server = TestServer::start(&repo).await;

    // Null byte in URL
    let response = server.get("/readme%00.md").await;
    assert_eq!(
        response.status(),
        404,
        "Null byte in path should return 404"
    );
}

#[tokio::test]
async fn test_very_long_path_returns_404() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello");

    let server = TestServer::start(&repo).await;

    // Very long path (1000 characters)
    let long_segment = "a".repeat(200);
    let long_path = format!(
        "/{}/{}/{}/{}/{}/",
        long_segment, long_segment, long_segment, long_segment, long_segment
    );
    let response = server.get(&long_path).await;

    // Should return 404 (not crash or hang)
    assert_eq!(response.status(), 404);
}

// Note: Dot file filtering (e.g., .env, .git) is not currently implemented.
// The server serves all static files. This could be a future security enhancement.

#[tokio::test]
async fn test_empty_markdown_file() {
    let repo = TestRepo::new();
    repo.create_markdown("empty.md", "");

    let server = TestServer::start(&repo).await;
    let response = server.get("/empty/").await;

    assert_eq!(response.status(), 200, "Empty markdown should still render");
}

#[tokio::test]
async fn test_markdown_with_only_frontmatter() {
    let repo = TestRepo::new();
    repo.create_markdown("only-frontmatter.md", "---\ntitle: Just Frontmatter\n---\n");

    let server = TestServer::start(&repo).await;
    let response = server.get("/only-frontmatter/").await;

    assert_eq!(
        response.status(),
        200,
        "Markdown with only frontmatter should render"
    );
}

// ============================================================================
// PDF Cover Sidecar Tests (media-metadata feature)
// ============================================================================

/// Test that a pre-generated sidecar file is served instead of dynamically generating.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_pdf_cover_serves_from_sidecar() {
    let repo = TestRepo::new();

    // Copy a real PDF to the test repo
    let test_pdf_src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/pdfs/DGA.pdf");
    let pdf_path = repo.path().join("docs/test.pdf");
    std::fs::create_dir_all(repo.path().join("docs")).unwrap();
    std::fs::copy(&test_pdf_src, &pdf_path).unwrap();

    // Create a fake sidecar file (JPEG with magic bytes)
    let sidecar_path = repo.path().join("docs/test.pdf.cover.jpg");
    // Create a minimal valid JPEG (1x1 pixel, red)
    // This is a valid JPEG that decodes to a 1x1 red pixel
    let jpg_data: Vec<u8> = vec![
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, // JPEG SOI + APP0
        0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01, // JFIF header
        0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, // DQT marker
        0x00, 0x08, 0x06, 0x06, 0x07, 0x06, 0x05, 0x08, // Quantization table
        0x07, 0x07, 0x07, 0x09, 0x09, 0x08, 0x0A, 0x0C, //
        0x14, 0x0D, 0x0C, 0x0B, 0x0B, 0x0C, 0x19, 0x12, //
        0x13, 0x0F, 0x14, 0x1D, 0x1A, 0x1F, 0x1E, 0x1D, //
        0x1A, 0x1C, 0x1C, 0x20, 0x24, 0x2E, 0x27, 0x20, //
        0x22, 0x2C, 0x23, 0x1C, 0x1C, 0x28, 0x37, 0x29, //
        0x2C, 0x30, 0x31, 0x34, 0x34, 0x34, 0x1F, 0x27, //
        0x39, 0x3D, 0x38, 0x32, 0x3C, 0x2E, 0x33, 0x34, //
        0x32, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, // SOF0 marker (1x1)
        0x00, 0x01, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xC4, // DHT marker
        0x00, 0x1F, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01, // Huffman table
        0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, //
        0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, //
        0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0xFF, //
        0xC4, 0x00, 0xB5, 0x10, 0x00, 0x02, 0x01, 0x03, // AC Huffman table
        0x03, 0x02, 0x04, 0x03, 0x05, 0x05, 0x04, 0x04, //
        0x00, 0x00, 0x01, 0x7D, 0x01, 0x02, 0x03, 0x00, //
        0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, //
        0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, //
        0x81, 0x91, 0xA1, 0x08, 0x23, 0x42, 0xB1, 0xC1, //
        0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72, //
        0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, //
        0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35, //
        0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, //
        0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, //
        0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, //
        0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75, //
        0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, //
        0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, //
        0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, //
        0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, //
        0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, //
        0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, //
        0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, //
        0xD9, 0xDA, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, //
        0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4, //
        0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFF, 0xDA, // SOS marker
        0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, //
        0x7F, 0xFF, 0xD9, // Image data + EOI
    ];
    std::fs::write(&sidecar_path, &jpg_data).unwrap();

    // Wait a bit to ensure mtime difference
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Touch the sidecar to ensure it's newer than the PDF
    let now = std::time::SystemTime::now();
    filetime::set_file_mtime(&sidecar_path, filetime::FileTime::from_system_time(now)).unwrap();

    let server = TestServer::start(&repo).await;
    let response = server.get("/docs/test.pdf.cover.jpg").await;

    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "image/jpeg"
    );

    let body = response.bytes().await.unwrap();
    // Should serve our fake sidecar, not the dynamically generated one
    // (our sidecar is tiny, a real generated one would be much larger)
    assert_eq!(
        body.len(),
        jpg_data.len(),
        "Should serve the pre-generated sidecar file"
    );
}

/// Test that a stale sidecar falls back to serving the stale sidecar when regeneration fails.
///
/// This test verifies graceful degradation: when the PDF is newer than the sidecar,
/// we attempt to regenerate, but if that fails (e.g., pdfium not available), we
/// serve the stale sidecar rather than returning an error.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_pdf_cover_stale_sidecar_serves_gracefully() {
    let repo = TestRepo::new();

    // Copy a real PDF to the test repo
    let test_pdf_src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/pdfs/DGA.pdf");
    let pdf_path = repo.path().join("docs/test.pdf");
    std::fs::create_dir_all(repo.path().join("docs")).unwrap();
    std::fs::copy(&test_pdf_src, &pdf_path).unwrap();

    // Create a fake sidecar file (valid but small JPEG)
    let sidecar_path = repo.path().join("docs/test.pdf.cover.jpg");
    // Create a minimal valid JPEG (just magic bytes + minimal structure)
    let stale_jpg: Vec<u8> = vec![
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, // JPEG SOI + APP0
        0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01, // JFIF header
        0x00, 0x01, 0x00, 0x00, 0xFF, 0xD9, // EOI
    ];
    std::fs::write(&sidecar_path, &stale_jpg).unwrap();

    // Make the sidecar older than the PDF (stale)
    let old_time = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000000000);
    filetime::set_file_mtime(
        &sidecar_path,
        filetime::FileTime::from_system_time(old_time),
    )
    .unwrap();

    // Now touch the PDF to make it newer than the sidecar
    let now = std::time::SystemTime::now();
    filetime::set_file_mtime(&pdf_path, filetime::FileTime::from_system_time(now)).unwrap();

    let server = TestServer::start(&repo).await;
    let response = server.get("/docs/test.pdf.cover.jpg").await;

    // Should serve successfully (either regenerated or stale fallback)
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "image/jpeg"
    );

    let body = response.bytes().await.unwrap();
    // If pdfium is not available, we fall back to stale sidecar
    // If pdfium is available, we regenerate (larger file)
    // Either way, we should get valid JPEG data
    assert!(
        body.len() >= stale_jpg.len(),
        "Should serve at least the stale sidecar"
    );
    assert_eq!(
        &body[0..2],
        &[0xFF, 0xD8],
        "Should be valid JPEG (SOI marker)"
    );
}

/// Test that PDF cover requests without sidecar return 404 when pdfium is not available.
///
/// Note: This test verifies the graceful failure case. In production with pdfium
/// available, dynamic generation would succeed.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_pdf_cover_no_sidecar_returns_404_without_pdfium() {
    let repo = TestRepo::new();

    // Copy a real PDF to the test repo (no sidecar)
    let test_pdf_src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/pdfs/DGA.pdf");
    let pdf_path = repo.path().join("docs/report.pdf");
    std::fs::create_dir_all(repo.path().join("docs")).unwrap();
    std::fs::copy(&test_pdf_src, &pdf_path).unwrap();

    // No sidecar file exists
    let sidecar_path = repo.path().join("docs/report.pdf.cover.jpg");
    assert!(!sidecar_path.exists());

    let server = TestServer::start(&repo).await;
    let response = server.get("/docs/report.pdf.cover.jpg").await;

    // Without pdfium, this will return 404 (no sidecar, can't generate)
    // With pdfium available, this would return 200 with generated cover
    // We accept either outcome as valid for this test
    let status = response.status();
    assert!(
        status == 200 || status == 404,
        "Expected 200 (pdfium available) or 404 (pdfium unavailable), got {}",
        status
    );
}

// ============================================================================
// Media Viewer Tests
// ============================================================================

#[tokio::test]
async fn test_media_viewer_video_missing_path_returns_error() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello");

    let server = TestServer::start(&repo).await;

    // Request video viewer without path parameter should return 400 Bad Request
    let response = server.get("/.mbr/videos/").await;

    assert_eq!(
        response.status(),
        400,
        "Missing path parameter should return 400 Bad Request"
    );

    let html = response.text().await.unwrap();
    assert!(
        html.contains("Bad Request") || html.contains("Missing"),
        "Error page should indicate missing path. Got: {}",
        &html[..std::cmp::min(500, html.len())]
    );
}

#[tokio::test]
async fn test_media_viewer_video_valid_path_returns_200() {
    let repo = TestRepo::new();

    // Create a test video file
    repo.create_dir("videos");
    repo.create_static_file("videos/test.mp4", b"fake video content");

    let server = TestServer::start(&repo).await;

    // Request video viewer with valid path
    let response = server.get("/.mbr/videos/?path=/videos/test.mp4").await;

    assert_eq!(
        response.status(),
        200,
        "Valid video path should return 200 OK"
    );

    let html = response.text().await.unwrap();

    // Verify the media viewer template is rendered
    assert!(
        html.contains("mbr-media-viewer"),
        "Response should contain mbr-media-viewer component. Got: {}",
        &html[..std::cmp::min(1000, html.len())]
    );

    // Verify media type is set correctly
    assert!(
        html.contains("video") || html.contains("Video"),
        "Response should indicate video media type"
    );
}

#[tokio::test]
async fn test_media_viewer_video_directory_traversal_blocked() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello");

    let server = TestServer::start(&repo).await;

    // Attempt directory traversal via path parameter
    let response = server.get("/.mbr/videos/?path=/../etc/passwd").await;

    assert_eq!(
        response.status(),
        403,
        "Directory traversal should return 403 Forbidden"
    );

    let html = response.text().await.unwrap();
    assert!(
        html.contains("Forbidden") || html.contains("Access denied"),
        "Error page should indicate access denied. Got: {}",
        &html[..std::cmp::min(500, html.len())]
    );
}

#[tokio::test]
async fn test_media_viewer_video_url_encoded_traversal_blocked() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello");

    let server = TestServer::start(&repo).await;

    // URL-encoded ".." = "%2e%2e"
    let response = server
        .get("/.mbr/videos/?path=%2f%2e%2e%2fetc%2fpasswd")
        .await;

    assert_eq!(
        response.status(),
        403,
        "URL-encoded directory traversal should return 403 Forbidden"
    );
}

#[tokio::test]
async fn test_media_viewer_video_nonexistent_file_returns_404() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello");

    let server = TestServer::start(&repo).await;

    // Request a video file that doesn't exist
    let response = server
        .get("/.mbr/videos/?path=/videos/nonexistent.mp4")
        .await;

    assert_eq!(
        response.status(),
        404,
        "Nonexistent video file should return 404 Not Found"
    );
}

#[tokio::test]
async fn test_media_viewer_video_nested_path() {
    let repo = TestRepo::new();

    // Create a nested video structure
    repo.create_dir("videos/2024/january");
    repo.create_static_file("videos/2024/january/event.mp4", b"fake video");

    let server = TestServer::start(&repo).await;

    let response = server
        .get("/.mbr/videos/?path=/videos/2024/january/event.mp4")
        .await;

    assert_eq!(
        response.status(),
        200,
        "Nested video path should return 200 OK"
    );

    let html = response.text().await.unwrap();
    assert!(
        html.contains("mbr-media-viewer"),
        "Response should contain media viewer component"
    );
}

#[tokio::test]
async fn test_media_viewer_video_with_spaces_in_path() {
    let repo = TestRepo::new();

    // Create a video file with spaces in the name
    repo.create_dir("videos");
    repo.create_static_file("videos/my video file.mp4", b"fake video");

    let server = TestServer::start(&repo).await;

    // URL-encoded path with spaces
    let response = server
        .get("/.mbr/videos/?path=/videos/my%20video%20file.mp4")
        .await;

    assert_eq!(
        response.status(),
        200,
        "Video path with spaces should return 200 OK"
    );
}

#[tokio::test]
async fn test_media_viewer_pdf_missing_path_returns_error() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello");

    let server = TestServer::start(&repo).await;

    // Request PDF viewer without path parameter
    let response = server.get("/.mbr/pdfs/").await;

    assert_eq!(
        response.status(),
        400,
        "Missing path parameter should return 400 Bad Request"
    );
}

#[tokio::test]
async fn test_media_viewer_pdf_valid_path_returns_200() {
    let repo = TestRepo::new();

    // Create a test PDF file
    repo.create_dir("documents");
    repo.create_static_file("documents/report.pdf", b"%PDF-1.4 fake pdf");

    let server = TestServer::start(&repo).await;

    let response = server.get("/.mbr/pdfs/?path=/documents/report.pdf").await;

    assert_eq!(
        response.status(),
        200,
        "Valid PDF path should return 200 OK"
    );

    let html = response.text().await.unwrap();
    assert!(
        html.contains("mbr-media-viewer"),
        "Response should contain mbr-media-viewer component"
    );
}

#[tokio::test]
async fn test_media_viewer_audio_missing_path_returns_error() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello");

    let server = TestServer::start(&repo).await;

    // Request audio viewer without path parameter
    let response = server.get("/.mbr/audio/").await;

    assert_eq!(
        response.status(),
        400,
        "Missing path parameter should return 400 Bad Request"
    );
}

#[tokio::test]
async fn test_media_viewer_audio_valid_path_returns_200() {
    let repo = TestRepo::new();

    // Create a test audio file
    repo.create_dir("audio");
    repo.create_static_file("audio/song.mp3", b"fake mp3 content");

    let server = TestServer::start(&repo).await;

    let response = server.get("/.mbr/audio/?path=/audio/song.mp3").await;

    assert_eq!(
        response.status(),
        200,
        "Valid audio path should return 200 OK"
    );

    let html = response.text().await.unwrap();
    assert!(
        html.contains("mbr-media-viewer"),
        "Response should contain mbr-media-viewer component"
    );
}

#[tokio::test]
async fn test_media_viewer_has_breadcrumbs() {
    let repo = TestRepo::new();

    repo.create_dir("videos/tutorials");
    repo.create_static_file("videos/tutorials/lesson.mp4", b"fake video");

    let server = TestServer::start(&repo).await;

    let response = server
        .get("/.mbr/videos/?path=/videos/tutorials/lesson.mp4")
        .await;

    assert_eq!(response.status(), 200);

    let html = response.text().await.unwrap();

    // Check for breadcrumb navigation
    assert!(
        html.contains("tutorials") || html.contains("videos"),
        "Response should contain breadcrumb navigation"
    );
}

#[tokio::test]
async fn test_media_viewer_has_back_navigation() {
    let repo = TestRepo::new();

    repo.create_dir("videos");
    repo.create_static_file("videos/demo.mp4", b"fake video");

    let server = TestServer::start(&repo).await;

    let response = server.get("/.mbr/videos/?path=/videos/demo.mp4").await;

    assert_eq!(response.status(), 200);

    let html = response.text().await.unwrap();

    // Check for back navigation
    assert!(
        html.contains("Back") || html.contains("parent_path") || html.contains("/videos/"),
        "Response should contain back navigation"
    );
}

// ==================== MBR Assets Path Traversal Tests ====================
//
// Note: Axum normalizes URL paths BEFORE route matching, providing first-layer defense.
// For example, `/.mbr/../readme.md` becomes `/readme.md` at the framework level.
// Our `safe_join_asset` function provides defense-in-depth for edge cases.
// These tests verify the combined security behavior.

#[tokio::test]
async fn test_mbr_assets_traversal_normalized_by_framework() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello");

    let server = TestServer::start(&repo).await;

    // Axum normalizes `/.mbr/../readme.md` to `/readme.md` before routing
    // This means the request never reaches serve_mbr_assets at all
    // Instead it routes to the markdown handler and returns 200 for readme.md
    // This is framework-level security - the traversal is neutralized
    let response = server.get("/.mbr/../readme.md").await;
    // The normalized path /readme.md routes to markdown handler
    assert!(
        response.status() == 200 || response.status() == 301,
        "Framework normalizes path - should route to markdown handler"
    );
}

#[tokio::test]
async fn test_mbr_assets_nonexistent_file_returns_404() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello");

    let server = TestServer::start(&repo).await;

    // Request for nonexistent .mbr file should return 404
    let response = server.get("/.mbr/nonexistent.css").await;
    assert_eq!(
        response.status(),
        404,
        "Nonexistent .mbr file should return 404"
    );
}

#[tokio::test]
async fn test_mbr_assets_cannot_access_files_outside_mbr_dir() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello");
    repo.create_markdown("secret.md", "# Secret content");
    repo.create_dir(".mbr");
    repo.create_static_file(".mbr/user.css", b"body {}");

    let server = TestServer::start(&repo).await;

    // Even if somehow a path like "/../secret.md" reached serve_mbr_assets,
    // safe_join_asset would reject it. We can't easily test this at integration
    // level due to Axum's normalization, but the unit tests in server.rs verify it.
    // Here we verify that .mbr serves only .mbr content.
    let response = server.get("/.mbr/user.css").await;
    assert_eq!(response.status(), 200, ".mbr file should be accessible");

    // A file that exists at root but not in .mbr should not be found via /.mbr/
    let response = server.get("/.mbr/secret.md").await;
    assert_eq!(
        response.status(),
        404,
        "Files outside .mbr dir should not be accessible via /.mbr/ route"
    );
}

#[tokio::test]
async fn test_mbr_assets_valid_file_still_works() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello");

    // Create a custom CSS file in .mbr directory
    repo.create_dir(".mbr");
    repo.create_static_file(".mbr/user.css", b"body { color: red; }");

    let server = TestServer::start(&repo).await;

    // Valid .mbr asset request should still work
    let response = server.get("/.mbr/user.css").await;
    assert_eq!(
        response.status(),
        200,
        "Valid .mbr asset should return 200 OK"
    );

    let content = response.text().await.unwrap();
    assert!(
        content.contains("body { color: red; }"),
        "Should return the correct CSS content"
    );
}

#[tokio::test]
async fn test_mbr_assets_nested_path_works() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello");

    // Create nested directory structure in .mbr
    repo.create_dir(".mbr/custom/styles");
    repo.create_static_file(
        ".mbr/custom/styles/theme.css",
        b".theme { display: block; }",
    );

    let server = TestServer::start(&repo).await;

    // Nested path should work
    let response = server.get("/.mbr/custom/styles/theme.css").await;
    assert_eq!(
        response.status(),
        200,
        "Nested .mbr asset path should return 200 OK"
    );
}

#[tokio::test]
async fn test_code_blocks_with_unsupported_language_render_correctly() {
    let repo = TestRepo::new();
    let markdown = concat!(
        "# Code Examples\n\n",
        "```rust\nfn hello() { println!(\"hi\"); }\n```\n\n",
        "```totally_bogus_lang\nsome code here\n```\n\n",
        "```python\ndef hello(): pass\n```",
    );
    repo.create_markdown("code.md", markdown);

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/code/").await;

    // Page renders successfully with all code blocks
    assert_html_contains(&html, "Code Examples");
    assert_html_contains(&html, "language-rust");
    assert_html_contains(&html, "language-totally_bogus_lang");
    assert_html_contains(&html, "language-python");

    // Code content is preserved
    assert_html_contains(&html, "fn hello");
    assert_html_contains(&html, "some code here");
    assert_html_contains(&html, "def hello");

    // hljs component is present so client-side highlighting can proceed
    assert_html_contains(&html, "mbr-hljs");
}

// ==================== Title Prefix/Suffix Tests ====================

#[tokio::test]
async fn test_title_prefix_in_markdown_page() {
    let repo = TestRepo::new();
    repo.create_markdown(
        "readme.md",
        "---\ntitle: My Page\n---\n\n# My Page\n\nContent.",
    );

    let server = TestServer::start_with_config_fn(&repo, |config| {
        config.title_prefix = "My Site: ".to_string();
    })
    .await;

    let html = server.get_text("/readme/").await;
    assert_html_contains(&html, "<title>My Site: My Page</title>");
}

#[tokio::test]
async fn test_title_suffix_in_markdown_page() {
    let repo = TestRepo::new();
    repo.create_markdown(
        "readme.md",
        "---\ntitle: My Page\n---\n\n# My Page\n\nContent.",
    );

    let server = TestServer::start_with_config_fn(&repo, |config| {
        config.title_suffix = " | My Site".to_string();
    })
    .await;

    let html = server.get_text("/readme/").await;
    assert_html_contains(&html, "<title>My Page | My Site</title>");
}

#[tokio::test]
async fn test_title_prefix_and_suffix_combined() {
    let repo = TestRepo::new();
    repo.create_markdown(
        "readme.md",
        "---\ntitle: My Page\n---\n\n# My Page\n\nContent.",
    );

    let server = TestServer::start_with_config_fn(&repo, |config| {
        config.title_prefix = "PREFIX ".to_string();
        config.title_suffix = " SUFFIX".to_string();
    })
    .await;

    let html = server.get_text("/readme/").await;
    assert_html_contains(&html, "<title>PREFIX My Page SUFFIX</title>");
}

#[tokio::test]
async fn test_title_prefix_in_directory_listing() {
    let repo = TestRepo::new();
    repo.create_dir("docs");
    repo.create_markdown("docs/guide.md", "# Guide");

    let server = TestServer::start_with_config_fn(&repo, |config| {
        config.title_prefix = "MySite: ".to_string();
    })
    .await;

    // Wait for scan so directory listing has files
    server.wait_for_scan().await;

    let html = server.get_text("/docs/").await;
    assert_html_contains(&html, "<title>MySite: ");
}

// ==================== Tag Page Search Tests ====================

#[tokio::test]
async fn test_search_finds_tag_pages() {
    let repo = TestRepo::new();
    // Create files with tags
    repo.create_markdown(
        "guide.md",
        "---\ntitle: Rust Guide\ntags:\n  - rust\n  - programming\n---\n\nContent.",
    );
    repo.create_markdown(
        "tutorial.md",
        "---\ntitle: Rust Tutorial\ntags:\n  - rust\n---\n\nMore content.",
    );

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    // Search for "rust" in metadata scope — should find the tag page too
    let response = server
        .post_json("/.mbr/search", r#"{"q": "rust", "scope": "metadata"}"#)
        .await;
    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.unwrap();
    let results = body["results"].as_array().unwrap();

    // Should find the tag page /tags/rust/
    let tag_result = results
        .iter()
        .find(|r| r["filetype"].as_str().unwrap() == "tag");
    assert!(
        tag_result.is_some(),
        "Expected to find a tag page result: {:?}",
        results
    );

    let tag_result = tag_result.unwrap();
    assert_eq!(tag_result["url_path"].as_str().unwrap(), "/tags/rust/");
    assert!(
        tag_result["title"].as_str().unwrap().contains("rust"),
        "Tag page title should contain the tag name"
    );
    assert!(
        tag_result["description"]
            .as_str()
            .unwrap()
            .contains("2 pages"),
        "Tag page should show page count"
    );
}

#[tokio::test]
async fn test_search_tag_pages_with_custom_source() {
    let repo = TestRepo::new();
    repo.create_markdown(
        "show1.md",
        "---\ntitle: Magic Show\nperformer: Joshua Jay\n---\n\nContent.",
    );
    repo.create_markdown(
        "show2.md",
        "---\ntitle: Card Tricks\nperformer: Joshua Jay\n---\n\nContent.",
    );

    let server = TestServer::start_with_config_fn(&repo, |config| {
        config.tag_sources = vec![mbr::config::TagSource {
            field: "performer".to_string(),
            label: Some("Performer".to_string()),
            label_plural: Some("Performers".to_string()),
        }];
    })
    .await;
    server.wait_for_scan().await;

    let response = server
        .post_json("/.mbr/search", r#"{"q": "Joshua", "scope": "metadata"}"#)
        .await;
    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.unwrap();
    let results = body["results"].as_array().unwrap();

    let tag_result = results
        .iter()
        .find(|r| r["filetype"].as_str().unwrap() == "tag");
    assert!(
        tag_result.is_some(),
        "Expected to find a performer tag page result: {:?}",
        results
    );

    let tag_result = tag_result.unwrap();
    assert!(
        tag_result["url_path"]
            .as_str()
            .unwrap()
            .contains("/performer/joshua_jay/"),
        "Tag page URL should use normalized value"
    );
    assert!(
        tag_result["title"].as_str().unwrap().contains("Performer"),
        "Tag page title should contain the source label"
    );
}

#[tokio::test]
async fn test_search_tag_pages_skipped_for_folder_scope() {
    let repo = TestRepo::new();
    repo.create_markdown(
        "docs/guide.md",
        "---\ntitle: Guide\ntags:\n  - rust\n---\n\nContent.",
    );

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    // Search with folder scope — tag pages should not appear
    let response = server
        .post_json(
            "/.mbr/search",
            r#"{"q": "rust", "scope": "metadata", "folder": "/docs/", "folder_scope": "current"}"#,
        )
        .await;
    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.unwrap();
    let results = body["results"].as_array().unwrap();

    let tag_result = results
        .iter()
        .find(|r| r["filetype"].as_str().unwrap() == "tag");
    assert!(
        tag_result.is_none(),
        "Tag pages should not appear when folder scope is current: {:?}",
        results
    );
}

// ============================================================================
// Readability Scores (window.extendedMeta)
// ============================================================================

#[tokio::test]
async fn test_readability_scores_injected_into_extended_meta() {
    let repo = TestRepo::new();
    // A document with enough sentences/words to produce plausible scores.
    repo.create_markdown(
        "article.md",
        "# Article\n\nThis is a simple test. It has several short sentences. \
         The quick brown fox jumps over the lazy dog. Another sentence follows.\n",
    );

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/article/").await;

    // The extendedMeta literal should include both new fields as JS numbers
    // (not the string "null") because the document has words and sentences.
    assert!(
        html.contains("fleschReadingEase:"),
        "expected fleschReadingEase field in extendedMeta; html snippet: {}",
        &html[..html.len().min(2000)]
    );
    assert!(
        html.contains("fleschKincaidGrade:"),
        "expected fleschKincaidGrade field in extendedMeta"
    );
    // Sanity: we should NOT be rendering these as null for a non-trivial doc.
    assert!(
        !html.contains("fleschReadingEase: null"),
        "FRE should be a number for a document with prose"
    );
    assert!(
        !html.contains("fleschKincaidGrade: null"),
        "FKGL should be a number for a document with prose"
    );
}

#[tokio::test]
async fn test_readability_scores_null_for_code_only_document() {
    let repo = TestRepo::new();
    // A document with only a code block has zero words counted, so scores
    // should serialize as `null` and the template must render them literally.
    repo.create_markdown(
        "code-only.md",
        "```rust\nfn main() { println!(\"hello\"); }\n```\n",
    );

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/code-only/").await;

    assert!(
        html.contains("fleschReadingEase: null"),
        "FRE should be null for a code-only document"
    );
    assert!(
        html.contains("fleschKincaidGrade: null"),
        "FKGL should be null for a code-only document"
    );
}

// ============================================================================
// Per-page errors.json endpoint
// ============================================================================

#[tokio::test]
async fn test_errors_json_clean_page_returns_empty() {
    let repo = TestRepo::new();
    repo.create_markdown("clean.md", "# Clean\n\nNo problems here.");

    let server = TestServer::start(&repo).await;
    let response = server.get("/clean/errors.json").await;

    assert_eq!(response.status(), 200);
    let json: serde_json::Value = response.json().await.unwrap();
    assert_eq!(json["page_url"], "/clean/");
    assert!(json["errors"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_errors_json_reports_broken_internal_link() {
    let repo = TestRepo::new();
    repo.create_markdown("page.md", "# Page\n\n[bad](/nonexistent/) link here.");

    let server = TestServer::start(&repo).await;
    let response = server.get("/page/errors.json").await;

    assert_eq!(response.status(), 200);
    let json: serde_json::Value = response.json().await.unwrap();
    let errors = json["errors"].as_array().unwrap();
    assert!(
        errors.iter().any(|e| e["type"] == "broken_internal_link"
            && e["target"].as_str().unwrap().contains("/nonexistent")),
        "expected broken_internal_link in {:?}",
        errors
    );
}

#[tokio::test]
async fn test_errors_json_reports_broken_media_reference() {
    let repo = TestRepo::new();
    repo.create_markdown("media.md", "# Media\n\n![alt](./missing.png)");

    let server = TestServer::start(&repo).await;
    let response = server.get("/media/errors.json").await;

    assert_eq!(response.status(), 200);
    let json: serde_json::Value = response.json().await.unwrap();
    let errors = json["errors"].as_array().unwrap();
    assert!(
        errors.iter().any(|e| e["type"] == "broken_media_reference"
            && e["kind"] == "image"
            && e["src"].as_str().unwrap().contains("missing.png")),
        "expected broken_media_reference image in {:?}",
        errors
    );
}

#[tokio::test]
async fn test_errors_json_reports_unresolved_wikilink() {
    let repo = TestRepo::new();
    // pulldown-cmark has native wikilink support when `ENABLE_WIKILINKS` is
    // on (we use `Options::all()` in `markdown.rs`), so simple `[[foo]]`
    // becomes `<a href="foo">foo</a>` and gets caught by the broken-internal-
    // link check instead. A literal `[[...]]` survives into HTML when it
    // appears inside a raw HTML block — this is the only pragmatic case
    // where the wikilink detector fires independently.
    repo.create_markdown(
        "wiki.md",
        "# Wiki\n\n<div class=\"raw\">See [[never-a-real-page]] here.</div>",
    );

    let server = TestServer::start(&repo).await;
    let response = server.get("/wiki/errors.json").await;

    assert_eq!(response.status(), 200);
    let json: serde_json::Value = response.json().await.unwrap();
    let errors = json["errors"].as_array().unwrap();
    assert!(
        errors.iter().any(|e| e["type"] == "unresolved_wikilink"),
        "expected unresolved_wikilink in {:?}",
        errors
    );
}

#[tokio::test]
async fn test_errors_json_percent_encoded_link_to_existing_file_is_clean() {
    // Regression: axum percent-decodes live request paths before resolution,
    // so an authored href like /Target%20With%20Spaces/ must not be reported
    // as a broken internal link when "Target With Spaces.md" exists.
    let repo = TestRepo::new();
    repo.create_markdown("Target With Spaces.md", "# Target");
    repo.create_markdown(
        "page.md",
        "# Page\n\n[spaced](/Target%20With%20Spaces/) link here.",
    );

    let server = TestServer::start(&repo).await;
    let response = server.get("/page/errors.json").await;

    assert_eq!(response.status(), 200);
    let json: serde_json::Value = response.json().await.unwrap();
    let errors = json["errors"].as_array().unwrap();
    assert!(
        errors.is_empty(),
        "percent-encoded link to existing file should produce no errors, got: {:?}",
        errors
    );
}

#[tokio::test]
async fn test_errors_json_returns_404_when_link_tracking_disabled() {
    let repo = TestRepo::new();
    repo.create_markdown("page.md", "# Page");

    let server = TestServer::start_with_config_fn(&repo, |cfg| {
        cfg.link_tracking = false;
    })
    .await;

    let response = server.get("/page/errors.json").await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn test_errors_json_multiple_problem_types_on_one_page() {
    let repo = TestRepo::new();
    repo.create_markdown(
        "mixed.md",
        concat!(
            "# Mixed\n\n",
            "[bad](./nonexistent/)\n\n",
            "![m](./missing.png)\n\n",
            // Inside a raw-HTML block, pulldown-cmark leaves `[[...]]`
            // untouched, exercising the unresolved-wikilink detector.
            "<div class=\"raw\">[[never-a-real-page]]</div>\n",
        ),
    );

    let server = TestServer::start(&repo).await;
    let response = server.get("/mixed/errors.json").await;

    assert_eq!(response.status(), 200);
    let json: serde_json::Value = response.json().await.unwrap();
    let errors = json["errors"].as_array().unwrap();

    assert!(errors.iter().any(|e| e["type"] == "broken_internal_link"));
    assert!(errors.iter().any(|e| e["type"] == "broken_media_reference"));
    assert!(errors.iter().any(|e| e["type"] == "unresolved_wikilink"));
}

#[tokio::test]
async fn test_errors_json_reports_frontmatter_parse_error() {
    let repo = TestRepo::new();
    // Invalid YAML frontmatter: `*` list markers with TAB indentation. This is
    // the real-world case (an Obsidian-style note) where the whole frontmatter
    // — including a valid `style: slides` — is silently discarded.
    repo.create_markdown(
        "broken.md",
        "---\ntitle: Broken\nstyle: slides\ntags:\n\t* presentation\n\t* ai\n---\n# Broken\n",
    );

    let server = TestServer::start(&repo).await;
    let response = server.get("/broken/errors.json").await;

    assert_eq!(response.status(), 200);
    let json: serde_json::Value = response.json().await.unwrap();
    let errors = json["errors"].as_array().unwrap();
    assert!(
        errors.iter().any(|e| e["type"] == "frontmatter_parse_error"
            && e["message"].as_str().is_some_and(|m| !m.is_empty())),
        "expected frontmatter_parse_error in {:?}",
        errors
    );
}

// ============================================================================
// errors.json: relationship data problems (cycles, ambiguous names)
// ============================================================================

/// Two notes each claiming the other as a parent. Impossible in a real family
/// tree, and the reason `family-chart`'s `d3.hierarchy()` allocated forever and
/// froze the browser on the affected person pages.
fn parent_child_cycle_repo() -> TestRepo {
    let repo = TestRepo::new();
    repo.create_markdown(
        "people/ada.md",
        "---\ntype: person\nrelationships:\n  - type: parent\n    to: \"[[Bob]]\"\n---\n# Ada\n",
    );
    repo.create_markdown(
        "people/bob.md",
        "---\ntype: person\nrelationships:\n  - type: parent\n    to: \"[[Ada]]\"\n---\n# Bob\n",
    );
    // A third note outside the loop, to prove the report is scoped to members.
    repo.create_markdown("people/cleo.md", "---\ntype: person\n---\n# Cleo\n");
    repo
}

#[tokio::test]
async fn test_errors_json_reports_relationship_cycle() {
    let repo = parent_child_cycle_repo();
    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    let json: serde_json::Value = server
        .get("/people/ada/errors.json")
        .await
        .json()
        .await
        .unwrap();
    let errors = json["errors"].as_array().unwrap();

    let cycle = errors
        .iter()
        .find(|e| e["type"] == "relationship_cycle")
        .unwrap_or_else(|| panic!("expected relationship_cycle in {errors:?}"));
    assert_eq!(cycle["rel_type"], "child");
    let members: Vec<&str> = cycle["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap())
        .collect();
    assert!(
        members.contains(&"/people/ada/") && members.contains(&"/people/bob/"),
        "both cycle members should be named: {members:?}"
    );
}

#[tokio::test]
async fn test_errors_json_cycle_is_reported_on_every_member_but_not_others() {
    let repo = parent_child_cycle_repo();
    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    for page in ["/people/ada/", "/people/bob/"] {
        let json: serde_json::Value = server
            .get(&format!("{page}errors.json"))
            .await
            .json()
            .await
            .unwrap();
        assert!(
            json["errors"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["type"] == "relationship_cycle"),
            "{page} is a cycle member and must report it: {json:?}"
        );
    }

    // Cleo declares nothing and is in no cycle.
    let json: serde_json::Value = server
        .get("/people/cleo/errors.json")
        .await
        .json()
        .await
        .unwrap();
    assert!(
        !json["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["type"] == "relationship_cycle"),
        "a note outside the cycle must not be flagged: {json:?}"
    );
}

#[tokio::test]
async fn test_errors_json_no_cycle_when_relationship_tracking_disabled() {
    let repo = parent_child_cycle_repo();
    let server = TestServer::start_with_config_fn(&repo, |cfg| {
        cfg.relationship_tracking = false;
    })
    .await;
    server.wait_for_scan().await;

    let json: serde_json::Value = server
        .get("/people/ada/errors.json")
        .await
        .json()
        .await
        .unwrap();
    assert!(
        !json["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["type"] == "relationship_cycle"),
        "--no-relationship-tracking must report no relationship problems: {json:?}"
    );
}

#[tokio::test]
async fn test_errors_json_reports_ambiguous_relationship_endpoint() {
    // A John Doe Sr and a John Doe Jr: `to: "[[John Doe]]"` silently landed on
    // whichever sorted first, which is how a cycle gets closed by accident.
    let repo = TestRepo::new();
    repo.create_markdown(
        "people/john-jr.md",
        "---\ntype: person\ntitle: John Doe\n---\n# John Doe Jr\n",
    );
    repo.create_markdown(
        "people/john-sr.md",
        "---\ntype: person\ntitle: John Doe\n---\n# John Doe Sr\n",
    );
    repo.create_markdown(
        "people/zeb.md",
        "---\ntype: person\nrelationships:\n  - type: parent\n    to: \"[[John Doe]]\"\n---\n# Zeb\n",
    );

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    let json: serde_json::Value = server
        .get("/people/zeb/errors.json")
        .await
        .json()
        .await
        .unwrap();
    let errors = json["errors"].as_array().unwrap();

    let found = errors
        .iter()
        .find(|e| e["type"] == "ambiguous_relationship_endpoint")
        .unwrap_or_else(|| panic!("expected ambiguous_relationship_endpoint in {errors:?}"));
    assert_eq!(found["raw"], "[[John Doe]]");
    assert_eq!(found["resolved_to"], "/people/john-jr/");
    assert_eq!(
        found["candidates"].as_array().unwrap(),
        &vec![serde_json::json!("/people/john-sr/")]
    );
}

#[tokio::test]
async fn test_errors_json_reports_ambiguous_body_wikilink() {
    // Same namesake problem, reached through a `[[wikilink]]` in prose rather
    // than through frontmatter.
    let repo = TestRepo::new();
    repo.create_markdown("people/a-sam.md", "---\ntitle: Sam\n---\n# Sam (a)\n");
    repo.create_markdown("people/z-sam.md", "---\ntitle: Sam\n---\n# Sam (z)\n");
    repo.create_markdown("notes/story.md", "# Story\n\nWe met [[Sam]] that day.");

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    let json: serde_json::Value = server
        .get("/notes/story/errors.json")
        .await
        .json()
        .await
        .unwrap();
    let errors = json["errors"].as_array().unwrap();

    let found = errors
        .iter()
        .find(|e| e["type"] == "ambiguous_wikilink")
        .unwrap_or_else(|| panic!("expected ambiguous_wikilink in {errors:?}"));
    assert_eq!(found["raw"], "[[Sam]]");
    assert_eq!(found["resolved_to"], "/people/a-sam/");
    assert_eq!(
        found["candidates"].as_array().unwrap(),
        &vec![serde_json::json!("/people/z-sam/")]
    );
}

#[tokio::test]
async fn test_errors_json_reports_frontmatter_parse_error_for_duplicate_key() {
    // Two `to:` keys in one `relationships:` entry. yaml-rust2 aborts the whole
    // document, so `type: person`, `born`, `aliases` and every relationship are
    // discarded — the failure mode that made a user's notes lose their identity
    // with nothing but an unattributed warning to show for it.
    let repo = TestRepo::new();
    repo.create_markdown(
        "people/john.md",
        concat!(
            "---\n",
            "type: person\n",
            "born: 1901-05-02\n",
            "aliases:\n",
            "  - Johnny Doe\n",
            "relationships:\n",
            "  - type: parent\n",
            "    to: \"[[Mary Doe]]\"\n",
            "    to: \"[[Sam Doe]]\"\n",
            "---\n",
            "# John Doe\n",
        ),
    );

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    let json: serde_json::Value = server
        .get("/people/john/errors.json")
        .await
        .json()
        .await
        .unwrap();
    let errors = json["errors"].as_array().unwrap();
    assert!(
        errors.iter().any(|e| e["type"] == "frontmatter_parse_error"
            && e["message"]
                .as_str()
                .is_some_and(|m| m.contains("duplicated key"))),
        "expected a duplicate-key frontmatter_parse_error in {errors:?}"
    );
}

#[tokio::test]
async fn test_errors_json_clean_relationship_data_reports_nothing() {
    // A correct little family must stay silent — no false cycle from the
    // reciprocal derivation, and no ambiguity from distinct names.
    let repo = TestRepo::new();
    repo.create_markdown(
        "people/parent.md",
        "---\ntype: person\nrelationships:\n  - type: child\n    to: \"[[Kid]]\"\n---\n# Parent\n",
    );
    repo.create_markdown(
        "people/kid.md",
        "---\ntype: person\ntitle: Kid\nrelationships:\n  - type: parent\n    to: \"[[Parent]]\"\n---\n# Kid\n",
    );

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    for page in ["/people/parent/", "/people/kid/"] {
        let json: serde_json::Value = server
            .get(&format!("{page}errors.json"))
            .await
            .json()
            .await
            .unwrap();
        let errors = json["errors"].as_array().unwrap();
        assert!(errors.is_empty(), "{page} should be clean, got: {errors:?}");
    }
}

// ============================================================================
// Unplayable-media detection (errors.json)
// ============================================================================
//
// Fixtures in `tests/videos/` are 2-3 KB each and form the 2x2 that bisection
// isolated in real Safari: only a `gpmd` data track *combined with* a `tx3g`
// subtitle track discriminated failing cuts from playing ones. Either track
// type alone measured as playing, so three of the four must never be flagged.
//
// - `gpmd-and-tx3g.mp4` — both tracks. The only fixture the heuristic flags.
// - `gpmd-only.mp4`     — data track without subtitles. False-positive guard.
// - `tx3g-only.mp4`     — subtitles without a risky data track. Same.
// - `neither.mp4`       — a harmless `text` data track. Same.
//
// NOTE: the heuristic is advisory, not an oracle. `gpmd-and-tx3g.mp4` itself
// *plays fine* in Safari despite matching, so the combination is necessary but
// not sufficient. These tests therefore assert only what the predicate
// computes, never that a flagged file fails or an unflagged one plays.

/// Copies a checked-in video fixture into the test repo.
#[cfg(feature = "media-metadata")]
fn copy_video_fixture(repo: &TestRepo, fixture: &str, dest: &str) {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/videos")
        .join(fixture);
    let bytes = std::fs::read(&source)
        .unwrap_or_else(|e| panic!("missing fixture {}: {e}", source.display()));
    repo.create_static_file(dest, &bytes);
}

#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_errors_json_reports_unplayable_gpmd_and_tx3g_video() {
    let repo = TestRepo::new();
    copy_video_fixture(&repo, "gpmd-and-tx3g.mp4", "Foo Bar.mp4");
    repo.create_markdown("note.md", "# Note\n\n![clip](<Foo Bar.mp4>)");

    let server = TestServer::start(&repo).await;
    let response = server.get("/note/errors.json").await;

    assert_eq!(response.status(), 200);
    let json: serde_json::Value = response.json().await.unwrap();
    let errors = json["errors"].as_array().unwrap();

    let unplayable: Vec<_> = errors
        .iter()
        .filter(|e| e["type"] == "unplayable_media")
        .collect();
    assert_eq!(
        unplayable.len(),
        1,
        "expected exactly one unplayable_media error in {:?}",
        errors
    );

    // This shape is a contract with the frontend component.
    let error = unplayable[0];
    assert_eq!(error["kind"], "video");
    assert_eq!(
        error["remedy"],
        "ffmpeg -i in.mp4 -map 0 -c copy -dn -movflags +faststart out.mp4"
    );
    // Marks the entry as a hint so the frontend keeps it out of the error
    // badge until the browser actually reports a failure for this src.
    assert_eq!(error["advisory"], true);

    // The heuristic has a known false positive, so the prose must name both
    // implicated tracks and must not claim the file is broken.
    let reason = error["reason"].as_str().expect("reason must be a string");
    assert!(reason.contains("'gpmd'"), "{reason}");
    assert!(reason.contains("'tx3g'"), "{reason}");
    assert!(reason.contains("most likely cause"), "{reason}");
    assert!(!reason.contains("cannot decode"), "too absolute: {reason}");

    // `src` must match the attribute the browser sees so the component can
    // join an error to a specific <video>.
    let src = error["src"].as_str().expect("src must be a string");
    let page_html = server.get_text("/note/").await;
    assert!(
        page_html.contains(&format!("src='{src}'"))
            || page_html.contains(&format!("src=\"{src}\"")),
        "errors.json src {src:?} does not appear as an attribute in the rendered page"
    );
}

/// The offending file resolves and serves fine — the diagnosis must not be
/// confused with the pre-existing "file is missing" check.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_unplayable_video_is_not_reported_as_broken_reference() {
    let repo = TestRepo::new();
    copy_video_fixture(&repo, "gpmd-and-tx3g.mp4", "clip.mp4");
    repo.create_markdown("note.md", "# Note\n\n![clip](clip.mp4)");

    let server = TestServer::start(&repo).await;
    let json: serde_json::Value = server.get("/note/errors.json").await.json().await.unwrap();
    let errors = json["errors"].as_array().unwrap();

    assert!(
        !errors.iter().any(|e| e["type"] == "broken_media_reference"),
        "a servable file must not be reported broken: {:?}",
        errors
    );
    assert!(errors.iter().any(|e| e["type"] == "unplayable_media"));
}

/// The three non-flagging quadrants of the bisected 2x2. Each of these
/// measured as *playing* in Safari, so flagging any of them would put a scary
/// notice on a working video — the exact failure mode this heuristic must
/// avoid.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_errors_json_does_not_flag_incomplete_risky_combinations() {
    for fixture in ["gpmd-only.mp4", "tx3g-only.mp4", "neither.mp4"] {
        let repo = TestRepo::new();
        copy_video_fixture(&repo, fixture, "fine.mp4");
        repo.create_markdown("note.md", "# Note\n\n![clip](fine.mp4)");

        let server = TestServer::start(&repo).await;
        let json: serde_json::Value = server.get("/note/errors.json").await.json().await.unwrap();
        let errors = json["errors"].as_array().unwrap();

        assert!(
            !errors.iter().any(|e| e["type"] == "unplayable_media"),
            "{fixture} plays in Safari and must not be flagged: {errors:?}"
        );
    }
}

/// The same clip embedded twice yields one diagnosis, so the endpoint never
/// probes the same bytes more than once per page.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_errors_json_dedupes_repeated_unplayable_embeds() {
    let repo = TestRepo::new();
    copy_video_fixture(&repo, "gpmd-and-tx3g.mp4", "clip.mp4");
    repo.create_markdown("note.md", "# Note\n\n![a](clip.mp4)\n\n![b](clip.mp4)");

    let server = TestServer::start(&repo).await;
    let json: serde_json::Value = server.get("/note/errors.json").await.json().await.unwrap();
    let errors = json["errors"].as_array().unwrap();

    assert_eq!(
        errors
            .iter()
            .filter(|e| e["type"] == "unplayable_media")
            .count(),
        1,
        "{:?}",
        errors
    );
}

// ==================== HLS Transcoding Security Tests ====================

/// HLS requests must not be able to reach files outside the served root via
/// path traversal (regression test for unvalidated path resolution in
/// `try_serve_hls_content`).
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_hls_traversal_blocked() {
    // The served root is a subdirectory of the temp dir; secret.mp4 lives
    // OUTSIDE it, as a sibling of the root.
    let temp_dir = tempfile::tempdir().unwrap();
    let outer = temp_dir.path().canonicalize().unwrap();
    let repo_root = outer.join("repo");
    std::fs::create_dir_all(repo_root.join(".mbr")).unwrap();
    std::fs::write(repo_root.join("readme.md"), "# Hello").unwrap();
    std::fs::write(outer.join("secret.mp4"), b"fake video outside served root").unwrap();

    let port = find_available_port();
    let server_root = repo_root.clone();
    let _handle = tokio::spawn(async move {
        let mut config = test_server_config(port, server_root);
        config.transcode_enabled = true;
        let server = mbr::server::Server::init(config).expect("Failed to initialize server");
        server.start().await.expect("test server failed to start");
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = mbr::http_client(Duration::from_secs(5));

    // Use a percent-encoded slash so the client doesn't normalize away the
    // ".." segment; the server decodes it back to "../secret-720p.m3u8".
    for path in ["/..%2Fsecret-720p.m3u8", "/..%2Fsecret-480p.m3u8"] {
        let response = client
            .get(format!("http://127.0.0.1:{port}{path}"))
            .send()
            .await
            .expect("Request failed");
        assert_eq!(
            response.status(),
            404,
            "HLS path traversal must be blocked with 404 for {path}"
        );
    }
}

// ============================================================================
// Stream-copy ("remux") HLS variant
// ============================================================================
//
// The frontend retries a `<video>` against these URLs *after* the browser has
// reported a real playback failure, because the offending track combination is
// not predictable. So the variant has to be reachable without `--transcode`:
// every server in this section leaves `transcode_enabled` at its default
// `false`, which is what `test_server_config` sets.
//
// `multitrack-24s.mp4` is a 24 s fixture with video, audio, a `tx3g` subtitle
// track and a data track, and a keyframe every 2 s — enough to exercise real
// multi-segment output, which the 1 s fixtures cannot.

/// URL of one part of a video's remux variant.
#[cfg(feature = "media-metadata")]
fn remux_url(video_url: &str, part: &str) -> String {
    format!("{video_url}-remux{part}")
}

/// Fetches every part of a video's remux variant and concatenates init +
/// segments into a single playable fMP4, the way a player would.
#[cfg(feature = "media-metadata")]
async fn fetch_remux_ladder(server: &TestServer, video_url: &str) -> (String, Vec<u8>) {
    let playlist_response = server.get(&remux_url(video_url, ".m3u8")).await;
    assert_eq!(
        playlist_response.status(),
        200,
        "playlist must be served without --transcode"
    );
    let playlist = playlist_response.text().await.unwrap();

    let init_response = server.get(&remux_url(video_url, "-init.mp4")).await;
    assert_eq!(init_response.status(), 200);
    let mut joined = init_response.bytes().await.unwrap().to_vec();

    let segment_count = playlist
        .lines()
        .filter(|line| line.ends_with(".m4s"))
        .count();
    assert!(segment_count > 0, "playlist listed no segments: {playlist}");
    for index in 0..segment_count {
        let response = server
            .get(&remux_url(video_url, &format!("-{index:03}.m4s")))
            .await;
        assert_eq!(response.status(), 200, "segment {index} must be served");
        joined.extend_from_slice(&response.bytes().await.unwrap());
    }

    (playlist, joined)
}

/// All three routes must serve, with the content types the frontend switches on,
/// and without `--transcode`.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_remux_routes_serve_without_transcode() {
    let repo = TestRepo::new();
    copy_video_fixture(&repo, "multitrack-24s.mp4", "videos/clip.mp4");
    let server = TestServer::start(&repo).await;

    for (part, expected_type) in [
        (".m3u8", "application/vnd.apple.mpegurl"),
        ("-init.mp4", "video/mp4"),
        ("-000.m4s", "video/iso.segment"),
    ] {
        let response = server.get(&remux_url("/videos/clip.mp4", part)).await;
        assert_eq!(
            response.status(),
            200,
            "{part} must be served without --transcode"
        );
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some(expected_type),
            "wrong content type for {part}"
        );
        assert!(
            !response.bytes().await.unwrap().is_empty(),
            "{part} must not be empty"
        );
    }
}

/// The playlist must be internally consistent: version 7 or later for
/// `#EXT-X-MAP`, a `TARGETDURATION` no smaller than any `EXTINF`, and every
/// listed URI actually fetchable.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_remux_playlist_is_self_consistent() {
    let repo = TestRepo::new();
    copy_video_fixture(&repo, "multitrack-24s.mp4", "videos/clip.mp4");
    let server = TestServer::start(&repo).await;

    let (playlist, _joined) = fetch_remux_ladder(&server, "/videos/clip.mp4").await;

    assert!(playlist.starts_with("#EXTM3U"), "{playlist}");
    assert!(playlist.contains("#EXT-X-ENDLIST"), "{playlist}");

    let version: u32 = playlist
        .lines()
        .find_map(|line| line.strip_prefix("#EXT-X-VERSION:"))
        .expect("version tag")
        .parse()
        .expect("numeric version");
    assert!(version >= 7, "EXT-X-MAP needs version >= 7, got {version}");

    assert!(
        playlist.contains("#EXT-X-MAP:URI=\"clip.mp4-remux-init.mp4\""),
        "EXT-X-MAP must point at the init segment: {playlist}"
    );

    let target: f64 = playlist
        .lines()
        .find_map(|line| line.strip_prefix("#EXT-X-TARGETDURATION:"))
        .expect("target duration")
        .parse()
        .expect("numeric target");
    let extinfs: Vec<f64> = playlist
        .lines()
        .filter_map(|line| line.strip_prefix("#EXTINF:"))
        .map(|value| value.trim_end_matches(',').parse().expect("numeric EXTINF"))
        .collect();

    assert!(
        extinfs.len() > 1,
        "a 24s fixture must yield several segments: {playlist}"
    );
    for extinf in &extinfs {
        assert!(
            *extinf <= target,
            "EXTINF {extinf} exceeds TARGETDURATION {target}"
        );
    }

    // The playlist timeline must match the source's 24 s duration.
    let total: f64 = extinfs.iter().sum();
    assert!(
        (total - 24.0).abs() < 0.5,
        "playlist covers {total}s, expected ~24s"
    );
}

/// The point of the whole variant: the served bytes must no longer carry the
/// data or subtitle tracks implicated in the WebKit decode failure.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_remux_drops_data_and_subtitle_tracks() {
    for fixture in ["gpmd-and-tx3g.mp4", "multitrack-24s.mp4"] {
        let repo = TestRepo::new();
        copy_video_fixture(&repo, fixture, "videos/clip.mp4");
        let server = TestServer::start(&repo).await;

        let (_playlist, joined) = fetch_remux_ladder(&server, "/videos/clip.mp4").await;

        // Sample entries live uncompressed in `moov`, so a surviving track would
        // show its FourCC verbatim.
        for tag in [b"gpmd".as_slice(), b"tx3g".as_slice()] {
            assert!(
                !joined.windows(4).any(|window| window == tag),
                "{fixture}: remux output still carries a '{}' track",
                String::from_utf8_lossy(tag)
            );
        }

        // Probe the bytes a player would receive.
        let temp = tempfile::Builder::new()
            .suffix(".mp4")
            .tempfile()
            .expect("temp file");
        std::fs::write(temp.path(), &joined).expect("write joined fMP4");

        let metadata =
            mbr::video_metadata::probe_video(temp.path()).expect("remux output must be readable");
        assert!(
            !metadata.has_subtitles,
            "{fixture}: remux output must have no subtitle track"
        );

        // And the advisory heuristic that flagged the source must no longer fire.
        let compat = mbr::video_metadata::probe_playback_compatibility(temp.path())
            .expect("remux output must be probeable");
        assert!(
            !compat.has_known_risk(),
            "{fixture}: remux output still matches the risky combination: {compat:?}"
        );
    }
}

/// The source must be left untouched — this is a serve-time repair, not an edit.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_remux_does_not_modify_the_source_file() {
    let repo = TestRepo::new();
    copy_video_fixture(&repo, "gpmd-and-tx3g.mp4", "videos/clip.mp4");
    let before = std::fs::read(repo.path().join("videos/clip.mp4")).unwrap();

    let server = TestServer::start(&repo).await;
    let (_playlist, _joined) = fetch_remux_ladder(&server, "/videos/clip.mp4").await;

    let after = std::fs::read(repo.path().join("videos/clip.mp4")).unwrap();
    assert_eq!(before, after, "the original file must not be rewritten");
    // The original URL still serves the original bytes.
    let original = server.get("/videos/clip.mp4").await;
    assert_eq!(original.status(), 200);
    assert_eq!(original.bytes().await.unwrap().as_ref(), before.as_slice());
}

/// Repeated requests must be served from the cache byte-for-byte, so a player
/// re-fetching a segment never gets a different answer.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_remux_segment_is_stable_across_requests() {
    let repo = TestRepo::new();
    copy_video_fixture(&repo, "multitrack-24s.mp4", "videos/clip.mp4");
    let server = TestServer::start(&repo).await;

    let first = server.get("/videos/clip.mp4-remux-002.m4s").await;
    assert_eq!(first.status(), 200);
    let first_bytes = first.bytes().await.unwrap();

    let second = server.get("/videos/clip.mp4-remux-002.m4s").await;
    assert_eq!(second.status(), 200);
    let second_bytes = second.bytes().await.unwrap();

    assert_eq!(first_bytes, second_bytes);
}

/// A segment index past the end must 404 rather than return an empty body a
/// player would sit waiting on.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_remux_segment_out_of_range_is_not_found() {
    let repo = TestRepo::new();
    copy_video_fixture(&repo, "multitrack-24s.mp4", "videos/clip.mp4");
    let server = TestServer::start(&repo).await;

    let response = server.get("/videos/clip.mp4-remux-999.m4s").await;
    assert_eq!(response.status(), 404);
}

/// Remux URLs must only apply to real video files.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_remux_ignores_non_video_and_missing_sources() {
    let repo = TestRepo::new();
    repo.create_markdown("note.md", "# Note");
    repo.create_static_file("docs/report.pdf", b"%PDF-1.4 fake");
    let server = TestServer::start(&repo).await;

    for path in [
        // Not a video extension.
        "/docs/report.pdf-remux.m3u8",
        "/note.md-remux-000.m4s",
        // Video extension, but no such file.
        "/videos/missing.mp4-remux.m3u8",
        "/videos/missing.mp4-remux-init.mp4",
    ] {
        assert_eq!(
            server.get(path).await.status(),
            404,
            "{path} must not be treated as a remux request"
        );
    }
}

/// A file that ffmpeg cannot open must produce a status, not a hanging player.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_remux_rejects_a_file_that_is_not_really_a_video() {
    let repo = TestRepo::new();
    repo.create_static_file("videos/bogus.mp4", b"this is not an mp4 at all");
    let server = TestServer::start(&repo).await;

    let response = server.get("/videos/bogus.mp4-remux.m3u8").await;
    assert!(
        response.status().is_client_error() || response.status().is_server_error(),
        "expected an error status, got {}",
        response.status()
    );
    assert_ne!(
        response.status(),
        200,
        "an unreadable file must never yield an empty playlist"
    );
}

/// Remux requests must not be able to reach files outside the served root, the
/// same guarantee `test_hls_traversal_blocked` gives the transcode ladder.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_remux_traversal_blocked() {
    // The served root is a subdirectory of the temp dir; the video lives
    // OUTSIDE it, as a sibling of the root.
    let temp_dir = tempfile::tempdir().unwrap();
    let outer = temp_dir.path().canonicalize().unwrap();
    let repo_root = outer.join("repo");
    std::fs::create_dir_all(repo_root.join(".mbr")).unwrap();
    std::fs::write(repo_root.join("readme.md"), "# Hello").unwrap();

    // A *real, remuxable* video outside the root, so a 404 proves containment
    // rather than merely that the source could not be read.
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/videos")
        .join("multitrack-24s.mp4");
    std::fs::copy(&fixture, outer.join("secret.mp4")).unwrap();

    let server = TestServer::start_at_path(repo_root).await;

    // Percent-encoded slashes so the client does not normalize the ".." away;
    // the server decodes them back to "../secret.mp4-remux...".
    for path in [
        "/..%2Fsecret.mp4-remux.m3u8",
        "/..%2Fsecret.mp4-remux-init.mp4",
        "/..%2Fsecret.mp4-remux-000.m4s",
    ] {
        assert_eq!(
            server.get(path).await.status(),
            404,
            "remux path traversal must be blocked for {path}"
        );
    }
}

/// The remux URLs must not shadow the transcode ladder or the metadata
/// sidecars, and must not be shadowed by them.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_remux_urls_do_not_collide_with_other_video_routes() {
    let repo = TestRepo::new();
    copy_video_fixture(&repo, "multitrack-24s.mp4", "videos/clip.mp4");
    let server = TestServer::start(&repo).await;

    // The transcode ladder is disabled here, so its URLs must still 404 while
    // the remux URLs succeed — proof the two are routed independently.
    for path in ["/videos/clip-720p.m3u8", "/videos/clip-480p-000.ts"] {
        assert_eq!(
            server.get(path).await.status(),
            404,
            "{path} must stay unavailable without --transcode"
        );
    }
    assert_eq!(
        server.get("/videos/clip.mp4-remux.m3u8").await.status(),
        200,
        "the remux variant must not depend on --transcode"
    );

    // The metadata sidecar routes still work for the same file.
    let cover = server.get("/videos/clip.mp4.cover.jpg").await;
    assert_eq!(
        cover.status(),
        200,
        "the cover sidecar must be unaffected by the remux routes"
    );
    assert_eq!(
        cover
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("image/jpeg")
    );
}

/// A request abandoned mid-generation must not poison its segment.
///
/// This is the shape Safari's HLS loader produces constantly: it starts a segment
/// prefetch and drops it. axum drops the request future on disconnect, so any
/// completion bookkeeping awaited in the request path is simply skipped — which
/// used to leave the cache key marked in-progress forever. Every later request
/// for that segment then waited out the 60 s in-flight timeout with no work
/// running, so `<video>` stalled at t=0 with no error to report: strictly worse
/// than the playback failure this feature exists to repair.
///
/// The aborts here race real generation, so a given run may or may not land
/// mid-flight. That asymmetry is fine: a missed race passes exactly as correct
/// code does, while a landed race on the old code hangs and fails. Twenty-odd
/// aborts on a cold cache reproduced it every time. The deterministic coverage of
/// the same invariant lives in `video_transcode_cache`'s cancellation tests.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_aborted_segment_request_does_not_poison_the_segment() {
    let repo = TestRepo::new();
    copy_video_fixture(&repo, "multitrack-24s.mp4", "videos/clip.mp4");
    let server = TestServer::start(&repo).await;

    let segment_count = server
        .get_text("/videos/clip.mp4-remux.m3u8")
        .await
        .lines()
        .filter(|line| line.ends_with(".m4s"))
        .count();
    assert!(segment_count > 1, "fixture must yield several segments");

    // Abandon a request for every segment, at a spread of deadlines so some land
    // while generation is in flight. Dropping the `send()` future closes the
    // connection, which is what the server sees when a client goes away.
    for index in 0..segment_count {
        for micros in [1u64, 50, 250, 1_000] {
            let url = server.url(&format!("/videos/clip.mp4-remux-{index:03}.m4s"));
            let _ = tokio::time::timeout(
                Duration::from_micros(micros),
                mbr::http_client(Duration::from_secs(5)).get(url).send(),
            )
            .await;
        }
    }

    // Every segment must still be servable, promptly. The bound is far below the
    // 60 s in-flight timeout, so a leaked marker fails fast instead of stalling
    // the suite.
    for index in 0..segment_count {
        let path = format!("/videos/clip.mp4-remux-{index:03}.m4s");
        let started = std::time::Instant::now();
        let response = tokio::time::timeout(Duration::from_secs(10), server.get(&path))
            .await
            .unwrap_or_else(|_| {
                panic!("{path} did not respond within 10s — the segment is wedged in-progress")
            });

        assert_eq!(
            response.status(),
            200,
            "{path} must still serve after an aborted request"
        );
        assert!(
            !response.bytes().await.unwrap().is_empty(),
            "{path} served an empty body"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{path} took {:?}, which means it waited on a stale in-flight marker",
            started.elapsed()
        );
    }
}

/// The same guarantee for the playlist and init segment, which a player fetches
/// first and is therefore the most likely thing to be abandoned.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_aborted_playlist_request_does_not_poison_the_variant() {
    let repo = TestRepo::new();
    copy_video_fixture(&repo, "multitrack-24s.mp4", "videos/clip.mp4");
    let server = TestServer::start(&repo).await;

    for part in [".m3u8", "-init.mp4"] {
        for micros in [1u64, 100, 500] {
            let url = server.url(&format!("/videos/clip.mp4-remux{part}"));
            let _ = tokio::time::timeout(
                Duration::from_micros(micros),
                mbr::http_client(Duration::from_secs(5)).get(url).send(),
            )
            .await;
        }
    }

    // The whole ladder must still be fetchable, and the playlist still well-formed.
    let (playlist, joined) = tokio::time::timeout(
        Duration::from_secs(20),
        fetch_remux_ladder(&server, "/videos/clip.mp4"),
    )
    .await
    .expect("the remux ladder must not be wedged after aborted requests");

    assert!(playlist.starts_with("#EXTM3U"), "{playlist}");
    assert!(!joined.is_empty());
}

/// A file name with spaces must survive the round trip: mbr percent-encodes the
/// playlist URIs (a raw space is not a legal URI), and the request path is
/// decoded again before parsing.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_remux_handles_spaces_in_file_names() {
    let repo = TestRepo::new();
    copy_video_fixture(
        &repo,
        "multitrack-24s.mp4",
        "videos/Eric Jones - Metal 3.mp4",
    );
    let server = TestServer::start(&repo).await;

    let playlist = server
        .get_text("/videos/Eric%20Jones%20-%20Metal%203.mp4-remux.m3u8")
        .await;
    assert!(
        playlist.contains("Eric%20Jones%20-%20Metal%203.mp4-remux-000.m4s"),
        "segment URIs must be percent-encoded: {playlist}"
    );
    assert!(
        !playlist.contains(' '),
        "no playlist line may contain a raw space: {playlist}"
    );

    // And the encoded URI a player would follow must actually resolve.
    let segment = server
        .get("/videos/Eric%20Jones%20-%20Metal%203.mp4-remux-000.m4s")
        .await;
    assert_eq!(segment.status(), 200);
    assert!(!segment.bytes().await.unwrap().is_empty());
}

/// A static file whose name genuinely ends in a remux suffix must win, because
/// the real file resolves before the fallback handlers run.
#[cfg(feature = "media-metadata")]
#[tokio::test]
async fn test_real_file_named_like_a_remux_url_is_served_verbatim() {
    let repo = TestRepo::new();
    copy_video_fixture(&repo, "multitrack-24s.mp4", "videos/clip.mp4");
    repo.create_static_file("videos/clip.mp4-remux.m3u8", b"#EXTM3U\n# hand written\n");
    let server = TestServer::start(&repo).await;

    let body = server.get_text("/videos/clip.mp4-remux.m3u8").await;
    assert!(
        body.contains("hand written"),
        "an existing file must take precedence: {body}"
    );
}

// ==================== Editing endpoint tests ====================

/// Enables editing on a loopback test server.
fn enable_editing(config: &mut mbr::server::ServerConfig) {
    config.edit_enabled = true;
}

#[tokio::test]
async fn test_edit_disabled_returns_403() {
    let repo = TestRepo::new();
    repo.create_markdown("note.md", "# Note\n\nbody");
    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    let raw = server
        .client
        .get(server.url("/.mbr/raw/note.md"))
        .header("X-MBR-Edit", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(raw.status(), 403, "raw fetch must be 403 when editing off");

    let edit = server
        .client
        .post(server.url("/.mbr/edit/note.md"))
        .header("X-MBR-Edit", "1")
        .header("Content-Type", "application/json")
        .body(r#"{"content":"x","base_hash":"y"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(edit.status(), 403, "save must be 403 when editing off");
}

#[tokio::test]
async fn test_edit_missing_csrf_header_returns_403() {
    let repo = TestRepo::new();
    repo.create_markdown("note.md", "# Note\n\nbody");
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    // No X-MBR-Edit header → rejected even though editing is enabled + loopback.
    let raw = server
        .client
        .get(server.url("/.mbr/raw/note.md"))
        .send()
        .await
        .unwrap();
    assert_eq!(raw.status(), 403, "missing X-MBR-Edit must be 403");
}

#[tokio::test]
async fn test_edit_cross_origin_blocked() {
    let repo = TestRepo::new();
    repo.create_markdown("note.md", "# Note\n\nbody");
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    let raw = server
        .client
        .get(server.url("/.mbr/raw/note.md"))
        .header("X-MBR-Edit", "1")
        .header("Origin", "http://evil.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(raw.status(), 403, "cross-origin edit must be 403");
}

#[tokio::test]
async fn test_edit_roundtrip_loopback_no_token() {
    let repo = TestRepo::new();
    let original = "# Note\n\noriginal body";
    let file = repo.create_markdown("note.md", original);
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    // Fetch raw + hash.
    let raw = server
        .client
        .get(server.url("/.mbr/raw/note.md"))
        .header("X-MBR-Edit", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(raw.status(), 200);
    let base_hash = raw
        .headers()
        .get("x-mbr-content-hash")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(base_hash, mbr::edit_auth::content_hash(original.as_bytes()));
    let fetched = raw.text().await.unwrap();
    assert_eq!(fetched, original);

    // Save new content.
    let new_content = "---\ntitle: Edited\n---\n# Note\n\nedited body";
    let body = serde_json::json!({ "content": new_content, "base_hash": base_hash }).to_string();
    let save = server
        .client
        .post(server.url("/.mbr/edit/note.md"))
        .header("X-MBR-Edit", "1")
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(
        save.status(),
        200,
        "loopback save without token should succeed"
    );
    let new_hash = save
        .headers()
        .get("x-mbr-content-hash")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(
        new_hash,
        mbr::edit_auth::content_hash(new_content.as_bytes())
    );

    // Verify on disk.
    let on_disk = std::fs::read_to_string(&file).unwrap();
    assert_eq!(on_disk, new_content);
}

#[tokio::test]
async fn test_edit_stale_hash_returns_409() {
    let repo = TestRepo::new();
    let original = "# Note\n\nbody";
    repo.create_markdown("note.md", original);
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    let body = serde_json::json!({
        "content": "new content",
        "base_hash": "0000000000000000000000000000000000000000000000000000000000000000",
    })
    .to_string();
    let save = server
        .client
        .post(server.url("/.mbr/edit/note.md"))
        .header("X-MBR-Edit", "1")
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(save.status(), 409, "stale base_hash must be 409");
}

#[tokio::test]
async fn test_edit_nonexistent_and_non_markdown_rejected() {
    let repo = TestRepo::new();
    repo.create_markdown("note.md", "# Note");
    repo.create_static_file("data.txt", b"not markdown");
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    for path in ["/.mbr/raw/missing.md", "/.mbr/raw/data.txt"] {
        let resp = server
            .client
            .get(server.url(path))
            .header("X-MBR-Edit", "1")
            .send()
            .await
            .unwrap();
        assert!(
            resp.status() == 404 || resp.status() == 400,
            "editing {path} must be rejected, got {}",
            resp.status()
        );
    }
}

#[tokio::test]
async fn test_edit_token_required_and_accepted() {
    let repo = TestRepo::new();
    let original = "# Note\n\nbody";
    repo.create_markdown("note.md", original);
    let hash = mbr::edit_auth::hash_token("s3cret-token").unwrap();
    let server = TestServer::start_with_config_fn(&repo, move |config| {
        config.edit_enabled = true;
        config.edit_require_token_on_loopback = true;
        config.edit_token_hash = Some(hash.clone());
    })
    .await;
    server.wait_for_scan().await;

    // Without a token → 401 even on loopback.
    let no_token = server
        .client
        .get(server.url("/.mbr/raw/note.md"))
        .header("X-MBR-Edit", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(no_token.status(), 401, "missing token must be 401");

    // With a wrong token → 401.
    let wrong = server
        .client
        .get(server.url("/.mbr/raw/note.md"))
        .header("X-MBR-Edit", "1")
        .header("Authorization", "Bearer nope")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401, "wrong token must be 401");

    // With the right token → 200.
    let ok = server
        .client
        .get(server.url("/.mbr/raw/note.md"))
        .header("X-MBR-Edit", "1")
        .header("Authorization", "Bearer s3cret-token")
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200, "valid token must be accepted");
}

#[tokio::test]
async fn test_edit_token_enforced_for_loopback_callers_when_configured() {
    let repo = TestRepo::new();
    let original = "# Note\n\nbody";
    let file = repo.create_markdown("note.md", original);
    let hash = mbr::edit_auth::hash_token("s3cret-token").unwrap();
    // `edit_require_token_on_loopback` stays FALSE on purpose: this is the
    // reverse-proxy deployment documented in docs/modes/editing.md, where every
    // proxied request reaches mbr from 127.0.0.1. A configured token must still
    // be enforced, or the proxy silently disables authentication entirely.
    let server = TestServer::start_with_config_fn(&repo, move |config| {
        config.edit_enabled = true;
        config.edit_token_hash = Some(hash.clone());
    })
    .await;
    server.wait_for_scan().await;

    let no_token = server
        .client
        .get(server.url("/.mbr/raw/note.md"))
        .header("X-MBR-Edit", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(
        no_token.status(),
        401,
        "a configured token must be required even from loopback"
    );

    // Writes are gated too, and the file is untouched.
    let save = server
        .client
        .post(server.url("/.mbr/edit/note.md"))
        .header("X-MBR-Edit", "1")
        .header("Content-Type", "application/json")
        .body(
            serde_json::json!({
                "content": "pwned",
                "base_hash": mbr::edit_auth::content_hash(original.as_bytes()),
            })
            .to_string(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(save.status(), 401, "unauthenticated save must be 401");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);

    // The token holder still gets through.
    let ok = server
        .client
        .get(server.url("/.mbr/raw/note.md"))
        .header("X-MBR-Edit", "1")
        .header("Authorization", "Bearer s3cret-token")
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200, "valid token must be accepted");
}

#[tokio::test]
async fn test_edit_rejects_dns_rebinding_host_header() {
    let repo = TestRepo::new();
    repo.create_markdown("note.md", "# Note\n\nbody");
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    // A DNS-rebound page is genuinely same-origin, so only the Host name gives
    // it away. No token is configured, so the Host must name this server.
    for host in ["evil.example.com", "localhost.evil.example.com"] {
        let rebound = server
            .client
            .get(server.url("/.mbr/raw/note.md"))
            .header("X-MBR-Edit", "1")
            .header("Host", host)
            .send()
            .await
            .unwrap();
        assert_eq!(
            rebound.status(),
            403,
            "Host {host} must be rejected, got {}",
            rebound.status()
        );
    }

    // The names the local UI actually uses keep working.
    for host in [
        format!("localhost:{}", server.port),
        format!("127.0.0.1:{}", server.port),
    ] {
        let allowed = server
            .client
            .get(server.url("/.mbr/raw/note.md"))
            .header("X-MBR-Edit", "1")
            .header("Host", &host)
            .send()
            .await
            .unwrap();
        assert_eq!(allowed.status(), 200, "Host {host} must be allowed");
    }
}

#[tokio::test]
async fn test_edit_allows_any_host_when_token_configured() {
    let repo = TestRepo::new();
    repo.create_markdown("note.md", "# Note\n\nbody");
    let hash = mbr::edit_auth::hash_token("s3cret-token").unwrap();
    let server = TestServer::start_with_config_fn(&repo, move |config| {
        config.edit_enabled = true;
        config.edit_token_hash = Some(hash.clone());
    })
    .await;
    server.wait_for_scan().await;

    // Behind a reverse proxy the Host is the public name, so the token — not
    // the Host — is the authority.
    let proxied = server
        .client
        .get(server.url("/.mbr/raw/note.md"))
        .header("X-MBR-Edit", "1")
        .header("Host", "notes.example.com")
        .header("Authorization", "Bearer s3cret-token")
        .send()
        .await
        .unwrap();
    assert_eq!(
        proxied.status(),
        200,
        "a valid token must work behind a proxy with any Host"
    );

    // ...but the token is still the gate.
    let no_token = server
        .client
        .get(server.url("/.mbr/raw/note.md"))
        .header("X-MBR-Edit", "1")
        .header("Host", "notes.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(no_token.status(), 401);
}

// ==================== File-management editing endpoints ====================

/// POST helper carrying the CSRF header + JSON body for the file endpoints.
async fn edit_post(server: &TestServer, path: &str, body: serde_json::Value) -> reqwest::Response {
    server
        .client
        .post(server.url(path))
        .header("X-MBR-Edit", "1")
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .expect("request failed")
}

#[tokio::test]
async fn test_create_markdown_happy_path() {
    let repo = TestRepo::new();
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    let resp = edit_post(
        &server,
        "/.mbr/create/hello.md",
        serde_json::json!({ "content": "# Hello\n\nbody" }),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["url_path"], "/hello/");
    assert_eq!(json["path"], "hello.md");

    let on_disk = std::fs::read_to_string(repo.path().join("hello.md")).unwrap();
    assert_eq!(on_disk, "# Hello\n\nbody");

    // The new page is served immediately.
    assert_eq!(server.get("/hello/").await.status(), 200);
}

#[tokio::test]
async fn test_create_markdown_create_dirs() {
    let repo = TestRepo::new();
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    // Missing parent without create_dirs → 400, nothing written.
    let resp = edit_post(
        &server,
        "/.mbr/create/new/deep/note.md",
        serde_json::json!({ "content": "# Deep" }),
    )
    .await;
    assert_eq!(resp.status(), 400);
    assert!(!repo.path().join("new/deep/note.md").exists());

    // With create_dirs → 200 and directories created.
    let resp = edit_post(
        &server,
        "/.mbr/create/new/deep/note.md",
        serde_json::json!({ "content": "# Deep", "create_dirs": true }),
    )
    .await;
    assert_eq!(resp.status(), 200);
    assert!(repo.path().join("new/deep/note.md").exists());
}

#[tokio::test]
async fn test_create_markdown_collision_409() {
    let repo = TestRepo::new();
    repo.create_markdown("exists.md", "# Exists");
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    let resp = edit_post(
        &server,
        "/.mbr/create/exists.md",
        serde_json::json!({ "content": "# New" }),
    )
    .await;
    assert_eq!(resp.status(), 409);
    // Original content preserved.
    assert_eq!(
        std::fs::read_to_string(repo.path().join("exists.md")).unwrap(),
        "# Exists"
    );
}

#[tokio::test]
async fn test_create_markdown_non_markdown_extension_400() {
    let repo = TestRepo::new();
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    let resp = edit_post(
        &server,
        "/.mbr/create/notes.txt",
        serde_json::json!({ "content": "not markdown" }),
    )
    .await;
    assert_eq!(resp.status(), 400);
    assert!(!repo.path().join("notes.txt").exists());
}

#[tokio::test]
async fn test_create_markdown_traversal_rejected() {
    let repo = TestRepo::new();
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    // Percent-encoded slash keeps the ".." segment intact through the client.
    let resp = server
        .client
        .post(server.url("/.mbr/create/..%2Fescape.md"))
        .header("X-MBR-Edit", "1")
        .header("Content-Type", "application/json")
        .body(r#"{"content":"x"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    assert!(!repo.path().parent().unwrap().join("escape.md").exists());
}

#[tokio::test]
async fn test_move_markdown_rewrites_inbound_links() {
    let repo = TestRepo::new();
    repo.create_markdown("guide.md", "# Guide\n\nThe guide.");
    // Three inbound link forms from a sibling folder: absolute, relative,
    // and a reference definition (its [g][ref] use must stay untouched).
    repo.create_markdown("refs/abs.md", "See [g](/guide/).");
    repo.create_markdown("refs/rel.md", "See [g](../guide/).");
    repo.create_markdown("refs/refdef.md", "See [g][ref].\n\n[ref]: /guide/\n");
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    let resp = edit_post(
        &server,
        "/.mbr/move/guide.md",
        serde_json::json!({ "to": "manual.md" }),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["from_url"], "/guide/");
    assert_eq!(json["url_path"], "/manual/");

    // All three inbound forms rewritten on disk.
    assert_eq!(
        std::fs::read_to_string(repo.path().join("refs/abs.md")).unwrap(),
        "See [g](/manual/)."
    );
    assert_eq!(
        std::fs::read_to_string(repo.path().join("refs/rel.md")).unwrap(),
        "See [g](../manual/)."
    );
    let refdef = std::fs::read_to_string(repo.path().join("refs/refdef.md")).unwrap();
    assert!(
        refdef.contains("[ref]: /manual/"),
        "ref def rewritten: {refdef}"
    );
    assert!(refdef.contains("[g][ref]"), "ref use untouched: {refdef}");

    // MoveResponse.rewritten lists the three source pages.
    let urls: Vec<&str> = json["rewritten"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(urls.contains(&"/refs/abs/"), "rewritten={urls:?}");
    assert!(urls.contains(&"/refs/rel/"), "rewritten={urls:?}");
    assert!(urls.contains(&"/refs/refdef/"), "rewritten={urls:?}");

    // Old file gone, new file present; old URL 404s, new URL serves.
    assert!(!repo.path().join("guide.md").exists());
    assert!(repo.path().join("manual.md").exists());
    assert_eq!(server.get("/guide/").await.status(), 404);
    assert_eq!(server.get("/manual/").await.status(), 200);
}

#[tokio::test]
async fn test_move_markdown_rename_rewrites_bare_wikilinks_with_guard() {
    let repo = TestRepo::new();
    // The note to rename (title differs from stem so [[alpha]] resolves by stem).
    repo.create_markdown("x/alpha.md", "# Alpha X\n\nbody");
    // A referrer whose [[alpha]] resolves globally to /x/alpha/.
    repo.create_markdown("notes/ref.md", "See [[alpha]].");
    // A decoy note also named "alpha" in a later-sorting folder, plus a sibling
    // whose [[alpha]] resolves to the DECOY (same-folder-first), not /x/alpha/.
    repo.create_markdown("zdecoy/alpha.md", "# Zdecoy Alpha\n\nbody");
    repo.create_markdown("zdecoy/other.md", "See [[alpha]].");
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    let resp = edit_post(
        &server,
        "/.mbr/move/x/alpha.md",
        serde_json::json!({ "to": "x/omega.md" }),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();

    // Referrer resolving to the renamed note is rewritten...
    assert_eq!(
        std::fs::read_to_string(repo.path().join("notes/ref.md")).unwrap(),
        "See [[omega]]."
    );
    // ...but the decoy's sibling [[alpha]] (a DIFFERENT note) is left intact.
    assert_eq!(
        std::fs::read_to_string(repo.path().join("zdecoy/other.md")).unwrap(),
        "See [[alpha]]."
    );

    let wikis: Vec<&str> = json["wikilinks_rewritten"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(wikis.contains(&"/notes/ref/"), "wikilinks={wikis:?}");
    assert!(!wikis.contains(&"/zdecoy/other/"), "wikilinks={wikis:?}");

    assert!(!repo.path().join("x/alpha.md").exists());
    assert!(repo.path().join("x/omega.md").exists());
}

#[tokio::test]
async fn test_move_markdown_collision_409() {
    let repo = TestRepo::new();
    repo.create_markdown("guide.md", "# Guide");
    repo.create_markdown("other.md", "# Other");
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    let resp = edit_post(
        &server,
        "/.mbr/move/guide.md",
        serde_json::json!({ "to": "other.md" }),
    )
    .await;
    assert_eq!(resp.status(), 409);
    // Both files intact.
    assert!(repo.path().join("guide.md").exists());
    assert_eq!(
        std::fs::read_to_string(repo.path().join("other.md")).unwrap(),
        "# Other"
    );
}

#[tokio::test]
async fn test_move_markdown_traversal_rejected() {
    let repo = TestRepo::new();
    repo.create_markdown("guide.md", "# Guide");
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    let resp = edit_post(
        &server,
        "/.mbr/move/guide.md",
        serde_json::json!({ "to": "../escape.md" }),
    )
    .await;
    assert_eq!(resp.status(), 400);
    assert!(
        repo.path().join("guide.md").exists(),
        "source must be intact"
    );
    assert!(!repo.path().parent().unwrap().join("escape.md").exists());
}

#[tokio::test]
async fn test_mkdir_happy_and_idempotent() {
    let repo = TestRepo::new();
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    let resp = server
        .client
        .post(server.url("/.mbr/mkdir/newdir"))
        .header("X-MBR-Edit", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["path"], "newdir");
    assert!(repo.path().join("newdir").is_dir());

    // Idempotent: pre-creating an existing folder succeeds again.
    let resp2 = server
        .client
        .post(server.url("/.mbr/mkdir/newdir"))
        .header("X-MBR-Edit", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(resp2.status(), 200);
}

#[tokio::test]
async fn test_mkdir_file_in_the_way_409() {
    let repo = TestRepo::new();
    repo.create_markdown("occupied.md", "# Occupied");
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    let resp = server
        .client
        .post(server.url("/.mbr/mkdir/occupied.md"))
        .header("X-MBR-Edit", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 409);
}

#[tokio::test]
async fn test_file_ops_disabled_returns_403() {
    let repo = TestRepo::new();
    repo.create_markdown("guide.md", "# Guide");
    let server = TestServer::start(&repo).await; // editing OFF
    server.wait_for_scan().await;

    let create = edit_post(
        &server,
        "/.mbr/create/x.md",
        serde_json::json!({ "content": "x" }),
    )
    .await;
    assert_eq!(create.status(), 403);
    let mv = edit_post(
        &server,
        "/.mbr/move/guide.md",
        serde_json::json!({ "to": "y.md" }),
    )
    .await;
    assert_eq!(mv.status(), 403);
    let mk = server
        .client
        .post(server.url("/.mbr/mkdir/d"))
        .header("X-MBR-Edit", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(mk.status(), 403);
}

#[tokio::test]
async fn test_file_ops_missing_csrf_and_cross_origin_403() {
    let repo = TestRepo::new();
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    // Missing X-MBR-Edit header → 403.
    let no_csrf = server
        .client
        .post(server.url("/.mbr/create/x.md"))
        .header("Content-Type", "application/json")
        .body(r#"{"content":"x"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), 403);

    // Cross-origin request → 403.
    let cross = server
        .client
        .post(server.url("/.mbr/mkdir/d"))
        .header("X-MBR-Edit", "1")
        .header("Origin", "http://evil.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(cross.status(), 403);
}

#[tokio::test]
async fn test_file_ops_token_required_401() {
    let repo = TestRepo::new();
    let hash = mbr::edit_auth::hash_token("s3cret-token").unwrap();
    let server = TestServer::start_with_config_fn(&repo, move |config| {
        config.edit_enabled = true;
        config.edit_require_token_on_loopback = true;
        config.edit_token_hash = Some(hash.clone());
    })
    .await;
    server.wait_for_scan().await;

    // No token → 401 even on loopback.
    let no_token = edit_post(
        &server,
        "/.mbr/create/x.md",
        serde_json::json!({ "content": "x" }),
    )
    .await;
    assert_eq!(no_token.status(), 401);

    // Wrong token → 401.
    let wrong = server
        .client
        .post(server.url("/.mbr/create/x.md"))
        .header("X-MBR-Edit", "1")
        .header("Authorization", "Bearer nope")
        .header("Content-Type", "application/json")
        .body(r#"{"content":"x"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);

    // Correct token → 200.
    let ok = server
        .client
        .post(server.url("/.mbr/create/note.md"))
        .header("X-MBR-Edit", "1")
        .header("Authorization", "Bearer s3cret-token")
        .header("Content-Type", "application/json")
        .body(serde_json::json!({ "content": "# Note" }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
}

// ==================== Asset upload endpoint ====================

/// Minimal percent-encoder for a query-parameter value (RFC 3986 unreserved
/// pass through; everything else is `%XX` over UTF-8 bytes).
fn q(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Builds the `/.mbr/upload?dir=&name=` URL for the given params.
fn upload_url(dir: &str, name: &str) -> String {
    format!("/.mbr/upload?dir={}&name={}", q(dir), q(name))
}

/// POST helper for `/.mbr/upload` carrying the CSRF header + raw file bytes.
async fn upload_post(
    server: &TestServer,
    dir: &str,
    name: &str,
    bytes: Vec<u8>,
) -> reqwest::Response {
    server
        .client
        .post(server.url(&upload_url(dir, name)))
        .header("X-MBR-Edit", "1")
        .header("Content-Type", "application/octet-stream")
        .body(bytes)
        .send()
        .await
        .expect("request failed")
}

#[tokio::test]
async fn test_upload_happy_path() {
    let repo = TestRepo::new();
    // Upload lands next to an existing note in its own folder (the norm).
    repo.create_markdown("notes/note.md", "# Note");
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    let bytes = b"\x89PNG\r\n\x1a\nfake-png-bytes".to_vec();
    let resp = upload_post(&server, "notes", "pic.png", bytes.clone()).await;
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["url"], "/notes/pic.png");
    assert_eq!(json["path"], "notes/pic.png");
    assert_eq!(json["name"], "pic.png");

    // Exact bytes are on disk at the reported path.
    let on_disk = std::fs::read(repo.path().join("notes/pic.png")).unwrap();
    assert_eq!(on_disk, bytes);
}

#[tokio::test]
async fn test_upload_collision_suffixes_name() {
    let repo = TestRepo::new();
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    let first = upload_post(&server, "notes", "pic.png", b"one".to_vec()).await;
    assert_eq!(first.status(), 200);
    let first_json: serde_json::Value = first.json().await.unwrap();
    assert_eq!(first_json["url"], "/notes/pic.png");

    // A second upload of the same name keeps the name and suffixes `-1`.
    let second = upload_post(&server, "notes", "pic.png", b"two".to_vec()).await;
    assert_eq!(second.status(), 200);
    let second_json: serde_json::Value = second.json().await.unwrap();
    assert_eq!(second_json["url"], "/notes/pic-1.png");
    assert_eq!(second_json["name"], "pic-1.png");
    assert_eq!(second_json["path"], "notes/pic-1.png");

    // Both files exist with their own bytes.
    assert_eq!(
        std::fs::read(repo.path().join("notes/pic.png")).unwrap(),
        b"one"
    );
    assert_eq!(
        std::fs::read(repo.path().join("notes/pic-1.png")).unwrap(),
        b"two"
    );
}

#[tokio::test]
async fn test_upload_root_dir() {
    let repo = TestRepo::new();
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    // Empty dir → root-level note; url has no folder segment.
    let resp = upload_post(&server, "", "pic.png", b"root".to_vec()).await;
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["url"], "/pic.png");
    assert_eq!(json["path"], "pic.png");
    assert_eq!(json["name"], "pic.png");
    assert_eq!(std::fs::read(repo.path().join("pic.png")).unwrap(), b"root");
}

#[tokio::test]
async fn test_upload_auth_rejected() {
    let repo = TestRepo::new();

    // Editing disabled → 403 even with the CSRF header.
    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;
    let disabled = server
        .client
        .post(server.url(&upload_url("", "pic.png")))
        .header("X-MBR-Edit", "1")
        .body(b"x".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(disabled.status(), 403);
    assert!(!repo.path().join("pic.png").exists());

    // Editing enabled but missing the CSRF header → 403.
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;
    let no_csrf = server
        .client
        .post(server.url(&upload_url("", "pic.png")))
        .body(b"x".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), 403);
    assert!(!repo.path().join("pic.png").exists());
}

#[tokio::test]
async fn test_upload_traversal_rejected() {
    let repo = TestRepo::new();
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    // `dir=..` escapes the root → 400, nothing written outside.
    let dir_esc = upload_post(&server, "..", "pic.png", b"x".to_vec()).await;
    assert_eq!(dir_esc.status(), 400);
    assert!(!repo.path().parent().unwrap().join("pic.png").exists());

    // A name containing a separator / `..` → 400 (not a pure basename).
    let name_esc = upload_post(&server, "notes", "../evil.png", b"x".to_vec()).await;
    assert_eq!(name_esc.status(), 400);
    assert!(!repo.path().parent().unwrap().join("evil.png").exists());
}

#[tokio::test]
async fn test_upload_markdown_extension_rejected() {
    let repo = TestRepo::new();
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    // Markdown files must go through /.mbr/create, not /.mbr/upload.
    let resp = upload_post(&server, "notes", "foo.md", b"# not allowed".to_vec()).await;
    assert_eq!(resp.status(), 400);
    assert!(!repo.path().join("notes/foo.md").exists());
}

#[tokio::test]
async fn test_upload_body_limit_rejects_oversize() {
    let repo = TestRepo::new();
    let server = TestServer::start_with_config_fn(&repo, |config| {
        config.edit_enabled = true;
        config.upload_max_bytes = 1024; // 1 KiB cap for the test
    })
    .await;
    server.wait_for_scan().await;

    // A 2 KiB body exceeds the 1 KiB cap → 413 (from DefaultBodyLimit).
    let big = vec![0u8; 2048];
    let resp = upload_post(&server, "", "big.bin", big).await;
    assert_eq!(resp.status(), 413);
    assert!(!repo.path().join("big.bin").exists());
}

#[tokio::test]
async fn test_upload_into_template_folder_rejected() {
    let repo = TestRepo::new();
    std::fs::create_dir_all(repo.path().join(".mbr/components")).unwrap();
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    // Even with an allowed media extension, `.mbr/` is off limits: the watcher
    // hot-reloads templates and components from there into every page.
    for (dir, name) in [
        (".mbr", "pic.png"),
        (".mbr/components", "logo.png"),
        ("./.mbr", "pic.png"),
    ] {
        let resp = upload_post(&server, dir, name, b"x".to_vec()).await;
        assert!(
            resp.status().is_client_error(),
            "upload into {dir} must be rejected, got {}",
            resp.status()
        );
    }
    assert!(!repo.path().join(".mbr/pic.png").exists());
    assert!(!repo.path().join(".mbr/components/logo.png").exists());
}

#[tokio::test]
async fn test_upload_executable_extensions_rejected() {
    let repo = TestRepo::new();
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    // The asset uploader must never create a file that the template engine
    // executes or the browser runs as script/style.
    for name in [
        "index.html",
        "mbr-components.min.js",
        "theme.css",
        "config.toml",
        "evil.svg",
    ] {
        let resp = upload_post(&server, "notes", name, b"x".to_vec()).await;
        assert_eq!(
            resp.status(),
            400,
            "upload of {name} must be rejected with 400"
        );
        assert!(!repo.path().join("notes").join(name).exists());
    }

    // Real media still uploads (no regression to the editor's image picker).
    let ok = upload_post(&server, "notes", "pic.png", b"png-bytes".to_vec()).await;
    assert_eq!(ok.status(), 200);
    assert_eq!(
        std::fs::read(repo.path().join("notes/pic.png")).unwrap(),
        b"png-bytes"
    );
}

// ==================== /.mbr asset allowlist ====================

#[tokio::test]
async fn test_mbr_config_toml_is_not_served() {
    let repo = TestRepo::new();
    repo.create_markdown("note.md", "# Note");
    // A real repo config: this file holds the Argon2 edit_token_hash.
    std::fs::write(
        repo.path().join(".mbr/config.toml"),
        "edit_enabled = true\nedit_token_hash = \"$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA\"\n",
    )
    .unwrap();
    std::fs::write(repo.path().join(".mbr/theme.css"), "body { color: red; }").unwrap();
    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    let leaked = server.get("/.mbr/config.toml").await;
    assert_eq!(leaked.status(), 404, "config.toml must never be served");

    // Non-asset files under .mbr/ are equally invisible.
    std::fs::write(repo.path().join(".mbr/.env"), "SECRET=1").unwrap();
    assert_eq!(server.get("/.mbr/.env").await.status(), 404);

    // ...while real assets still come from the repo's .mbr/ folder.
    let css = server.get("/.mbr/theme.css").await;
    assert_eq!(css.status(), 200, "theme.css must still be served");
    assert_eq!(css.text().await.unwrap(), "body { color: red; }");
    assert_eq!(server.get("/.mbr/pico.min.css").await.status(), 200);
    assert_eq!(
        server
            .get("/.mbr/components/mbr-components.min.js")
            .await
            .status(),
        200
    );
}

// ==================== Symlink containment ====================

#[cfg(unix)]
#[tokio::test]
async fn test_symlink_escaping_repo_root_is_not_served() {
    let outside = tempfile::TempDir::new().unwrap();
    let secret = outside.path().join("secret.txt");
    std::fs::write(&secret, "top secret").unwrap();

    let repo = TestRepo::new();
    repo.create_markdown("note.md", "# Note");
    // A symlink whose target lives outside the repository root.
    std::os::unix::fs::symlink(&secret, repo.path().join("secret.txt")).unwrap();
    // ...and one reachable through a directory listing request.
    std::os::unix::fs::symlink(outside.path(), repo.path().join("outside")).unwrap();

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    let resp = server.get("/secret.txt").await;
    assert_eq!(
        resp.status(),
        404,
        "a symlink pointing outside the repo must not be served"
    );
    let body = resp.text().await.unwrap_or_default();
    assert!(
        !body.contains("top secret"),
        "file contents outside the repo leaked: {body}"
    );
    assert_eq!(
        server.get("/outside/secret.txt").await.status(),
        404,
        "a symlinked directory outside the repo must not be served"
    );
}

// ==================== Live-reload WebSocket handshake ====================

/// Performs a raw WebSocket handshake and returns the HTTP status line.
///
/// Done over a bare TCP socket because the handshake response is what we are
/// asserting on (101 vs 403), and no WebSocket client crate is available.
async fn ws_handshake_status_line(port: u16, origin: Option<&str>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect failed");
    let origin_header = origin
        .map(|o| format!("Origin: {o}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "GET /.mbr/ws/changes HTTP/1.1\r\n\
         Host: 127.0.0.1:{port}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\
         {origin_header}\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write failed");

    let mut buf = [0u8; 256];
    let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf))
        .await
        .expect("handshake timed out")
        .expect("read failed");
    String::from_utf8_lossy(&buf[..n])
        .lines()
        .next()
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn test_websocket_upgrade_requires_same_origin() {
    let repo = TestRepo::new();
    repo.create_markdown("note.md", "# Note");
    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    // Same-origin handshake succeeds.
    let same_origin = format!("http://127.0.0.1:{}", server.port);
    let ok = ws_handshake_status_line(server.port, Some(&same_origin)).await;
    assert!(
        ok.contains("101"),
        "same-origin WebSocket upgrade should succeed, got: {ok}"
    );

    // A page on any other origin must not be able to watch the file feed.
    let cross = ws_handshake_status_line(server.port, Some("http://evil.example.com")).await;
    assert!(
        cross.contains("403"),
        "cross-origin WebSocket upgrade must be 403, got: {cross}"
    );

    // Browsers always send Origin on a WS upgrade; a missing one is rejected.
    let no_origin = ws_handshake_status_line(server.port, None).await;
    assert!(
        no_origin.contains("403"),
        "WebSocket upgrade without Origin must be 403, got: {no_origin}"
    );
}

// ============================================================================
// Typed relationships (genealogy fixture)
// ============================================================================

/// Absolute path to the committed genealogy fixture.
fn genealogy_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/genealogy")
}

/// Fetch and parse a JSON endpoint.
async fn get_json(server: &TestServer, path: &str) -> serde_json::Value {
    let text = server.get_text(path).await;
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("invalid JSON from {path}: {e}\n{text}"))
}

#[tokio::test]
async fn test_relationships_in_links_json() {
    let server = TestServer::start_at_path(genealogy_fixture_path()).await;
    server.wait_for_scan().await;

    // John declares parent (George/Martha), spouse (Mary), sibling (Robert),
    // and children (Alice/Sam).
    let john = get_json(&server, "/people/john/links.json").await;
    let rels = john["relationships"]
        .as_array()
        .expect("relationships array");

    // Reciprocal parent edge resolves to George under predicate "parent".
    let george = rels
        .iter()
        .find(|r| r["neighbor"] == "/people/george/")
        .expect("john should have an edge to george");
    assert_eq!(george["predicate"], "parent");
    assert_eq!(george["resolved"], true);

    // Spouse edge to Mary carries the marriage attributes.
    let mary = rels
        .iter()
        .find(|r| r["neighbor"] == "/people/mary/")
        .expect("john should have an edge to mary");
    assert_eq!(mary["predicate"], "spouse");
    assert_eq!(mary["attributes"]["married"], "1948-06-01");
    assert_eq!(mary["attributes"]["place"], "Denver, CO");

    // Children appear under predicate "child".
    assert!(
        rels.iter()
            .any(|r| r["neighbor"] == "/people/alice/" && r["predicate"] == "child"),
        "john should list Alice as a child"
    );
}

#[tokio::test]
async fn test_derived_relationships_on_counterpart() {
    let server = TestServer::start_at_path(genealogy_fixture_path()).await;
    server.wait_for_scan().await;

    // Sam declares NO relationships, but should still see derived parents
    // (John, Mary) and a derived sibling (Alice).
    let sam = get_json(&server, "/people/sam/links.json").await;
    let rels = sam["relationships"]
        .as_array()
        .expect("relationships array");

    let parents: Vec<&str> = rels
        .iter()
        .filter(|r| r["predicate"] == "parent")
        .map(|r| r["neighbor"].as_str().unwrap_or(""))
        .collect();
    assert!(
        parents.contains(&"/people/john/"),
        "Sam's parents: {parents:?}"
    );
    assert!(
        parents.contains(&"/people/mary/"),
        "Sam's parents: {parents:?}"
    );

    assert!(
        rels.iter()
            .any(|r| r["predicate"] == "sibling" && r["neighbor"] == "/people/alice/"),
        "Sam should have derived sibling Alice"
    );

    // All of Sam's edges are derived (declared on other notes).
    assert!(rels.iter().all(|r| r["derived"] == true));
}

#[tokio::test]
async fn test_unresolved_relationship_endpoint_kept_raw() {
    let server = TestServer::start_at_path(genealogy_fixture_path()).await;
    server.wait_for_scan().await;

    // Robert has a spouse edge to an unknown "[[Jane Ghost]]".
    let robert = get_json(&server, "/people/robert/links.json").await;
    let rels = robert["relationships"]
        .as_array()
        .expect("relationships array");
    let ghost = rels
        .iter()
        .find(|r| r["neighbor_raw"] == "[[Jane Ghost]]")
        .expect("unresolved edge should be present");
    assert_eq!(ghost["resolved"], false);
    assert_eq!(ghost["neighbor"], "");
    assert_eq!(ghost["neighbor_title"], "Jane Ghost");
}

#[tokio::test]
async fn test_site_json_relationship_types_and_edges() {
    let server = TestServer::start_at_path(genealogy_fixture_path()).await;
    server.wait_for_scan().await;

    let site = get_json(&server, "/.mbr/site.json").await;

    // relationship_types registry is exposed with concrete labels.
    let types = site["relationship_types"]
        .as_array()
        .expect("relationship_types array");
    let child = types
        .iter()
        .find(|t| t["name"] == "child")
        .expect("child type");
    assert_eq!(child["label_plural"], "Children");
    assert_eq!(child["inverse"], "parent");

    // Per-note resolved relationships are attached to markdown_files entries.
    let files = site["markdown_files"].as_array().expect("markdown_files");
    let george = files
        .iter()
        .find(|f| f["url_path"] == "/people/george/")
        .expect("george entry");
    // In site.json, markdown_files entries carry frontmatter nested (the flat
    // `type` field is a directory-listing convenience, not a site.json field).
    assert_eq!(george["frontmatter"]["type"], "person");
    let george_rels = george["relationships"].as_array().expect("george rels");
    assert!(
        george_rels
            .iter()
            .any(|r| r["predicate"] == "child" && r["neighbor"] == "/people/john/"),
        "George should list John as a child in site.json"
    );
}

#[tokio::test]
async fn test_no_relationship_tracking_disables() {
    let server = TestServer::start_at_path_with(genealogy_fixture_path(), |c| {
        c.relationship_tracking = false;
    })
    .await;
    server.wait_for_scan().await;

    // links.json still serves, but with no relationships array.
    let john = get_json(&server, "/people/john/links.json").await;
    assert!(
        john.get("relationships").is_none()
            || john["relationships"].as_array().map(|a| a.is_empty()) == Some(true),
        "relationships must be absent/empty when tracking is disabled: {john}"
    );

    // site.json omits relationship_types entirely.
    let site = get_json(&server, "/.mbr/site.json").await;
    assert!(site.get("relationship_types").is_none());
}

/// The sidebar mini graph fans out many links.json requests at once. A burst
/// of uncached lookups must all succeed: the inbound grep is single-flighted
/// per page and bounded by a semaphore, so no request stampedes or hangs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_links_json_concurrent_burst_all_succeed() {
    let server = TestServer::start_at_path(genealogy_fixture_path()).await;
    server.wait_for_scan().await;

    // Every person page (distinct inbound greps) plus repeats of the same
    // pages (same-page single-flight) in one concurrent volley.
    let people = [
        "john", "mary", "george", "martha", "alice", "robert", "sam", // distinct
        "john", "mary", "george", // repeats exercise the single-flight path
    ];
    let handles: Vec<_> = people
        .iter()
        .map(|person| {
            let client = server.client.clone();
            let url = server.url(&format!("/people/{person}/links.json"));
            tokio::spawn(async move {
                let response = client.get(&url).send().await.expect("request failed");
                assert_eq!(response.status(), 200, "burst request failed for {url}");
                let text = response.text().await.expect("body read failed");
                let json: serde_json::Value = serde_json::from_str(&text)
                    .unwrap_or_else(|e| panic!("invalid JSON from {url}: {e}\n{text}"));
                assert!(json.is_object(), "expected a JSON object from {url}");
            })
        })
        .collect();

    for handle in handles {
        handle.await.expect("burst task panicked");
    }
}

#[tokio::test]
async fn test_person_page_renders_genealogy_element() {
    let server = TestServer::start_at_path(genealogy_fixture_path()).await;

    // Person pages get the d3 genealogy chart element, not the removed
    // mermaid relationships element.
    let html = server.get_text("/people/john/").await;
    assert_html_contains(&html, "<mbr-genealogy");
    assert_html_not_contains(&html, "<mbr-relationships");
}

#[tokio::test]
async fn test_typed_nonperson_page_has_no_graph_element() {
    // Typed non-person notes lost the inline graph entirely: the sidebar mini
    // graph and the textual Relationships section cover them.
    let repo = TestRepo::new();
    repo.create_markdown(
        "gandalf.md",
        "---\ntype: character\ntitle: Gandalf\n---\n\n# Gandalf\n\nA wizard.\n",
    );

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/gandalf/").await;
    assert_html_not_contains(&html, "<mbr-genealogy");
    assert_html_not_contains(&html, "<mbr-relationships");
}

#[tokio::test]
async fn test_person_page_prefetches_genealogy_chunk() {
    let server = TestServer::start_at_path(genealogy_fixture_path()).await;

    // Person pages idle-prefetch the genealogy chunk so the chart opens fast.
    let html = server.get_text("/people/john/").await;
    assert_html_contains(&html, r#"rel="prefetch""#);
    assert_html_contains(&html, "components/mbr-genealogy.min.js");

    // Non-person pages (the fixture index has no `type`) must not reference
    // the chunk at all.
    let html = server.get_text("/").await;
    assert_html_not_contains(&html, "components/mbr-genealogy.min.js");
}

// ============================================================================
// Body wikilink global resolution (Obsidian-style: current folder first, else
// first match anywhere)
// ============================================================================

#[tokio::test]
async fn test_body_wikilink_resolves_globally_across_folders() {
    let repo = TestRepo::new();
    // Target file lives in one folder...
    repo.create_markdown("Walsh/Patrick Walsh.md", "# Patrick Walsh\n\nBio.");
    // ...and the referencing page is in a *different* folder. `[[Patrick Walsh]]`
    // is not a sibling, so it must resolve to the file's absolute URL. The
    // missing wikilink must stay unresolved (404).
    repo.create_markdown(
        "Notes/family.md",
        "# Family\n\nSee [[Patrick Walsh]] and [[Totally Missing]].",
    );

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    // The rendered page links to the target's absolute URL (space percent-
    // encoded by the HTML href escaper).
    let html = server.get_text("/Notes/family/").await;
    assert!(
        html.contains("/Walsh/Patrick%20Walsh/") || html.contains("/Walsh/Patrick Walsh/"),
        "expected a global link to the Patrick Walsh page, got:\n{html}"
    );

    // The globally-resolved target actually serves.
    assert_eq!(
        server.get("/Walsh/Patrick%20Walsh/").await.status(),
        200,
        "globally-resolved wikilink target should serve 200"
    );

    // The genuinely-missing wikilink still 404s (nothing matches anywhere).
    assert_eq!(
        server.get("/Notes/Totally%20Missing/").await.status(),
        404,
        "missing wikilink target must still 404"
    );

    // errors.json: NO broken link for the resolved wikilink, but the missing
    // one IS reported broken.
    let json: serde_json::Value = server
        .get("/Notes/family/errors.json")
        .await
        .json()
        .await
        .unwrap();
    let errors = json["errors"].as_array().unwrap();
    assert!(
        !errors.iter().any(|e| e["type"] == "broken_internal_link"
            && e["target"].as_str().unwrap_or("").contains("Patrick")),
        "resolved [[Patrick Walsh]] must NOT be reported broken: {errors:?}"
    );
    assert!(
        errors.iter().any(|e| e["type"] == "broken_internal_link"
            && e["target"]
                .as_str()
                .unwrap_or("")
                .contains("Totally Missing")),
        "unresolved [[Totally Missing]] must be reported broken: {errors:?}"
    );
}

// ============================================================================
// Cached site.json + memoized directory listings
// ============================================================================

/// Tera escapes `/` as `&#x2F;` inside attribute values; normalize the markup
/// so listing tests can assert on plain URLs.
fn with_plain_slashes(html: &str) -> String {
    html.replace("&#x2F;", "/")
}

/// The site.json body is cached, so consecutive requests must return the exact
/// same bytes (and still carry the whole markdown index).
#[tokio::test]
async fn test_site_json_body_is_byte_stable_across_requests() {
    let repo = TestRepo::new();
    repo.create_markdown("alpha.md", "---\ntitle: Alpha\n---\n\nA");
    repo.create_markdown("docs/beta.md", "---\ntitle: Beta\n---\n\nB");
    repo.create_static_file("images/photo.jpg", b"fake jpg data");

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    let first = server.get_text("/.mbr/site.json").await;
    let second = server.get_text("/.mbr/site.json").await;
    assert_eq!(
        first, second,
        "consecutive site.json responses must be identical"
    );

    let parsed: serde_json::Value = serde_json::from_str(&first).expect("site.json is valid JSON");
    assert!(
        parsed["other_files"].is_null(),
        "site.json must not carry the media catalog"
    );
    let files = parsed["markdown_files"]
        .as_array()
        .expect("markdown_files array");
    assert_eq!(files.len(), 2, "both markdown files must be indexed");
    assert!(parsed["sort"].is_array(), "sort config must be present");
    assert_eq!(parsed["sidebar_style"], "panel");

    // Payload shape: exactly the keys the frontend consumes, no media catalog.
    let mut keys: Vec<&str> = parsed
        .as_object()
        .expect("site.json is an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "index_file",
            "markdown_files",
            "relationship_types",
            "sidebar_max_items",
            "sidebar_style",
            "sort",
        ]
    );
    // Relationship data is injected per entry.
    assert!(files.iter().all(|f| f["relationships"].is_array()));
}

/// Creating a file must invalidate the cached site.json body — a stale cache
/// would hide the new page from every navigation component until restart.
#[tokio::test]
async fn test_site_json_cache_invalidated_when_file_created() {
    let repo = TestRepo::new();
    repo.create_markdown("alpha.md", "---\ntitle: Alpha\n---\n\nA");
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    let before: serde_json::Value = serde_json::from_str(&server.get_text("/.mbr/site.json").await)
        .expect("site.json is valid JSON");
    assert_eq!(before["markdown_files"].as_array().unwrap().len(), 1);

    let resp = edit_post(
        &server,
        "/.mbr/create/gamma.md",
        serde_json::json!({ "content": "---\ntitle: Gamma\n---\n\nG" }),
    )
    .await;
    assert_eq!(resp.status(), 200);

    let after: serde_json::Value = serde_json::from_str(&server.get_text("/.mbr/site.json").await)
        .expect("site.json is valid JSON");
    let urls: Vec<&str> = after["markdown_files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["url_path"].as_str())
        .collect();
    assert!(
        urls.contains(&"/gamma/"),
        "site.json must include the newly created page: {urls:?}"
    );
}

/// An edit made outside the server (an external editor, `git pull`) must also
/// drop the cached site.json body, via the watcher's debounced invalidation.
#[tokio::test]
async fn test_site_json_cache_invalidated_by_watcher_edit() {
    let repo = TestRepo::new();
    let note = repo.create_markdown("alpha.md", "---\ntitle: Alpha\n---\n\nA");

    let server = TestServer::start(&repo).await;
    // The watcher is initialized on a background thread; an edit that lands
    // before it is listening is simply never seen.
    tokio::time::sleep(Duration::from_millis(750)).await;

    // Populate the cache with the pre-edit title.
    let before: serde_json::Value = serde_json::from_str(&server.get_text("/.mbr/site.json").await)
        .expect("site.json is valid JSON");
    assert_eq!(before["markdown_files"][0]["frontmatter"]["title"], "Alpha");

    let edited = "---\ntitle: Renamed\n---\n\nA";
    std::fs::write(&note, edited).expect("rewrite note");

    // Watcher event + 2 s debounce; poll rather than sleeping blind, re-touching
    // the file so a dropped first event cannot hang the test.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let body: serde_json::Value =
            serde_json::from_str(&server.get_text("/.mbr/site.json").await)
                .expect("site.json is valid JSON");
        if body["markdown_files"][0]["frontmatter"]["title"] == "Renamed" {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "site.json still served the pre-edit body: {:?}",
            body["markdown_files"]
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
        std::fs::write(&note, edited).expect("re-touch note");
    }
}

/// A directory listing served from the in-memory index must show every direct
/// child file and every immediate subdirectory, including one that holds only
/// assets.
#[tokio::test]
async fn test_directory_listing_from_index_lists_files_and_subdirs() {
    let repo = TestRepo::new();
    repo.create_markdown("docs/guide.md", "---\ntitle: Guide\n---\n\nG");
    repo.create_markdown("docs/deep/inner.md", "---\ntitle: Inner\n---\n\nI");
    repo.create_static_file("docs/media/photo.jpg", b"fake jpg data");

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    let html = with_plain_slashes(&server.get_text("/docs/").await);
    assert_html_contains(&html, "href=\"/docs/guide/\"");
    assert_html_contains(&html, "href=\"/docs/deep/\"");
    assert_html_contains(&html, "href=\"/docs/media/\"");
    // The nested page is not a direct child, so it must not be listed as a file.
    assert_html_not_contains(&html, "href=\"/docs/deep/inner/\"");
}

/// The memoized listing must be dropped when a file is created, otherwise the
/// directory keeps serving its pre-create snapshot.
#[tokio::test]
async fn test_directory_listing_updates_after_file_created() {
    let repo = TestRepo::new();
    repo.create_markdown("docs/guide.md", "---\ntitle: Guide\n---\n\nG");
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    // Populate the cache.
    let before = with_plain_slashes(&server.get_text("/docs/").await);
    assert_html_not_contains(&before, "href=\"/docs/extra/\"");

    let resp = edit_post(
        &server,
        "/.mbr/create/docs/extra.md",
        serde_json::json!({ "content": "---\ntitle: Extra\n---\n\nE" }),
    )
    .await;
    assert_eq!(resp.status(), 200);

    let after = with_plain_slashes(&server.get_text("/docs/").await);
    assert_html_contains(&after, "href=\"/docs/extra/\"");
    assert_html_contains(&after, "href=\"/docs/guide/\"");
}

/// Proves the listing is served from the in-memory index instead of a live
/// per-request disk scan: a file written straight to disk (bypassing the
/// editing endpoints) is not visible until the watcher's 2 s debounce window
/// closes and drops the listing caches. Listings are therefore eventually
/// consistent with disk, like every other derived cache in this server.
#[tokio::test]
async fn test_directory_listing_served_from_index_not_rescanned() {
    let repo = TestRepo::new();
    repo.create_markdown("docs/guide.md", "---\ntitle: Guide\n---\n\nG");

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    // Populate the memoized listing.
    let before = with_plain_slashes(&server.get_text("/docs/").await);
    assert_html_contains(&before, "href=\"/docs/guide/\"");

    // Write directly to disk; nothing tells the repo index about it yet.
    repo.create_markdown("docs/sneaky.md", "---\ntitle: Sneaky\n---\n\nS");

    let after = with_plain_slashes(&server.get_text("/docs/").await);
    assert_html_not_contains(&after, "href=\"/docs/sneaky/\"");
}

/// The listing must still work before the background scan finishes: that path
/// falls back to a live disk scan because the in-memory index is incomplete.
/// Deliberately does not call `wait_for_scan()`.
#[tokio::test]
async fn test_directory_listing_before_scan_completes() {
    let repo = TestRepo::new();
    for i in 0..400 {
        repo.create_markdown(
            &format!("bulk/note-{i:04}.md"),
            &format!("---\ntitle: Note {i}\n---\n\nBody"),
        );
    }
    repo.create_dir("bulk/nested");
    repo.create_markdown("bulk/nested/inner.md", "---\ntitle: Inner\n---\n\nI");

    let server = TestServer::start(&repo).await;

    let html = with_plain_slashes(&server.get_text("/bulk/").await);
    assert_html_contains(&html, "href=\"/bulk/note-0000/\"");
    assert_html_contains(&html, "href=\"/bulk/nested/\"");
}

// ============================================================================
// Performance measurement harnesses (ignored by default)
//
// These are not assertions about wall-clock time (CI machines vary); they
// exist so the directory-listing and site.json costs can be measured on
// demand with `cargo test --release --test server_integration -- --ignored
// --nocapture perf_`.
// ============================================================================

/// Measures repeated directory-listing latency on a 5,000-file directory.
#[tokio::test]
#[ignore = "performance measurement; run manually with --release --ignored"]
async fn perf_directory_listing_latency() {
    let repo = TestRepo::new();
    for i in 0..5000 {
        repo.create_markdown(
            &format!("bulk/note-{i:05}.md"),
            &format!(
                "---\ntitle: Note {i}\ntags: [alpha, beta]\n---\n\n# Note {i}\n\nBody text.\n"
            ),
        );
    }

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    for round in 0..5 {
        let start = std::time::Instant::now();
        let body = server.get_text("/bulk/").await;
        println!(
            "perf directory listing round {round}: {:?} ({} bytes)",
            start.elapsed(),
            body.len()
        );
    }
}

/// Measures repeated `/.mbr/site.json` latency on a large mixed repo.
#[tokio::test]
#[ignore = "performance measurement; run manually with --release --ignored"]
async fn perf_site_json_latency() {
    let repo = TestRepo::new();
    for i in 0..5000 {
        repo.create_markdown(
            &format!("notes/dir-{}/note-{i:05}.md", i % 50),
            &format!("---\ntitle: Note {i}\ntags: [alpha, beta]\n---\n\n# Note {i}\n\nBody.\n"),
        );
    }
    for i in 0..10000 {
        repo.create_static_file(
            &format!("assets/dir-{}/img-{i:05}.png", i % 50),
            b"not-a-png",
        );
    }

    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    for round in 0..5 {
        let start = std::time::Instant::now();
        let body = server.get_text("/.mbr/site.json").await;
        println!(
            "perf site.json round {round}: {:?} ({} bytes)",
            start.elapsed(),
            body.len()
        );
    }

    // Cold rounds: creating a file drops the cached body, so each request pays
    // the full rebuild (this is the cost the `SiteJson` view struct reduces).
    for round in 0..3 {
        let created = edit_post(
            &server,
            &format!("/.mbr/create/perf-{round}.md"),
            serde_json::json!({ "content": "# Perf" }),
        )
        .await;
        assert_eq!(created.status(), 200);

        let start = std::time::Instant::now();
        let body = server.get_text("/.mbr/site.json").await;
        println!(
            "perf site.json rebuild round {round}: {:?} ({} bytes)",
            start.elapsed(),
            body.len()
        );
    }
}

// ============================================================================
// Task browser (`POST /.mbr/tasks`)
// ============================================================================

/// A repository with tasks spread across folders, statuses, priorities, tags
/// and due dates. Due dates are absolute, so tests that depend on "today" pin
/// the bucket they assert on rather than relying on the wall clock.
fn task_repo() -> TestRepo {
    let repo = TestRepo::new();
    repo.create_markdown(
        "inbox.md",
        concat!(
            "# Inbox\n\n",
            "- [ ] write the report #work !!\n",
            "- [x] file the receipts\n",
            "- [-] abandoned idea\n",
        ),
    );
    repo.create_markdown(
        "docs/plan.md",
        concat!(
            "---\ntitle: The Plan\n---\n\n",
            "- [ ] draft the outline #work\n",
            "- [ ] review with Bob\n",
        ),
    );
    repo.create_markdown(
        "docs/notes/weekly.md",
        "- [ ] weekly retro #team @due(2026-08-05)\n",
    );
    repo.create_markdown("prose.md", "# Just prose\n\nNo tasks here at all.\n");
    repo
}

/// POSTs a task query and returns the parsed JSON body.
async fn tasks_query(server: &TestServer, body: &str) -> serde_json::Value {
    let response = server.post_json("/.mbr/tasks", body).await;
    assert_eq!(response.status(), 200, "task query should succeed");
    response.json().await.expect("task response is JSON")
}

/// Every returned task's display text, in response order.
fn task_texts(body: &serde_json::Value) -> Vec<String> {
    body["groups"]
        .as_array()
        .expect("groups array")
        .iter()
        .flat_map(|g| g["tasks"].as_array().expect("tasks array"))
        .map(|t| t["text"].as_str().expect("text").to_string())
        .collect()
}

#[tokio::test]
async fn test_tasks_endpoint_returns_404_when_disabled() {
    let repo = task_repo();
    let server = TestServer::start_with_config_fn(&repo, |c| {
        c.tasks_enabled = false;
    })
    .await;

    let response = server.post_json("/.mbr/tasks", "{}").await;
    assert_eq!(
        response.status(),
        404,
        "a disabled task browser must not answer queries"
    );
}

#[tokio::test]
async fn test_tasks_endpoint_returns_grouped_incomplete_tasks_by_default() {
    let repo = task_repo();
    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    let body = tasks_query(&server, "{}").await;

    // Default view: incomplete only, grouped by file, files sorted by URL.
    let groups = body["groups"].as_array().expect("groups array");
    assert_eq!(
        groups
            .iter()
            .map(|g| g["key"].as_str().expect("key"))
            .collect::<Vec<_>>(),
        vec!["/docs/notes/weekly/", "/docs/plan/", "/inbox/"],
        "a file with no tasks must not produce a group"
    );

    // Group metadata: frontmatter title, folder sublabel, page URL.
    let plan = &groups[1];
    assert_eq!(plan["label"], "The Plan");
    assert_eq!(plan["sublabel"], "docs");
    assert_eq!(plan["url_path"], "/docs/plan/");
    assert!(plan["date"].is_null(), "category groups carry no date");

    // Parsed annotations survive the round trip.
    let report = &groups[2]["tasks"][0];
    assert_eq!(report["text"], "write the report");
    assert_eq!(report["status"], "open");
    assert_eq!(report["priority"], "high");
    assert_eq!(report["tags"][0], "work");
    assert_eq!(report["url_path"], "/inbox/");
    assert_eq!(report["path"], "inbox.md");
    assert_eq!(report["line"], 3);

    assert_eq!(body["total_matches"], 4);
    assert!(body["duration_ms"].is_number());
    assert_eq!(body["scan_in_progress"], false);
}

#[tokio::test]
async fn test_tasks_endpoint_counts_include_tasks_filtered_out_of_the_view() {
    let repo = task_repo();
    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    // Only the open task in inbox.md matches, but the group's progress must
    // describe the whole file — and exclude the canceled item from both halves.
    let body = tasks_query(&server, r#"{"q": "report"}"#).await;
    let groups = body["groups"].as_array().expect("groups array");
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["tasks"].as_array().expect("tasks").len(), 1);
    assert_eq!(groups[0]["done"], 1);
    assert_eq!(groups[0]["total"], 2);
}

#[tokio::test]
async fn test_tasks_endpoint_status_filter_is_a_multi_select() {
    let repo = task_repo();
    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    let done_only = tasks_query(&server, r#"{"statuses": ["done"]}"#).await;
    assert_eq!(task_texts(&done_only), vec!["file the receipts"]);

    let canceled = tasks_query(&server, r#"{"statuses": ["canceled"]}"#).await;
    assert_eq!(task_texts(&canceled), vec!["abandoned idea"]);

    let everything = tasks_query(
        &server,
        r#"{"statuses": ["open", "done", "canceled"], "folder": "/"}"#,
    )
    .await;
    assert_eq!(everything["total_matches"], 6);
}

#[tokio::test]
async fn test_tasks_endpoint_folder_scope_includes_subfolders() {
    let repo = task_repo();
    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    let body = tasks_query(&server, r#"{"folder": "/docs/"}"#).await;
    assert_eq!(
        task_texts(&body),
        vec!["weekly retro", "draft the outline", "review with Bob"],
        "a folder scope must reach into its subfolders"
    );

    let deeper = tasks_query(&server, r#"{"folder": "/docs/notes/"}"#).await;
    assert_eq!(task_texts(&deeper), vec!["weekly retro"]);

    // Facets are computed ignoring the folder filter, so the folder pane can
    // still offer somewhere else to go, and count subfolders cumulatively.
    let facets: Vec<(&str, u64)> = deeper["folders"]
        .as_array()
        .expect("folders array")
        .iter()
        .map(|f| {
            (
                f["path"].as_str().expect("path"),
                f["count"].as_u64().expect("count"),
            )
        })
        .collect();
    assert_eq!(facets, vec![("/", 4), ("/docs/", 3), ("/docs/notes/", 1)]);
}

#[tokio::test]
async fn test_tasks_endpoint_query_matches_text_and_tags() {
    let repo = task_repo();
    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    // A `#tag` token matches tags only.
    let tagged = tasks_query(&server, r##"{"q": "#work"}"##).await;
    assert_eq!(
        task_texts(&tagged),
        vec!["draft the outline", "write the report"]
    );

    // Bare words match the display text, case-insensitively, and AND together.
    let words = tasks_query(&server, r#"{"q": "THE report"}"#).await;
    assert_eq!(task_texts(&words), vec!["write the report"]);

    let none = tasks_query(&server, r##"{"q": "#nonexistent"}"##).await;
    assert!(none["groups"].as_array().expect("groups").is_empty());
    assert_eq!(none["total_matches"], 0);
}

#[tokio::test]
async fn test_tasks_endpoint_limit_truncates_without_changing_total_matches() {
    let repo = task_repo();
    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    let body = tasks_query(&server, r#"{"limit": 2}"#).await;
    assert_eq!(task_texts(&body).len(), 2, "limit caps returned tasks");
    assert_eq!(
        body["total_matches"], 4,
        "total_matches is counted before truncation"
    );
}

#[tokio::test]
async fn test_tasks_endpoint_calendar_mode_buckets_by_due_date() {
    let repo = TestRepo::new();
    // Pinned far in the past and future so the buckets are stable whenever the
    // suite runs; "today"/"tomorrow" are covered by the unit tests, which can
    // supply their own `today`.
    repo.create_markdown(
        "due.md",
        concat!(
            "- [ ] ancient @due(2001-01-01)\n",
            "- [ ] distant @due(2999-12-31)\n",
            "- [x] distant done @due(2999-12-31)\n",
            "- [-] canceled @due(2999-12-31)\n",
            "- [ ] undated\n",
        ),
    );
    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    let body = tasks_query(&server, r#"{"mode": "calendar"}"#).await;
    let groups = body["groups"].as_array().expect("groups array");
    assert_eq!(
        groups
            .iter()
            .map(|g| g["key"].as_str().expect("key"))
            .collect::<Vec<_>>(),
        vec!["overdue", "upcoming:2999-12-31", "none"]
    );

    // Overdue carries no progress numbers at all.
    assert_eq!(
        (groups[0]["done"].clone(), groups[0]["total"].clone()),
        (serde_json::json!(0), serde_json::json!(0))
    );

    // The dated bucket shows only the open task (default status filter) but
    // counts the completed one too — and ignores the canceled one entirely.
    let dated = &groups[1];
    assert_eq!(
        dated["tasks"]
            .as_array()
            .expect("tasks")
            .iter()
            .map(|t| t["text"].as_str().expect("text"))
            .collect::<Vec<_>>(),
        vec!["distant"]
    );
    assert_eq!(dated["done"], 1);
    assert_eq!(dated["total"], 2);
    assert_eq!(dated["date"], "2999-12-31");
    assert!(dated["url_path"].is_null());
}

/// The watcher must keep the task index fresh once it has been built, the same
/// way it keeps links.json fresh (see
/// `test_links_json_refreshes_after_watcher_sees_external_edit`).
#[tokio::test]
async fn test_tasks_refresh_after_watcher_sees_external_edit() {
    let repo = TestRepo::new();
    let notes = repo.create_markdown("notes.md", "- [ ] original task\n");

    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;
    // The watcher is initialized on a background thread; an edit that lands
    // before it is listening is simply never seen.
    tokio::time::sleep(Duration::from_millis(750)).await;

    // Build the index by querying it once — nothing is indexed before this.
    let before = tasks_query(&server, "{}").await;
    assert_eq!(task_texts(&before), vec!["original task"]);

    let edited = "- [ ] original task\n- [ ] added externally\n";
    std::fs::write(&notes, edited).expect("rewrite notes");

    // Watcher event + 2 s debounce; poll rather than sleeping blind, re-touching
    // the file so a dropped first event cannot hang the test.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let body = tasks_query(&server, "{}").await;
        if task_texts(&body) == vec!["original task", "added externally"] {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "task index still served the stale file: {:?}",
            task_texts(&body)
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
        std::fs::write(&notes, edited).expect("re-touch notes");
    }
}

// ============================================================================
// Task toggle (`POST /.mbr/task`)
// ============================================================================

/// A file whose second task is the one every toggle test aims at, with a
/// neighbour above and below so a patch that moves other bytes is caught.
const TOGGLE_SOURCE: &str = "# Notes\n\n- [ ] write the report !!\n- [ ] second\n";

/// The body of a toggle request for line 3 of [`TOGGLE_SOURCE`].
fn toggle_body(to: &str) -> serde_json::Value {
    serde_json::json!({
        "path": "notes.md",
        "line": 3,
        "expected": "- [ ] write the report !!",
        "to": to,
    })
}

#[tokio::test]
async fn test_task_toggle_disabled_returns_403() {
    let repo = TestRepo::new();
    let file = repo.create_markdown("notes.md", TOGGLE_SOURCE);
    // Editing off (the default) — the task browser being on must not matter.
    let server = TestServer::start(&repo).await;
    server.wait_for_scan().await;

    let resp = edit_post(&server, "/.mbr/task", toggle_body("done")).await;
    assert_eq!(
        resp.status(),
        403,
        "toggling must be 403 when editing is off"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        TOGGLE_SOURCE,
        "a rejected toggle must not touch the file"
    );
}

#[tokio::test]
async fn test_task_toggle_missing_csrf_header_returns_403() {
    let repo = TestRepo::new();
    let file = repo.create_markdown("notes.md", TOGGLE_SOURCE);
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    let resp = server
        .client
        .post(server.url("/.mbr/task"))
        .header("Content-Type", "application/json")
        .body(toggle_body("done").to_string())
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 403, "missing X-MBR-Edit must be 403");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), TOGGLE_SOURCE);
}

#[tokio::test]
async fn test_task_toggle_happy_path_stamps_and_writes_exact_bytes() {
    let repo = TestRepo::new();
    let file = repo.create_markdown("notes.md", TOGGLE_SOURCE);
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    let resp = edit_post(&server, "/.mbr/task", toggle_body("done")).await;
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.expect("JSON response");
    assert_eq!(json["line"], 3);
    let text = json["text"].as_str().expect("text").to_string();

    // The stamp is wall-clock, so assert its shape by reading it back with the
    // same parser the index uses rather than by pinning a literal timestamp.
    assert!(
        text.starts_with("- [x] write the report !! @done("),
        "unexpected line: {text}"
    );
    let parsed = mbr::tasks::parse_task_line(&text, 3).expect("still a task");
    assert_eq!(parsed.status, mbr::tasks::TaskStatus::Done);
    assert_eq!(parsed.text, "write the report");
    assert!(parsed.done.is_some() && parsed.done_has_time);

    // Exactly that line changed; every other byte of the file is where it was.
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        format!("# Notes\n\n{text}\n- [ ] second\n")
    );

    // Reopening it removes the stamp again, restoring the original file.
    let reopen = edit_post(
        &server,
        "/.mbr/task",
        serde_json::json!({
            "path": "notes.md", "line": 3, "expected": text, "to": "open",
        }),
    )
    .await;
    assert_eq!(reopen.status(), 200);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), TOGGLE_SOURCE);
}

#[tokio::test]
async fn test_task_toggle_without_stamping_rewrites_only_the_marker() {
    let repo = TestRepo::new();
    let file = repo.create_markdown("notes.md", TOGGLE_SOURCE);
    let server = TestServer::start_with_config_fn(&repo, |config| {
        config.edit_enabled = true;
        config.tasks_stamp_done = false;
    })
    .await;
    server.wait_for_scan().await;

    let resp = edit_post(&server, "/.mbr/task", toggle_body("done")).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "# Notes\n\n- [x] write the report !!\n- [ ] second\n"
    );
}

#[tokio::test]
async fn test_task_toggle_preserves_crlf_line_endings() {
    let repo = TestRepo::new();
    let file = repo.create_markdown("notes.md", "- [ ] windows task\r\n- [ ] second\r\n");
    let server = TestServer::start_with_config_fn(&repo, |config| {
        config.edit_enabled = true;
        config.tasks_stamp_done = false;
    })
    .await;
    server.wait_for_scan().await;

    let resp = edit_post(
        &server,
        "/.mbr/task",
        serde_json::json!({
            "path": "notes.md", "line": 1,
            "expected": "- [ ] windows task", "to": "canceled",
        }),
    )
    .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "- [-] windows task\r\n- [ ] second\r\n",
        "a CRLF file must stay a CRLF file"
    );
}

#[tokio::test]
async fn test_task_toggle_stale_expected_returns_409() {
    let repo = TestRepo::new();
    let file = repo.create_markdown("notes.md", TOGGLE_SOURCE);
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    // Somebody edited that very line since the page was rendered.
    let edited = "# Notes\n\n- [ ] write the report tomorrow !!\n- [ ] second\n";
    std::fs::write(&file, edited).expect("rewrite");

    let resp = edit_post(&server, "/.mbr/task", toggle_body("done")).await;
    assert_eq!(resp.status(), 409, "a changed line must be 409");
    let message = resp.text().await.expect("body");
    assert!(
        message.contains("changed on disk"),
        "the 409 should say why: {message}"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        edited,
        "a rejected toggle must not touch the file"
    );
}

#[tokio::test]
async fn test_task_toggle_rejects_a_line_that_is_not_a_task() {
    let repo = TestRepo::new();
    let file = repo.create_markdown("notes.md", TOGGLE_SOURCE);
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    // Line 1 is the heading, and matches `expected` exactly — it is simply not
    // a task, which is a different failure from a stale line.
    let resp = edit_post(
        &server,
        "/.mbr/task",
        serde_json::json!({
            "path": "notes.md", "line": 1, "expected": "# Notes", "to": "done",
        }),
    )
    .await;
    assert_eq!(resp.status(), 400, "a non-task line must be 400");

    // As is a line past the end of the file.
    let past_end = edit_post(
        &server,
        "/.mbr/task",
        serde_json::json!({
            "path": "notes.md", "line": 99, "expected": "", "to": "done",
        }),
    )
    .await;
    assert_eq!(past_end.status(), 400, "a missing line must be 400");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), TOGGLE_SOURCE);
}

#[tokio::test]
async fn test_task_toggle_rejects_paths_outside_the_root() {
    let repo = TestRepo::new();
    repo.create_markdown("notes.md", TOGGLE_SOURCE);
    repo.create_static_file("data.txt", b"- [ ] not markdown\n");
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    for path in [
        "../escape.md",
        "../../etc/passwd",
        "/etc/passwd",
        "missing.md",
        "data.txt",
    ] {
        let resp = edit_post(
            &server,
            "/.mbr/task",
            serde_json::json!({
                "path": path, "line": 1, "expected": "- [ ] x", "to": "done",
            }),
        )
        .await;
        assert!(
            resp.status() == 404 || resp.status() == 400,
            "toggling {path} must be rejected, got {}",
            resp.status()
        );
    }
}

/// The panel that sent a toggle re-queries immediately, long before the
/// watcher's debounce elapses, so the handler has to invalidate the index
/// itself.
#[tokio::test]
async fn test_task_toggle_is_reflected_by_the_task_index() {
    let repo = TestRepo::new();
    repo.create_markdown("notes.md", "- [ ] toggle me\n- [ ] leave me\n");
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    // Build the index first: this is the "already built" path, where a stale
    // entry would otherwise survive.
    let before = tasks_query(&server, "{}").await;
    assert_eq!(task_texts(&before), vec!["toggle me", "leave me"]);

    let resp = edit_post(
        &server,
        "/.mbr/task",
        serde_json::json!({
            "path": "notes.md", "line": 1,
            "expected": "- [ ] toggle me", "to": "done",
        }),
    )
    .await;
    assert_eq!(resp.status(), 200);

    // Default view is incomplete-only, so the completed task drops out at once.
    let after = tasks_query(&server, "{}").await;
    assert_eq!(task_texts(&after), vec!["leave me"]);

    // ...and it really is done, stamp and all, in the everything view.
    let all = tasks_query(&server, r#"{"statuses": ["open", "done", "canceled"]}"#).await;
    let toggled = &all["groups"][0]["tasks"][0];
    assert_eq!(toggled["text"], "toggle me");
    assert_eq!(toggled["status"], "done");
    assert!(
        !toggled["done"].is_null(),
        "the @done stamp should be indexed: {toggled}"
    );
    assert_eq!(all["groups"][0]["done"], 1);
    assert_eq!(all["groups"][0]["total"], 2);
}

#[tokio::test]
async fn test_task_query_path_round_trips_into_a_toggle() {
    // `docs/index.md` is served at `/docs/`, so a client that rebuilt the file
    // path out of `url_path` would send `docs.md` and get a 404. This is the
    // pairing that makes `TaskHit::path` worth putting on the wire.
    let repo = TestRepo::new();
    let file = repo.create_markdown("docs/index.md", "- [ ] indexed task\n");
    let server = TestServer::start_with_config_fn(&repo, enable_editing).await;
    server.wait_for_scan().await;

    let body = tasks_query(&server, "{}").await;
    let hit = &body["groups"][0]["tasks"][0];
    assert_eq!(hit["url_path"], "/docs/", "the URL hides the file name");
    assert_eq!(hit["path"], "docs/index.md");

    let resp = edit_post(
        &server,
        "/.mbr/task",
        serde_json::json!({
            "path": hit["path"],
            "line": hit["line"],
            "expected": "- [ ] indexed task",
            "to": "done",
        }),
    )
    .await;
    assert_eq!(
        resp.status(),
        200,
        "the path a query reports must be the one the toggle accepts"
    );
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .starts_with("- [x] indexed task"),
        "the toggle should have landed on the indexed file"
    );
}

#[tokio::test]
async fn test_head_config_includes_tasks_enabled() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello\n\nBody.");

    // Markdown pages, section pages and the home page all need the flag, since
    // the task panel is reachable from every one of them.
    let server = TestServer::start(&repo).await;
    for path in ["/readme/", "/"] {
        assert_html_contains(&server.get_text(path).await, "tasksEnabled: true");
    }

    let server = TestServer::start_with_config_fn(&repo, |c| {
        c.tasks_enabled = false;
    })
    .await;
    for path in ["/readme/", "/"] {
        assert_html_contains(&server.get_text(path).await, "tasksEnabled: false");
    }
}
