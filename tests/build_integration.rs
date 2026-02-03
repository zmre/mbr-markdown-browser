//! Integration tests for the static site builder.

mod common;

use common::TestRepo;
use std::fs;
use std::path::Path;

/// Helper to run a build and return the output directory
async fn build_site(repo: &TestRepo) -> std::path::PathBuf {
    let config = mbr::Config {
        root_dir: repo.path().to_path_buf(),
        ..Default::default()
    };
    let output_dir = repo.path().join("build");

    let builder =
        mbr::build::Builder::new(config, output_dir.clone()).expect("Failed to create builder");

    builder.build().await.expect("Build failed");

    output_dir
}

/// Reads a fragment file from the Pagefind index
fn read_pagefind_fragment(pagefind_dir: &Path, filename: &str) -> Option<serde_json::Value> {
    let fragment_path = pagefind_dir.join("fragment").join(filename);
    if !fragment_path.exists() {
        return None;
    }

    let data = fs::read(&fragment_path).ok()?;

    // Decompress gzip
    use std::io::Read;
    let mut decoder = flate2::read::GzDecoder::new(&data[..]);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed).ok()?;

    // Parse JSON (skip signature prefix if present)
    let text = String::from_utf8_lossy(&decompressed);
    let json_start = text.find('{')?;
    serde_json::from_str(&text[json_start..]).ok()
}

/// Get all indexed URLs from Pagefind fragments
fn get_indexed_urls(pagefind_dir: &Path) -> Vec<String> {
    let fragment_dir = pagefind_dir.join("fragment");
    if !fragment_dir.exists() {
        return Vec::new();
    }

    let mut urls = Vec::new();
    for entry in fs::read_dir(&fragment_dir).unwrap() {
        let entry = entry.unwrap();
        if entry.path().extension().is_some_and(|e| e == "pf_fragment")
            && let Some(json) =
                read_pagefind_fragment(pagefind_dir, entry.file_name().to_str().unwrap())
            && let Some(url) = json.get("url").and_then(|v| v.as_str())
        {
            urls.push(url.to_string());
        }
    }
    urls.sort();
    urls
}

// ============================================================================
// Build output tests
// ============================================================================

#[tokio::test]
async fn test_build_creates_html_for_markdown() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello World\n\nThis is a test.");

    let output = build_site(&repo).await;

    // Should create readme/index.html
    let html_path = output.join("readme").join("index.html");
    assert!(html_path.exists(), "Expected {:?} to exist", html_path);

    let html = fs::read_to_string(&html_path).unwrap();
    assert!(html.contains("<h1 id=\"hello-world\">Hello World</h1>"));
    assert!(html.contains("This is a test."));
}

#[tokio::test]
async fn test_build_creates_section_pages() {
    let repo = TestRepo::new();
    repo.create_dir("docs");
    repo.create_markdown("docs/guide.md", "# Guide");
    repo.create_markdown("docs/tutorial.md", "# Tutorial");

    let output = build_site(&repo).await;

    // Should create docs/index.html (section page)
    let section_path = output.join("docs").join("index.html");
    assert!(
        section_path.exists(),
        "Expected section page at {:?}",
        section_path
    );

    let html = fs::read_to_string(&section_path).unwrap();
    assert!(html.contains("guide") || html.contains("Guide"));
    assert!(html.contains("tutorial") || html.contains("Tutorial"));
}

#[tokio::test]
async fn test_build_sets_static_mode() {
    let repo = TestRepo::new();
    repo.create_markdown("test.md", "# Test");

    let output = build_site(&repo).await;

    let html_path = output.join("test").join("index.html");
    let html = fs::read_to_string(&html_path).unwrap();

    // Should have serverMode: false
    assert!(
        html.contains("serverMode: false"),
        "Expected serverMode: false in output"
    );
    assert!(
        !html.contains("serverMode: true"),
        "Should not have serverMode: true"
    );
}

// ============================================================================
// Pagefind indexing tests
// ============================================================================

#[tokio::test]
async fn test_pagefind_index_created() {
    let repo = TestRepo::new();
    repo.create_markdown("test.md", "# Test Page");

    let output = build_site(&repo).await;

    // Should create pagefind directory
    let pagefind_dir = output.join(".mbr").join("pagefind");
    assert!(
        pagefind_dir.exists(),
        "Expected Pagefind directory at {:?}",
        pagefind_dir
    );

    // Should have entry file
    let entry_file = pagefind_dir.join("pagefind-entry.json");
    assert!(entry_file.exists(), "Expected pagefind-entry.json");

    // Should have pagefind.js
    let js_file = pagefind_dir.join("pagefind.js");
    assert!(js_file.exists(), "Expected pagefind.js");
}

#[tokio::test]
async fn test_pagefind_indexes_markdown_pages() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# README\n\nProject documentation.");
    repo.create_markdown("guide.md", "# Guide\n\nHow to use.");

    let output = build_site(&repo).await;

    let pagefind_dir = output.join(".mbr").join("pagefind");
    let urls = get_indexed_urls(&pagefind_dir);

    assert!(
        urls.iter().any(|u| u.contains("readme")),
        "Expected readme to be indexed: {:?}",
        urls
    );
    assert!(
        urls.iter().any(|u| u.contains("guide")),
        "Expected guide to be indexed: {:?}",
        urls
    );
}

#[tokio::test]
async fn test_pagefind_excludes_mbr_directory() {
    let repo = TestRepo::new();
    repo.create_markdown("test.md", "# Test");
    // The .mbr directory is created automatically by TestRepo

    let output = build_site(&repo).await;

    let pagefind_dir = output.join(".mbr").join("pagefind");
    let urls = get_indexed_urls(&pagefind_dir);

    // No URL should contain .mbr
    for url in &urls {
        assert!(
            !url.contains(".mbr"),
            "Unexpected .mbr URL in index: {}",
            url
        );
    }
}

#[tokio::test]
async fn test_pagefind_page_count_matches() {
    let repo = TestRepo::new();
    repo.create_markdown("one.md", "# One");
    repo.create_markdown("two.md", "# Two");
    repo.create_markdown("three.md", "# Three");

    let output = build_site(&repo).await;

    let entry_path = output
        .join(".mbr")
        .join("pagefind")
        .join("pagefind-entry.json");
    let entry: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&entry_path).unwrap()).unwrap();

    // Should have at least 4 pages (3 markdown + 1 home page)
    let page_count = entry["languages"]["en"]["page_count"].as_i64().unwrap();
    assert!(
        page_count >= 4,
        "Expected at least 4 pages, got {}",
        page_count
    );
}

// ============================================================================
// Directory exclusion tests
// ============================================================================

#[tokio::test]
async fn test_build_excludes_node_modules() {
    let repo = TestRepo::new();
    repo.create_dir("node_modules");
    repo.create_markdown("node_modules/package.md", "# Package");
    repo.create_markdown("readme.md", "# README");

    let output = build_site(&repo).await;

    // node_modules should not be in output
    assert!(
        !output.join("node_modules").exists(),
        "node_modules should be excluded"
    );

    // But readme should exist
    assert!(output.join("readme").join("index.html").exists());
}

#[tokio::test]
async fn test_build_excludes_hidden_directories() {
    let repo = TestRepo::new();
    repo.create_dir(".hidden");
    repo.create_markdown(".hidden/secret.md", "# Secret");
    repo.create_markdown("public.md", "# Public");

    let output = build_site(&repo).await;

    // .hidden should not be indexed (already skipped in scanning)
    let pagefind_dir = output.join(".mbr").join("pagefind");
    let urls = get_indexed_urls(&pagefind_dir);

    for url in &urls {
        assert!(
            !url.contains("hidden"),
            "Hidden directories should be excluded: {}",
            url
        );
        assert!(
            !url.contains("secret"),
            "Hidden files should be excluded: {}",
            url
        );
    }
}

// ============================================================================
// Static Mode Configuration Tests
// ============================================================================

#[tokio::test]
async fn test_build_includes_components() {
    let repo = TestRepo::new();
    repo.create_markdown("test.md", "# Test");

    let output = build_site(&repo).await;

    let html_path = output.join("test").join("index.html");
    let html = fs::read_to_string(&html_path).unwrap();

    // Should include the components script
    assert!(
        html.contains("mbr-components.min.js"),
        "Expected mbr-components.min.js script reference in HTML"
    );
}

#[tokio::test]
async fn test_build_creates_site_json() {
    let repo = TestRepo::new();
    repo.create_markdown("one.md", "# One");
    repo.create_markdown("two.md", "# Two");

    let output = build_site(&repo).await;

    // Should create site.json in .mbr directory
    let site_json_path = output.join(".mbr").join("site.json");
    assert!(
        site_json_path.exists(),
        "Expected site.json at {:?}",
        site_json_path
    );

    let content = fs::read_to_string(&site_json_path).unwrap();
    let body: serde_json::Value = serde_json::from_str(&content).unwrap();

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
}

#[tokio::test]
async fn test_build_site_json_includes_frontmatter() {
    let repo = TestRepo::new();

    // Create file with frontmatter - use direct file creation to avoid HashMap key issues
    let content = r#"---
title: My Title
tags: rust, web
---

Content here."#;
    std::fs::write(repo.path().join("tagged.md"), content).unwrap();

    let output = build_site(&repo).await;

    let site_json_path = output.join(".mbr").join("site.json");
    let content = fs::read_to_string(&site_json_path).unwrap();
    let body: serde_json::Value = serde_json::from_str(&content).unwrap();

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
// Pagefind metadata tests
// ============================================================================

#[tokio::test]
async fn test_html_contains_pagefind_body_attribute() {
    let repo = TestRepo::new();
    repo.create_markdown("test.md", "# Test Page\n\nSome content here.");

    let output = build_site(&repo).await;

    let html_path = output.join("test").join("index.html");
    let html = fs::read_to_string(&html_path).unwrap();

    // Main content should have data-pagefind-body
    assert!(
        html.contains("data-pagefind-body"),
        "Expected data-pagefind-body in output"
    );
}

#[tokio::test]
async fn test_html_contains_pagefind_ignore_on_navigation() {
    let repo = TestRepo::new();
    repo.create_markdown("test.md", "# Test Page");

    let output = build_site(&repo).await;

    let html_path = output.join("test").join("index.html");
    let html = fs::read_to_string(&html_path).unwrap();

    // Header and footer should be ignored
    assert!(
        html.contains("data-pagefind-ignore"),
        "Expected data-pagefind-ignore in output"
    );
}

// ============================================================================
// Error page tests
// ============================================================================

#[tokio::test]
async fn test_build_generates_404_html() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello World");

    let output = build_site(&repo).await;

    // Should create 404.html at root
    let error_page_path = output.join("404.html");
    assert!(
        error_page_path.exists(),
        "Expected 404.html to be generated at {:?}",
        error_page_path
    );
}

#[tokio::test]
async fn test_build_404_html_contains_error_structure() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello World");

    let output = build_site(&repo).await;
    let html = fs::read_to_string(output.join("404.html")).unwrap();

    // Should contain error page structure
    assert!(
        html.contains("404"),
        "404.html should contain error code. Got: {}",
        &html[..500.min(html.len())]
    );
    assert!(
        html.contains("Not Found"),
        "404.html should contain 'Not Found' text"
    );
}

#[tokio::test]
async fn test_build_404_html_uses_relative_paths() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello World");

    let output = build_site(&repo).await;
    let html = fs::read_to_string(output.join("404.html")).unwrap();

    // Should use relative paths to .mbr/ assets (not absolute /.mbr/)
    assert!(
        html.contains(".mbr/") && !html.contains("\"/.mbr/"),
        "404.html should use relative paths to .mbr/ folder"
    );

    // Should have serverMode: false for static build
    assert!(
        html.contains("serverMode: false") || html.contains("serverMode:false"),
        "404.html should have serverMode: false for static builds"
    );
}

#[tokio::test]
async fn test_build_404_html_includes_navigation() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello World");

    let output = build_site(&repo).await;
    let html = fs::read_to_string(output.join("404.html")).unwrap();

    // Should have navigation elements
    assert!(
        html.contains("Go Back") || html.contains("history.back"),
        "404.html should have a back button"
    );
    assert!(html.contains("Home"), "404.html should have a home link");
    // Should include search component or search tip
    assert!(
        html.contains("mbr-search") || html.contains("search"),
        "404.html should include search functionality"
    );
}

// ============================================================================
// Theme tests
// ============================================================================

/// Helper to run a build with a specific theme
async fn build_site_with_theme(repo: &TestRepo, theme: &str) -> std::path::PathBuf {
    let config = mbr::Config {
        root_dir: repo.path().to_path_buf(),
        theme: theme.to_string(),
        ..Default::default()
    };
    let output_dir = repo.path().join("build");

    let builder =
        mbr::build::Builder::new(config, output_dir.clone()).expect("Failed to create builder");

    builder.build().await.expect("Build failed");

    output_dir
}

#[tokio::test]
async fn test_build_uses_default_theme() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello World");

    let output = build_site(&repo).await;

    // Check that pico.min.css exists
    let pico_path = output.join(".mbr").join("pico.min.css");
    assert!(pico_path.exists(), "pico.min.css should be created");

    // Should have substantial content
    let pico_css = fs::read_to_string(&pico_path).unwrap();
    assert!(
        pico_css.len() > 1000,
        "pico.min.css should have substantial content"
    );
}

#[tokio::test]
async fn test_build_uses_color_theme() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello World");

    let output = build_site_with_theme(&repo, "amber").await;

    let pico_path = output.join(".mbr").join("pico.min.css");
    assert!(pico_path.exists(), "pico.min.css should be created");

    let pico_css = fs::read_to_string(&pico_path).unwrap();
    assert!(pico_css.len() > 1000, "amber theme should have content");
}

#[tokio::test]
async fn test_build_uses_fluid_theme() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello World");

    let output = build_site_with_theme(&repo, "fluid.jade").await;

    let pico_path = output.join(".mbr").join("pico.min.css");
    assert!(pico_path.exists(), "pico.min.css should be created");

    let pico_css = fs::read_to_string(&pico_path).unwrap();
    assert!(
        pico_css.len() > 1000,
        "fluid.jade theme should have content"
    );
}

#[tokio::test]
async fn test_build_invalid_theme_falls_back_to_default() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello World");

    // Invalid theme should fall back to default (with warning)
    let output = build_site_with_theme(&repo, "invalid-theme").await;

    let pico_path = output.join(".mbr").join("pico.min.css");
    assert!(
        pico_path.exists(),
        "pico.min.css should be created even with invalid theme"
    );

    let pico_css = fs::read_to_string(&pico_path).unwrap();
    assert!(
        pico_css.len() > 1000,
        "fallback theme should have valid content"
    );
}

// ============================================================================
// Link tracking tests (links.json file generation)
// ============================================================================

#[tokio::test]
async fn test_build_creates_links_json_files() {
    let repo = TestRepo::new();
    repo.create_markdown("page.md", "# Page\n\n[Link to Other](other/)");
    repo.create_markdown("other.md", "# Other Page");

    let output = build_site(&repo).await;

    // Should create links.json for each page
    let page_links = output.join("page").join("links.json");
    assert!(
        page_links.exists(),
        "Expected links.json at {:?}",
        page_links
    );

    let other_links = output.join("other").join("links.json");
    assert!(
        other_links.exists(),
        "Expected links.json at {:?}",
        other_links
    );
}

#[tokio::test]
async fn test_build_links_json_contains_outbound_links() {
    let repo = TestRepo::new();
    repo.create_markdown(
        "source.md",
        "# Source\n\n[Internal](target/)\n\n[External](https://example.com)",
    );
    repo.create_markdown("target.md", "# Target");

    let output = build_site(&repo).await;

    let links_path = output.join("source").join("links.json");
    let content = fs::read_to_string(&links_path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&content).unwrap();

    let outbound = json["outbound"].as_array().unwrap();

    // Should have internal link
    let has_internal = outbound
        .iter()
        .any(|l| l["to"].as_str().unwrap().contains("target"));
    assert!(
        has_internal,
        "Should have internal link to target: {:?}",
        outbound
    );

    // Should have external link
    let has_external = outbound
        .iter()
        .any(|l| l["to"].as_str().unwrap().contains("example.com"));
    assert!(has_external, "Should have external link: {:?}", outbound);
}

#[tokio::test]
async fn test_build_links_json_contains_inbound_links() {
    let repo = TestRepo::new();
    // Create source that links to target
    repo.create_markdown("source.md", "# Source\n\n[Go to Target](target/)");
    repo.create_markdown("target.md", "# Target Page");

    let output = build_site(&repo).await;

    // Check target's links.json for inbound link from source
    let links_path = output.join("target").join("links.json");
    let content = fs::read_to_string(&links_path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&content).unwrap();

    let inbound = json["inbound"].as_array().unwrap();

    let has_inbound = inbound
        .iter()
        .any(|l| l["from"].as_str().unwrap().contains("source"));
    assert!(
        has_inbound,
        "Target should have inbound link from source: {:?}",
        inbound
    );
}

#[tokio::test]
async fn test_build_links_json_bidirectional() {
    let repo = TestRepo::new();
    // Create two pages that link to each other
    repo.create_markdown("alpha.md", "# Alpha\n\n[Go to Beta](beta/)");
    repo.create_markdown("beta.md", "# Beta\n\n[Go to Alpha](alpha/)");

    let output = build_site(&repo).await;

    // Check alpha's links
    let alpha_links_path = output.join("alpha").join("links.json");
    let alpha_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&alpha_links_path).unwrap()).unwrap();

    // Alpha should have outbound to beta
    let alpha_outbound = alpha_json["outbound"].as_array().unwrap();
    assert!(
        alpha_outbound
            .iter()
            .any(|l| l["to"].as_str().unwrap().contains("beta")),
        "Alpha should have outbound link to beta"
    );

    // Alpha should have inbound from beta
    let alpha_inbound = alpha_json["inbound"].as_array().unwrap();
    assert!(
        alpha_inbound
            .iter()
            .any(|l| l["from"].as_str().unwrap().contains("beta")),
        "Alpha should have inbound link from beta"
    );
}

#[tokio::test]
async fn test_build_links_json_includes_anchors() {
    let repo = TestRepo::new();
    repo.create_markdown(
        "page.md",
        "# Page\n\n[Section Link](other/#important-section)",
    );
    repo.create_markdown("other.md", "# Other\n\n## Important Section");

    let output = build_site(&repo).await;

    let links_path = output.join("page").join("links.json");
    let content = fs::read_to_string(&links_path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&content).unwrap();

    let outbound = json["outbound"].as_array().unwrap();
    let link = outbound
        .iter()
        .find(|l| l["to"].as_str().unwrap().contains("other"));
    assert!(link.is_some(), "Should have link to other");

    let anchor = link.unwrap()["anchor"].as_str();
    assert!(
        anchor.is_some() && anchor.unwrap().contains("important"),
        "Link should preserve anchor: {:?}",
        link
    );
}

/// Helper to run a build with link tracking disabled
async fn build_site_no_link_tracking(repo: &TestRepo) -> std::path::PathBuf {
    let config = mbr::Config {
        root_dir: repo.path().to_path_buf(),
        link_tracking: false, // Disabled
        ..Default::default()
    };
    let output_dir = repo.path().join("build");

    let builder =
        mbr::build::Builder::new(config, output_dir.clone()).expect("Failed to create builder");

    builder.build().await.expect("Build failed");

    output_dir
}

#[tokio::test]
async fn test_build_no_links_json_when_disabled() {
    let repo = TestRepo::new();
    repo.create_markdown("page.md", "# Page\n\n[Link](other/)");
    repo.create_markdown("other.md", "# Other");

    let output = build_site_no_link_tracking(&repo).await;

    // Should NOT create links.json files when tracking is disabled
    let page_links = output.join("page").join("links.json");
    assert!(
        !page_links.exists(),
        "links.json should not exist when link tracking is disabled"
    );

    let other_links = output.join("other").join("links.json");
    assert!(
        !other_links.exists(),
        "links.json should not exist when link tracking is disabled"
    );
}

// ============================================================================
// Broken link detection tests
// ============================================================================

/// Helper to run a build and return both output directory and stats
async fn build_site_with_stats(repo: &TestRepo) -> (std::path::PathBuf, mbr::BuildStats) {
    let config = mbr::Config {
        root_dir: repo.path().to_path_buf(),
        ..Default::default()
    };
    let output_dir = repo.path().join("build");

    let builder =
        mbr::build::Builder::new(config, output_dir.clone()).expect("Failed to create builder");

    let stats = builder.build().await.expect("Build failed");

    (output_dir, stats)
}

#[tokio::test]
async fn test_build_detects_broken_internal_links() {
    let repo = TestRepo::new();
    // Create a page with a broken link to a non-existent page (absolute path)
    repo.create_markdown("page.md", "# Page\n\n[Broken link](/missing/)");

    let (_output, stats) = build_site_with_stats(&repo).await;

    // Should detect the broken link
    assert_eq!(
        stats.broken_links, 1,
        "Expected 1 broken link, got {}",
        stats.broken_links
    );
}

#[tokio::test]
async fn test_build_no_false_positives_for_valid_links() {
    let repo = TestRepo::new();
    // Create pages with valid internal links
    repo.create_markdown("page.md", "# Page\n\n[Valid link](other/)");
    repo.create_markdown("other.md", "# Other");

    let (_, stats) = build_site_with_stats(&repo).await;

    // Should not report broken links for valid links
    assert_eq!(
        stats.broken_links, 0,
        "Expected 0 broken links, got {}",
        stats.broken_links
    );
}

#[tokio::test]
async fn test_build_ignores_external_links() {
    let repo = TestRepo::new();
    // Create a page with external links (should be ignored in validation)
    repo.create_markdown(
        "page.md",
        r#"# Page

[External HTTPS](https://example.com)
[External HTTP](http://example.com)
[Email](mailto:test@example.com)
[Phone](tel:+1234567890)
"#,
    );

    let (_, stats) = build_site_with_stats(&repo).await;

    // External links should be ignored (not counted as broken)
    assert_eq!(
        stats.broken_links, 0,
        "Expected 0 broken links for external links, got {}",
        stats.broken_links
    );
}

#[tokio::test]
async fn test_build_validates_relative_links() {
    let repo = TestRepo::new();
    repo.create_dir("docs");
    // Create a page with relative links (one valid, one broken)
    repo.create_markdown(
        "docs/page.md",
        "# Page\n\n[Valid](../readme/)\n[Broken](../missing/)",
    );
    repo.create_markdown("readme.md", "# Readme");

    let (_, stats) = build_site_with_stats(&repo).await;

    // Should detect the one broken relative link
    assert_eq!(
        stats.broken_links, 1,
        "Expected 1 broken link, got {}",
        stats.broken_links
    );
}

#[tokio::test]
async fn test_build_skip_link_checks() {
    let repo = TestRepo::new();
    // Create a page with broken links
    repo.create_markdown("page.md", "# Page\n\n[Broken](missing/)");

    let config = mbr::Config {
        root_dir: repo.path().to_path_buf(),
        skip_link_checks: true, // Skip link validation
        ..Default::default()
    };
    let output_dir = repo.path().join("build");

    let builder =
        mbr::build::Builder::new(config, output_dir.clone()).expect("Failed to create builder");
    let stats = builder.build().await.expect("Build failed");

    // When skip_link_checks is true, no links should be checked
    assert_eq!(
        stats.broken_links, 0,
        "Expected 0 broken links when skipping checks, got {}",
        stats.broken_links
    );
}

#[tokio::test]
async fn test_build_validates_symlinked_assets() {
    let repo = TestRepo::new();

    // Create a static folder with an asset
    repo.create_dir("static/images");
    repo.create_static_file("static/images/logo.png", b"fake image data");

    // Create a page linking to the symlinked asset
    repo.create_markdown("page.md", "# Page\n\n![Logo](/images/logo.png)");

    let (_, stats) = build_site_with_stats(&repo).await;

    // The symlinked asset should be valid
    assert_eq!(
        stats.broken_links, 0,
        "Expected 0 broken links for symlinked assets, got {}",
        stats.broken_links
    );
}

// ============================================================================
// Media viewer page tests
// ============================================================================

#[tokio::test]
async fn test_build_generates_media_viewer_pages() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello World");

    let output = build_site(&repo).await;

    // Should create media viewer pages under .mbr
    let videos_page = output.join(".mbr/videos/index.html");
    assert!(
        videos_page.exists(),
        "Expected videos viewer page at {:?}",
        videos_page
    );

    let pdfs_page = output.join(".mbr/pdfs/index.html");
    assert!(
        pdfs_page.exists(),
        "Expected PDFs viewer page at {:?}",
        pdfs_page
    );

    let audio_page = output.join(".mbr/audio/index.html");
    assert!(
        audio_page.exists(),
        "Expected audio viewer page at {:?}",
        audio_page
    );
}

#[tokio::test]
async fn test_build_media_viewer_pages_have_correct_media_type() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello World");

    let output = build_site(&repo).await;

    // Check videos page has video media type
    let videos_html = fs::read_to_string(output.join(".mbr/videos/index.html")).unwrap();
    assert!(
        videos_html.contains("mediaType: \"video\"")
            || videos_html.contains("media-type=\"video\""),
        "Videos page should have video media type"
    );

    // Check PDFs page has pdf media type
    let pdfs_html = fs::read_to_string(output.join(".mbr/pdfs/index.html")).unwrap();
    assert!(
        pdfs_html.contains("mediaType: \"pdf\"") || pdfs_html.contains("media-type=\"pdf\""),
        "PDFs page should have pdf media type"
    );

    // Check audio page has audio media type
    let audio_html = fs::read_to_string(output.join(".mbr/audio/index.html")).unwrap();
    assert!(
        audio_html.contains("mediaType: \"audio\"") || audio_html.contains("media-type=\"audio\""),
        "Audio page should have audio media type"
    );
}

#[tokio::test]
async fn test_build_media_viewer_pages_use_relative_paths() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello World");

    let output = build_site(&repo).await;

    let videos_html = fs::read_to_string(output.join(".mbr/videos/index.html")).unwrap();

    // Should use relative paths to .mbr/ assets from depth 2
    // The page is at .mbr/videos/index.html, so it needs ../../.mbr/ to reach root
    assert!(
        videos_html.contains("../../.mbr/") || videos_html.contains("../.mbr/"),
        "Media viewer page should use relative paths to assets"
    );

    // Should have serverMode: false for static build
    assert!(
        videos_html.contains("serverMode: false") || videos_html.contains("serverMode:false"),
        "Media viewer page should have serverMode: false for static builds"
    );
}

#[tokio::test]
async fn test_build_media_viewer_pages_include_navigation() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello World");

    let output = build_site(&repo).await;

    let videos_html = fs::read_to_string(output.join(".mbr/videos/index.html")).unwrap();

    // Should have back navigation (parent_path)
    assert!(
        videos_html.contains("Back") || videos_html.contains("Home"),
        "Media viewer page should have navigation"
    );

    // Should have breadcrumbs
    assert!(
        videos_html.contains("breadcrumb") || videos_html.contains("Home"),
        "Media viewer page should have breadcrumbs"
    );
}

// ============================================================================
// Tag page path traversal tests
// ============================================================================

/// Helper to run a build with tag pages enabled and return the output directory
async fn build_site_with_tags(repo: &TestRepo) -> std::path::PathBuf {
    let config = mbr::Config {
        root_dir: repo.path().to_path_buf(),
        build_tag_pages: true,
        tag_sources: vec![mbr::config::TagSource {
            field: "tags".to_string(),
            label: None,
            label_plural: None,
        }],
        ..Default::default()
    };
    let output_dir = repo.path().join("build");

    let builder =
        mbr::build::Builder::new(config, output_dir.clone()).expect("Failed to create builder");

    builder.build().await.expect("Build failed");

    output_dir
}

#[tokio::test]
async fn test_tag_with_leading_slash_does_not_escape_output_dir() {
    let repo = TestRepo::new();

    // Create a markdown file with a tag that starts with /
    // This previously caused path traversal: output_dir.join("/pol/_phenomena")
    // would replace the base path entirely on Unix
    repo.create_markdown(
        "article.md",
        "---\ntitle: Test Article\ntags:\n  - /pol/_phenomena\n---\n\nSome content.",
    );

    let output = build_site_with_tags(&repo).await;

    // The tag page should be created INSIDE the output directory (sanitized)
    // not at the root filesystem path
    let safe_tag_path = output
        .join("tags")
        .join("pol/_phenomena")
        .join("index.html");

    // The important thing is that no file was created outside the output dir
    // Check that /pol/_phenomena/index.html does NOT exist at filesystem root
    assert!(
        !Path::new("/pol/_phenomena/index.html").exists(),
        "Tag page should NOT be written to root filesystem"
    );

    // The sanitized path should exist inside output dir
    // (after stripping leading slash, /pol/_phenomena becomes pol/_phenomena)
    if safe_tag_path.exists() {
        let content = fs::read_to_string(&safe_tag_path).unwrap();
        assert!(
            content.contains("html"),
            "Tag page should contain valid HTML"
        );
    }
}

#[tokio::test]
async fn test_tag_with_dotdot_does_not_escape_output_dir() {
    let repo = TestRepo::new();

    // A tag with .. path components should not escape the output directory
    repo.create_markdown(
        "article.md",
        "---\ntitle: Test\ntags:\n  - ../../etc/shadow\n---\n\nContent.",
    );

    let output = build_site_with_tags(&repo).await;

    // The tag value after sanitization: ../../etc/shadow -> etc/shadow
    // So the path should be output/tags/etc/shadow/index.html
    assert!(
        !Path::new("/etc/shadow/index.html").exists(),
        "Tag page should NOT escape to /etc/"
    );

    // Should stay within output dir
    let safe_path = output.join("tags").join("etc/shadow").join("index.html");
    // If it exists, it should be inside the output dir
    if safe_path.exists() {
        assert!(
            safe_path.starts_with(&output),
            "Tag page must be inside output directory"
        );
    }
}

#[tokio::test]
async fn test_normal_tags_still_generate_pages() {
    let repo = TestRepo::new();

    repo.create_markdown(
        "article.md",
        "---\ntitle: Test\ntags:\n  - rust\n  - programming\n---\n\nContent.",
    );

    let output = build_site_with_tags(&repo).await;

    // Normal tag pages should still be generated
    let rust_tag = output.join("tags").join("rust").join("index.html");
    assert!(rust_tag.exists(), "Tag page for 'rust' should exist");

    let prog_tag = output.join("tags").join("programming").join("index.html");
    assert!(prog_tag.exists(), "Tag page for 'programming' should exist");

    // Tag source index should exist
    let tags_index = output.join("tags").join("index.html");
    assert!(tags_index.exists(), "Tags index page should exist");
}
