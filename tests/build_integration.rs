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
