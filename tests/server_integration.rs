//! Integration tests for the mbr server.

mod common;

use common::{TestRepo, assert_html_contains, find_available_port};
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
        template_folder: None,
        sort: mbr::config::default_sort_config(),
        gui_mode: false,
        theme: "default".to_string(),
        log_filter: None,
        link_tracking: true,
        tag_sources: mbr::config::default_tag_sources(),
        sidebar_style: "panel".to_string(),
        sidebar_max_items: 100,
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
            let _ = server.start().await;
        });

        // Give server time to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        let client = reqwest::Client::new();

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
    let client = reqwest::Client::builder()
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
    let client = reqwest::Client::builder()
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

// ============================================================================
// Search endpoint tests
// ============================================================================

#[tokio::test]
async fn test_search_endpoint_returns_json() {
    let repo = TestRepo::new();
    repo.create_markdown("test.md", "# Test Page\n\nSome searchable content.");

    let server = TestServer::start(&repo).await;
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

            let _ = server.start().await;
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let client = reqwest::Client::new();

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
async fn test_components_js_bundle_no_missing_imports() {
    // This test verifies that the JS bundle doesn't try to dynamically import
    // files that aren't served, which would cause 404 errors in the browser
    let repo = TestRepo::new();

    let server = TestServer::start(&repo).await;
    let js_content = server
        .get_text("/.mbr/components/mbr-components.min.js")
        .await;

    // Check that there are no dynamic imports to external chunk files
    // These would look like: import("./main-xxx.js") or import("/.mbr/components/main-xxx.js")
    let has_dynamic_import_to_chunk =
        js_content.contains(r#"import("./"#) || js_content.contains(r#"import("/.mbr/components/"#);

    // If there are dynamic imports, they should either:
    // 1. Not exist (everything bundled inline)
    // 2. Or be to files that are served
    if has_dynamic_import_to_chunk {
        // Extract the imported paths and verify they're served
        // For now, just fail with a helpful message
        assert!(
            !has_dynamic_import_to_chunk,
            "Components bundle contains dynamic imports to external chunks. \
             These chunks must either be bundled inline or explicitly served. \
             Check vite.config.ts to disable code splitting."
        );
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

            let _ = server.start().await;
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let client = reqwest::Client::new();

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

            let _ = server.start().await;
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let client = reqwest::Client::new();

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

    // Create a fake sidecar file (PNG with magic bytes)
    let sidecar_path = repo.path().join("docs/test.pdf.cover.png");
    // Create a minimal valid PNG (1x1 pixel, red)
    let png_data: Vec<u8> = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1x1 pixels
        0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, // 8-bit RGB
        0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, // IDAT chunk
        0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, // Compressed data
        0x00, 0x00, 0x03, 0x00, 0x01, 0x00, 0x18, 0xDD, //
        0x8D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, // IEND chunk
        0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    std::fs::write(&sidecar_path, &png_data).unwrap();

    // Wait a bit to ensure mtime difference
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Touch the sidecar to ensure it's newer than the PDF
    let now = std::time::SystemTime::now();
    filetime::set_file_mtime(&sidecar_path, filetime::FileTime::from_system_time(now)).unwrap();

    let server = TestServer::start(&repo).await;
    let response = server.get("/docs/test.pdf.cover.png").await;

    assert_eq!(response.status(), 200);
    assert_eq!(response.headers().get("content-type").unwrap(), "image/png");

    let body = response.bytes().await.unwrap();
    // Should serve our fake sidecar, not the dynamically generated one
    // (our sidecar is tiny, a real generated one would be much larger)
    assert_eq!(
        body.len(),
        png_data.len(),
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

    // Create a fake sidecar file (valid but small PNG)
    let sidecar_path = repo.path().join("docs/test.pdf.cover.png");
    // Create a minimal valid PNG (1x1 pixel)
    let stale_png: Vec<u8> = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1x1 pixels
        0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, // 8-bit RGB
        0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, // IDAT chunk
        0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, // Compressed data
        0x00, 0x00, 0x03, 0x00, 0x01, 0x00, 0x18, 0xDD, //
        0x8D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, // IEND chunk
        0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    std::fs::write(&sidecar_path, &stale_png).unwrap();

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
    let response = server.get("/docs/test.pdf.cover.png").await;

    // Should serve successfully (either regenerated or stale fallback)
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers().get("content-type").unwrap(), "image/png");

    let body = response.bytes().await.unwrap();
    // If pdfium is not available, we fall back to stale sidecar
    // If pdfium is available, we regenerate (larger file)
    // Either way, we should get valid PNG data
    assert!(
        body.len() >= stale_png.len(),
        "Should serve at least the stale sidecar"
    );
    assert_eq!(
        &body[0..8],
        &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
        "Should be valid PNG"
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
    let sidecar_path = repo.path().join("docs/report.pdf.cover.png");
    assert!(!sidecar_path.exists());

    let server = TestServer::start(&repo).await;
    let response = server.get("/docs/report.pdf.cover.png").await;

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
