//! Integration tests for the mbr server.

mod common;

use common::{TestRepo, assert_html_contains, find_available_port};
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
                &[
                    ".direnv".to_string(),
                    ".git".to_string(),
                    "result".to_string(),
                    "target".to_string(),
                    "build".to_string(),
                ],
                "index.md",
                100,                                 // oembed_timeout_ms
                2 * 1024 * 1024,                     // oembed_cache_size (2MB)
                None,                                // template_folder
                mbr::config::default_sort_config(),  // sort
                false,                               // gui_mode
                "default",                           // theme
                None,                                // log_filter
                true,                                // link_tracking
                &mbr::config::default_tag_sources(), // tag_sources
                "panel",                             // sidebar_style
                100,                                 // sidebar_max_items
                #[cfg(feature = "media-metadata")]
                false, // transcode_enabled
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
            let server = mbr::server::Server::init(
                [127, 0, 0, 1],
                port,
                root_dir,
                "static",
                &["md".to_string()],
                &["target".to_string(), "node_modules".to_string()],
                &["*.log".to_string()],
                &[
                    ".direnv".to_string(),
                    ".git".to_string(),
                    "result".to_string(),
                    "target".to_string(),
                    "build".to_string(),
                ],
                "index.md",
                100,             // oembed_timeout_ms
                2 * 1024 * 1024, // oembed_cache_size (2MB)
                template_folder,
                mbr::config::default_sort_config(),  // sort
                false,                               // gui_mode
                "default",                           // theme
                None,                                // log_filter
                true,                                // link_tracking
                &mbr::config::default_tag_sources(), // tag_sources
                "panel",                             // sidebar_style
                100,                                 // sidebar_max_items
                #[cfg(feature = "media-metadata")]
                false, // transcode_enabled
            )
            .expect("Failed to initialize server");

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
            let server = mbr::server::Server::init(
                [127, 0, 0, 1],
                port,
                root_dir,
                "static",
                &["md".to_string()],
                &["target".to_string(), "node_modules".to_string()],
                &["*.log".to_string()],
                &[
                    ".direnv".to_string(),
                    ".git".to_string(),
                    "result".to_string(),
                    "target".to_string(),
                    "build".to_string(),
                ],
                "index.md",
                100,
                2 * 1024 * 1024,                     // oembed_cache_size (2MB)
                None,                                // template_folder
                mbr::config::default_sort_config(),  // sort
                false,                               // gui_mode
                &theme,                              // theme
                None,                                // log_filter
                true,                                // link_tracking
                &mbr::config::default_tag_sources(), // tag_sources
                "panel",                             // sidebar_style
                100,                                 // sidebar_max_items
                #[cfg(feature = "media-metadata")]
                false, // transcode_enabled
            )
            .expect("Failed to initialize server");

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
            let server = mbr::server::Server::init(
                [127, 0, 0, 1],
                port,
                root_dir,
                "static",
                &["md".to_string()],
                &["target".to_string(), "node_modules".to_string()],
                &["*.log".to_string()],
                &[
                    ".direnv".to_string(),
                    ".git".to_string(),
                    "result".to_string(),
                    "target".to_string(),
                    "build".to_string(),
                ],
                "index.md",
                100,                                 // oembed_timeout_ms
                2 * 1024 * 1024,                     // oembed_cache_size (2MB)
                None,                                // template_folder
                mbr::config::default_sort_config(),  // sort
                false,                               // gui_mode
                "default",                           // theme
                None,                                // log_filter
                false,                               // link_tracking DISABLED
                &mbr::config::default_tag_sources(), // tag_sources
                "panel",                             // sidebar_style
                100,                                 // sidebar_max_items
                #[cfg(feature = "media-metadata")]
                false, // transcode_enabled
            )
            .expect("Failed to initialize server");

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
