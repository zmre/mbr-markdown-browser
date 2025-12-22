//! Integration tests for the mbr server.

mod common;

use common::{assert_html_contains, find_available_port, TestRepo};
use std::collections::HashMap;
use std::time::Duration;

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
            let server = mbr::server::Server::init(
                [127, 0, 0, 1],
                port,
                root_dir,
                "static",
                &["md".to_string()],
                &["target".to_string(), "node_modules".to_string()],
                &["*.log".to_string()],
                "index.md",
                100,
            )
            .expect("Failed to initialize server");

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
    assert_html_contains(&html, "<h1>Hello World</h1>");
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

    assert_html_contains(&html, "<h1>Home Page</h1>");
}

#[tokio::test]
async fn test_serve_directory_index() {
    let repo = TestRepo::new();
    repo.create_dir("docs");
    repo.create_markdown("docs/index.md", "# Documentation");

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/docs/").await;

    assert_html_contains(&html, "<h1>Documentation</h1>");
}

#[tokio::test]
async fn test_serve_nested_markdown() {
    let repo = TestRepo::new();
    repo.create_markdown("blog/posts/first.md", "# First Post\n\nContent.");

    let server = TestServer::start(&repo).await;
    let html = server.get_text("/blog/posts/first/").await;

    assert_html_contains(&html, "<h1>First Post</h1>");
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

#[tokio::test]
async fn test_404_for_missing_file() {
    let repo = TestRepo::new();

    let server = TestServer::start(&repo).await;
    let response = server.get("/nonexistent/").await;

    assert_eq!(response.status(), 404);
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
    let response = server.post_json("/.mbr/search", r#"{"q": "Unique Search"}"#).await;

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();

    assert!(body["total_matches"].as_i64().unwrap() >= 1);
    let results = body["results"].as_array().unwrap();
    assert!(!results.is_empty());

    // Check that our file was found
    let found = results.iter().any(|r| r["url_path"].as_str().unwrap().contains("findme"));
    assert!(found, "Expected to find 'findme' in results: {:?}", results);
}

#[tokio::test]
async fn test_search_with_scope_metadata() {
    let repo = TestRepo::new();
    let mut frontmatter = HashMap::new();
    frontmatter.insert("title", "Metadata Only Title");
    repo.create_markdown_with_frontmatter("meta.md", &frontmatter, "Body text without match.");

    let server = TestServer::start(&repo).await;
    let response = server.post_json("/.mbr/search", r#"{"q": "Metadata Only", "scope": "metadata"}"#).await;

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
    let response = server.post_json("/.mbr/search", r#"{"q": "file", "limit": 2}"#).await;

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();

    let results = body["results"].as_array().unwrap();
    assert!(results.len() <= 2, "Expected at most 2 results, got {}", results.len());
}

#[tokio::test]
async fn test_search_includes_duration() {
    let repo = TestRepo::new();
    repo.create_markdown("test.md", "# Test");

    let server = TestServer::start(&repo).await;
    let response = server.post_json("/.mbr/search", r#"{"q": "test"}"#).await;

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();

    assert!(body["duration_ms"].is_number(), "Expected duration_ms in response");
    assert!(body["query"].as_str().unwrap() == "test", "Expected query echo in response");
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
    repo.create_markdown("docs/page.md", "# Page\n\n[Other Section](other.md#section)");

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
