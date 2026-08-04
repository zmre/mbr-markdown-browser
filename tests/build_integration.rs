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
async fn test_build_embeds_bare_giphy_url_without_network() {
    // Regression: no-network embeds (Giphy) must render in static builds even at
    // the build-default oembed timeout of 0 (which skips network fetches). The
    // bare URL should become a giphy-embed figure, not a plain <a> hotlink.
    let repo = TestRepo::new();
    repo.create_markdown(
        "index.md",
        "# Gallery\n\nhttps://giphy.com/gifs/cat-funny-CAxbo8KC2A0y4\n",
    );

    let config = mbr::Config {
        root_dir: repo.path().to_path_buf(),
        oembed_timeout_ms: 0, // build default: network fetches disabled
        ..Default::default()
    };
    let output_dir = repo.path().join("build");
    let builder =
        mbr::build::Builder::new(config, output_dir.clone()).expect("Failed to create builder");
    builder.build().await.expect("Build failed");

    // index.md at the root renders to build/index.html
    let html = fs::read_to_string(output_dir.join("index.html")).unwrap();
    assert!(
        html.contains("giphy-embed"),
        "expected a giphy embed, got:\n{html}"
    );
    assert!(
        !html.contains(r#"<a href="https://giphy.com"#),
        "bare giphy URL should not remain a plain hotlink, got:\n{html}"
    );
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

#[tokio::test]
async fn test_build_omits_find_bar() {
    let repo = TestRepo::new();
    repo.create_markdown("test.md", "# Test");

    let output = build_site(&repo).await;

    let html_path = output.join("test").join("index.html");
    let html = fs::read_to_string(&html_path).unwrap();

    // The find bar is GUI-only. Builds never set gui_mode, so the
    // `{% if gui_mode %}` gate in _footer.html must keep it out of every
    // static page -- otherwise it ships to every site mbr builds.
    assert!(
        !html.contains("mbr-find-bar"),
        "Static builds must not ship the GUI-only find bar"
    );
    assert!(
        html.contains("guiMode: false"),
        "Expected guiMode: false in output"
    );
}

#[tokio::test]
async fn test_build_head_includes_graph_depth() {
    let repo = TestRepo::new();
    repo.create_markdown("test.md", "# Test");

    let output = build_site(&repo).await;

    let html_path = output.join("test").join("index.html");
    let html = fs::read_to_string(&html_path).unwrap();

    // The default graph_depth flows into the __MBR_CONFIG__ head script.
    assert!(
        html.contains("graphDepth: 2"),
        "Expected graphDepth: 2 in built HTML"
    );
}

/// The task browser is server/GUI only: its index is built by reading live
/// files, which a published static site does not have. Every page kind must
/// therefore advertise `tasksEnabled: false`, no matter what the config says.
#[tokio::test]
async fn test_build_head_disables_tasks() {
    let repo = TestRepo::new();
    repo.create_markdown("test.md", "# Test");
    repo.create_markdown("docs/guide.md", "# Guide");

    let output = build_site(&repo).await;

    for page in [
        output.join("index.html"),              // home
        output.join("test").join("index.html"), // markdown page
        output.join("docs").join("index.html"), // section page
        output.join("docs").join("guide").join("index.html"),
    ] {
        let html = fs::read_to_string(&page).unwrap();
        assert!(
            html.contains("tasksEnabled: false"),
            "Expected tasksEnabled: false in {}",
            page.display()
        );
    }
}

#[tokio::test]
async fn test_build_no_incomplete_spans_by_default() {
    // Static builds default mark_incomplete=false; published sites must not
    // contain mbr-incomplete spans unless the user explicitly opts in.
    let repo = TestRepo::new();
    repo.create_markdown(
        "drafts.md",
        "# Drafts\n\nTK rewrite this paragraph.\n\n- TODO finish item\n",
    );

    let output = build_site(&repo).await;

    let html_path = output.join("drafts").join("index.html");
    let html = fs::read_to_string(&html_path).expect("rendered html");
    assert!(
        !html.contains("mbr-incomplete"),
        "Default build should not include mbr-incomplete spans: {html}"
    );
}

#[tokio::test]
async fn test_build_incomplete_spans_when_opted_in() {
    // When mark_incomplete is forced on (e.g., via CLI/config), the build
    // should emit the spans.
    let repo = TestRepo::new();
    repo.create_markdown(
        "drafts.md",
        "# Drafts\n\nTK rewrite this paragraph.\n\nNormal paragraph.",
    );

    let mut config = mbr::Config {
        root_dir: repo.path().to_path_buf(),
        ..Default::default()
    };
    config.mark_incomplete = Some(true);

    let output_dir = repo.path().join("build");
    let builder = mbr::build::Builder::new(config, output_dir.clone()).expect("builder");
    builder.build().await.expect("build");

    let html_path = output_dir.join("drafts").join("index.html");
    let html = fs::read_to_string(&html_path).expect("rendered html");
    assert!(
        html.contains(r#"<span class="mbr-incomplete">"#),
        "Opted-in build should include mbr-incomplete spans: {html}"
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
// KaTeX asset tests
// ============================================================================

/// Regression test: KaTeX renders in server/GUI mode because
/// serve_default_file() also serves KATEX_FILES, but the static build's
/// `.mbr` step only mirrored DEFAULT_FILES. That gap meant the CSS, JS, and
/// WOFF2 fonts were never written, so <mbr-katex>'s fetch of
/// `.mbr/katex.min.css` (and the CSS's relative `fonts/…` urls) 404'd and math
/// never rendered. The build must write these assets, including the nested
/// `fonts/` subdirectory.
#[tokio::test]
async fn test_build_writes_katex_assets() {
    let repo = TestRepo::new();
    repo.create_markdown("math.md", "# Math\n\n$$E = mc^2$$");

    let output = build_site(&repo).await;

    let css = output.join(".mbr").join("katex.min.css");
    let js = output.join(".mbr").join("katex.min.js");
    let font = output
        .join(".mbr")
        .join("fonts")
        .join("KaTeX_Main-Regular.woff2");

    assert!(css.exists(), "Expected KaTeX CSS at {:?}", css);
    assert!(js.exists(), "Expected KaTeX JS at {:?}", js);
    assert!(font.exists(), "Expected KaTeX font at {:?}", font);
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

// ============================================================================
// Link tracking + static file symlink tests
// ============================================================================

#[tokio::test]
async fn test_static_file_not_shadowed_by_link_tracking() {
    let repo = TestRepo::new();

    // Create a PDF file in pdfs/ directory
    repo.create_static_file("pdfs/example.pdf", b"%PDF-1.4 fake pdf content");

    // Create a markdown file that links to the PDF
    repo.create_markdown(
        "page.md",
        "# My Page\n\nCheck out [this PDF](/pdfs/example.pdf).",
    );

    // Build with link tracking enabled (default)
    let output = build_site(&repo).await;

    // The PDF should be symlinked, not turned into a directory
    let pdf_path = output.join("pdfs").join("example.pdf");
    assert!(pdf_path.exists(), "PDF file should exist in build output");
    assert!(
        !pdf_path.is_dir(),
        "PDF path should be a file (symlink), not a directory created by link tracking"
    );

    // No links.json should exist for the static file
    let links_json = output.join("pdfs").join("example.pdf").join("links.json");
    assert!(
        !links_json.exists(),
        "links.json should not be created for static files"
    );
}

// ============================================================================
// Tag page generation + manual link tests
// ============================================================================

#[tokio::test]
async fn test_tag_page_exists_when_linked_manually() {
    let repo = TestRepo::new();

    // Page A has a tag in frontmatter
    repo.create_markdown(
        "article.md",
        "---\ntitle: Article\ntags:\n  - mytag\n---\n\nSome content.",
    );

    // Page B manually links to the tag page
    repo.create_markdown(
        "index.md",
        "# Home\n\nSee articles tagged [mytag](/tags/mytag/).",
    );

    let output = build_site_with_tags(&repo).await;

    // The tag page should exist
    let tag_page = output.join("tags").join("mytag").join("index.html");
    assert!(
        tag_page.exists(),
        "Tag page for 'mytag' should be generated"
    );

    // The tag index page should exist
    let tag_index = output.join("tags").join("index.html");
    assert!(tag_index.exists(), "Tags index page should exist");
}

// ============================================================================
// Readability Scores (window.extendedMeta)
// ============================================================================

#[tokio::test]
async fn test_build_injects_readability_scores() {
    let repo = TestRepo::new();
    repo.create_markdown(
        "article.md",
        "# Article\n\nThis is a simple test. It has several short sentences. \
         The quick brown fox jumps over the lazy dog. Another sentence follows.\n",
    );

    let output = build_site(&repo).await;

    let html_path = output.join("article").join("index.html");
    assert!(html_path.exists(), "Expected {:?} to exist", html_path);
    let html = fs::read_to_string(&html_path).unwrap();

    assert!(
        html.contains("fleschReadingEase:"),
        "Static build should include fleschReadingEase in extendedMeta"
    );
    assert!(
        html.contains("fleschKincaidGrade:"),
        "Static build should include fleschKincaidGrade in extendedMeta"
    );
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
async fn test_build_readability_scores_null_for_code_only() {
    let repo = TestRepo::new();
    repo.create_markdown(
        "code-only.md",
        "```rust\nfn main() { println!(\"hello\"); }\n```\n",
    );

    let output = build_site(&repo).await;

    let html_path = output.join("code-only").join("index.html");
    let html = fs::read_to_string(&html_path).unwrap();

    assert!(
        html.contains("fleschReadingEase: null"),
        "FRE should render as null for a code-only document"
    );
    assert!(
        html.contains("fleschKincaidGrade: null"),
        "FKGL should render as null for a code-only document"
    );
}

// ============================================================================
// Static build guarantee: per-page error surface must never leak into output
// ============================================================================

/// Recursively collect every regular file under `root`.
fn walk_all_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // Follow into directories only (don't chase symlinks to foreign trees).
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_dir() {
                stack.push(path);
            } else if meta.file_type().is_file() {
                out.push(path);
            }
        }
    }
    out
}

#[tokio::test]
async fn test_build_output_contains_no_errors_json_files() {
    let repo = TestRepo::new();
    repo.create_markdown(
        "readme.md",
        "# Hello\n\n[link](./other/)\n\n![img](./img.png)",
    );
    repo.create_markdown("other.md", "# Other");

    let output = build_site(&repo).await;

    let all_files = walk_all_files(&output);
    let errors_json_files: Vec<_> = all_files
        .iter()
        .filter(|p| {
            p.file_name()
                .and_then(|f| f.to_str())
                .is_some_and(|f| f == "errors.json")
        })
        .collect();

    assert!(
        errors_json_files.is_empty(),
        "static build must not emit errors.json files (found {:?})",
        errors_json_files
    );
}

#[tokio::test]
async fn test_build_output_does_not_reference_mbr_page_errors() {
    let repo = TestRepo::new();
    repo.create_markdown("readme.md", "# Hello");
    repo.create_markdown("docs/page.md", "# Doc page");

    let output = build_site(&repo).await;

    for file in walk_all_files(&output) {
        // Only inspect HTML files; .mbr/ built assets contain the compiled JS
        // which legitimately defines the custom element class.
        let Some(ext) = file.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if ext != "html" {
            continue;
        }

        let content = fs::read_to_string(&file).unwrap_or_default();
        assert!(
            !content.contains("<mbr-page-errors"),
            "built HTML at {:?} must not contain <mbr-page-errors>, found: {}",
            file,
            content
                .lines()
                .find(|l| l.contains("<mbr-page-errors"))
                .unwrap_or("")
        );
    }
}

#[tokio::test]
async fn test_build_counts_frontmatter_parse_errors() {
    let repo = TestRepo::new();
    // Valid page (no error) plus a page with invalid YAML frontmatter
    // (`*` list markers with TAB indentation).
    repo.create_markdown("good.md", "# Good\n\nFine.");
    repo.create_markdown(
        "bad.md",
        "---\ntitle: Bad\nstyle: slides\ntags:\n\t* presentation\n\t* ai\n---\n# Bad\n",
    );

    let config = mbr::Config {
        root_dir: repo.path().to_path_buf(),
        ..Default::default()
    };
    let output_dir = repo.path().join("build");
    let builder = mbr::build::Builder::new(config, output_dir).expect("Failed to create builder");
    let stats = builder.build().await.expect("Build failed");

    assert!(
        stats.frontmatter_errors >= 1,
        "expected at least one frontmatter parse error, got {}",
        stats.frontmatter_errors
    );
}

// ============================================================================
// Typed relationships (genealogy fixture)
// ============================================================================

/// Absolute path to the committed genealogy fixture.
fn genealogy_fixture_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/genealogy")
}

/// Build the genealogy fixture into a fresh temp output dir. The `TempDir`
/// guard is returned so it outlives the assertions.
async fn build_genealogy() -> (tempfile::TempDir, std::path::PathBuf) {
    let out = tempfile::TempDir::new().expect("temp output dir");
    let output_dir = out.path().join("build");
    let config = mbr::Config {
        root_dir: genealogy_fixture_path(),
        oembed_timeout_ms: 0,
        ..Default::default()
    };
    let builder =
        mbr::build::Builder::new(config, output_dir.clone()).expect("Failed to create builder");
    builder.build().await.expect("Build failed");
    (out, output_dir)
}

#[tokio::test]
async fn test_build_writes_relationships_into_links_json() {
    let (_guard, output) = build_genealogy().await;

    let links_path = output.join("people").join("john").join("links.json");
    let body: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&links_path).expect("john links.json")).unwrap();

    let rels = body["relationships"]
        .as_array()
        .expect("relationships array");
    assert!(
        rels.iter()
            .any(|r| r["neighbor"] == "/people/george/" && r["predicate"] == "parent"),
        "John's build links.json should include parent George"
    );
    let mary = rels
        .iter()
        .find(|r| r["neighbor"] == "/people/mary/")
        .expect("mary edge");
    assert_eq!(mary["predicate"], "spouse");
    assert_eq!(mary["attributes"]["married"], "1948-06-01");
}

#[tokio::test]
async fn test_build_site_json_has_relationships() {
    let (_guard, output) = build_genealogy().await;

    let site_path = output.join(".mbr").join("site.json");
    let body: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&site_path).expect("site.json")).unwrap();

    // relationship_types registry present.
    let types = body["relationship_types"]
        .as_array()
        .expect("relationship_types");
    assert!(
        types
            .iter()
            .any(|t| t["name"] == "spouse" && t["symmetric"] == true)
    );

    // Per-note relationships attached.
    let files = body["markdown_files"].as_array().expect("markdown_files");
    let alice = files
        .iter()
        .find(|f| f["url_path"] == "/people/alice/")
        .expect("alice entry");
    let alice_rels = alice["relationships"].as_array().expect("alice rels");
    // Alice's parents (John, Mary) are derived from edges declared elsewhere.
    assert!(
        alice_rels
            .iter()
            .any(|r| r["predicate"] == "parent" && r["neighbor"] == "/people/john/"),
        "Alice should have derived parent John in site.json"
    );
}

#[tokio::test]
async fn test_build_person_infobox_and_aliases() {
    let (_guard, output) = build_genealogy().await;

    // Mary's page renders the optional person infobox (portrait / birthplace /
    // aliases). The infobox lives inside `data-pagefind-body` so its text is
    // also indexed for static search.
    let mary_html = fs::read_to_string(output.join("people").join("mary").join("index.html"))
        .expect("mary index.html");
    // The portrait <img> element (distinct from the `.mbr-person-portrait` CSS
    // rule in the scoped style block) proves the image field rendered.
    assert!(
        mary_html.contains(r#"<img class="mbr-person-portrait""#),
        "Mary's page should render the portrait img element"
    );
    assert!(
        mary_html.contains("Cheyenne, WY"),
        "Mary's page should show her birthplace"
    );
    // "Also known as ..." text is emitted only by the infobox alias line.
    assert!(
        mary_html.contains("Also known as Mary Doe"),
        "Mary's page should list her alias (maiden/married name)"
    );

    // Mary's site.json frontmatter carries the `aliases` array verbatim.
    let site: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(output.join(".mbr").join("site.json")).unwrap())
            .unwrap();
    let files = site["markdown_files"].as_array().expect("markdown_files");
    let mary = files
        .iter()
        .find(|f| f["url_path"] == "/people/mary/")
        .expect("mary entry");
    let aliases = mary["frontmatter"]["aliases"]
        .as_array()
        .expect("aliases array in Mary's frontmatter");
    assert!(
        aliases.iter().any(|a| a == "Mary Doe"),
        "Mary's frontmatter aliases should include 'Mary Doe'"
    );
}

#[tokio::test]
async fn test_build_person_page_has_genealogy_element() {
    let (_guard, output) = build_genealogy().await;

    // Person pages get the d3 genealogy chart element, not the removed
    // mermaid relationships element.
    let john_html = fs::read_to_string(output.join("people").join("john").join("index.html"))
        .expect("john index.html");
    assert!(
        john_html.contains("<mbr-genealogy"),
        "Built person page should contain <mbr-genealogy>: {john_html}"
    );
    assert!(
        !john_html.contains("<mbr-relationships"),
        "Built person page must not contain the removed <mbr-relationships> element"
    );
}

#[tokio::test]
async fn test_build_nonperson_typed_page_has_no_graph_element() {
    // Typed non-person notes lost the inline graph entirely. Uses a fresh
    // TestRepo so the shared genealogy fixture stays untouched.
    let repo = TestRepo::new();
    repo.create_markdown(
        "gandalf.md",
        "---\ntype: character\ntitle: Gandalf\n---\n\n# Gandalf\n\nA wizard.\n",
    );

    let output = build_site(&repo).await;

    let html =
        fs::read_to_string(output.join("gandalf").join("index.html")).expect("gandalf index.html");
    assert!(
        !html.contains("<mbr-genealogy"),
        "Typed non-person page must not contain <mbr-genealogy>: {html}"
    );
    assert!(
        !html.contains("<mbr-relationships"),
        "Typed non-person page must not contain the removed <mbr-relationships> element"
    );
}

#[tokio::test]
async fn test_build_writes_graph_chunks() {
    // Static builds ship the lazy-loaded chunks via DEFAULT_FILES.
    let repo = TestRepo::new();
    repo.create_markdown("test.md", "# Test");

    let output = build_site(&repo).await;

    for chunk in ["mbr-graph.min.js", "mbr-genealogy.min.js"] {
        let path = output.join(".mbr").join("components").join(chunk);
        assert!(path.exists(), "Expected chunk at {}", path.display());
        let size = fs::metadata(&path).expect("chunk metadata").len();
        assert!(size > 0, "{} should be non-empty", path.display());
    }
}

#[tokio::test]
async fn test_build_body_wikilink_resolves_globally() {
    let repo = TestRepo::new();
    // Target file in one folder; referencing page in a *different* folder.
    repo.create_markdown("Walsh/Patrick Walsh.md", "# Patrick Walsh\n\nBio.");
    repo.create_markdown(
        "Notes/family.md",
        "# Family\n\nSee [[Patrick Walsh]] and [[Totally Missing]].",
    );

    let (output, stats) = build_site_with_stats(&repo).await;

    // The target page was emitted, so the globally-resolved link points at a
    // real file in the output tree.
    let target = output
        .join("Walsh")
        .join("Patrick Walsh")
        .join("index.html");
    assert!(
        target.exists(),
        "target page should be emitted at {}",
        target.display()
    );

    // The referencing page links (relatively) to the target's URL — the build
    // relativizes the absolute URL and percent-encodes the space.
    let family_html = fs::read_to_string(output.join("Notes").join("family").join("index.html"))
        .expect("family page html");
    assert!(
        family_html.contains("Walsh/Patrick%20Walsh/"),
        "family page should link to the Patrick Walsh page, got:\n{family_html}"
    );

    // Exactly one broken link: `[[Totally Missing]]`. The resolved
    // `[[Patrick Walsh]]` contributes zero broken links.
    assert_eq!(
        stats.broken_links, 1,
        "only [[Totally Missing]] should be broken, got {}",
        stats.broken_links
    );
}

/// A published static site must not contain the absolute path of the machine
/// that built it. This is the end-to-end guard: it scans **every** generated
/// file, so it covers `site.json`, `media.json` and any HTML/JS output at once.
///
/// The fixture deliberately includes non-markdown assets — with a markdown-only
/// repo `other_files` is empty and the whole `StaticFileMetadata` surface goes
/// unchecked, which is how an absolute `metadata.path` shipped undetected.
#[tokio::test]
async fn test_built_site_contains_no_absolute_host_paths() {
    let repo = TestRepo::new();
    repo.create_markdown("note.md", "# Note");
    repo.create_markdown("docs/guide.md", "# Guide");
    repo.create_static_file("images/pic.png", b"\x89PNG\r\n\x1a\nfake");
    repo.create_static_file("docs/report.pdf", b"%PDF-1.4 fake");
    repo.create_static_file("videos/clip.mp4", b"\x00\x00\x00\x18ftypmp42");
    repo.create_static_file("data/notes.txt", b"some text");

    let output = build_site(&repo).await;
    let root_str = repo.path().to_string_lossy().to_string();

    // Sanity: the fixture must really have produced media entries, otherwise a
    // clean scan would prove nothing.
    let media: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(output.join(".mbr").join("media.json")).expect("media.json"),
    )
    .expect("valid media.json");
    assert!(
        !media["other_files"]
            .as_array()
            .expect("other_files")
            .is_empty(),
        "fixture must produce other_files for this test to be meaningful"
    );

    // Walk the output tree without following symlinks (assets are symlinked
    // back into the source repo on Unix, and we only care about generated
    // text output, not the originals).
    let mut offenders = Vec::new();
    let mut stack = vec![output.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
            } else if let Ok(content) = fs::read_to_string(&path)
                && content.contains(&root_str)
            {
                offenders.push(path.display().to_string());
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "built site leaked the absolute repo root {root_str} in:\n  {}",
        offenders.join("\n  ")
    );
}

/// A non-canonical `root_dir` must still produce short, repo-relative URLs.
///
/// Regression guard for a canonicalization asymmetry: `scan_folder` walked from
/// `root_dir.canonicalize()` while paths were relativized against the raw
/// `root_dir`. When the two differ, `diff_paths` finds no common prefix and
/// every `url_path` embeds the whole filesystem path — which then makes the
/// build emit one "section" per markdown file instead of one per directory.
///
/// Reproducible on Unix because the temp dir is reached via a symlink
/// (`/tmp` -> `/private/tmp` on macOS), so the raw and canonical roots differ
/// exactly the way `D:\...` and `\\?\D:\...` do on Windows.
#[tokio::test]
async fn test_build_with_non_canonical_root_dir() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    // Deliberately NOT canonicalized - this is the whole point of the test.
    let root = tmp.path().to_path_buf();

    std::fs::create_dir_all(root.join("people")).expect("create people dir");
    std::fs::write(root.join("index.md"), "# Home").expect("write index");
    std::fs::write(root.join("people/john.md"), "# John").expect("write john");
    std::fs::write(root.join("people/jane.md"), "# Jane").expect("write jane");

    // Sanity: the test is only meaningful if the root really is non-canonical.
    let canonical = root.canonicalize().expect("canonicalize root");
    if canonical == root {
        eprintln!("skipping: temp dir is already canonical on this platform");
        return;
    }

    let output_dir = tmp.path().join("out");
    let config = mbr::Config {
        root_dir: root.clone(),
        oembed_timeout_ms: 0,
        ..Default::default()
    };
    let builder =
        mbr::build::Builder::new(config, output_dir.clone()).expect("Failed to create builder");
    let stats = builder.build().await.expect("Build failed");

    // Exactly one section page per directory: the root and `people`.
    assert_eq!(
        stats.section_pages, 2,
        "expected one section per directory (root + people), got {}",
        stats.section_pages
    );

    // Pages land at their short relative URLs, not at a mirror of the
    // filesystem layout.
    assert!(
        output_dir
            .join("people")
            .join("john")
            .join("index.html")
            .exists(),
        "expected people/john/index.html; output tree: {:?}",
        std::fs::read_dir(&output_dir).map(|d| d
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .collect::<Vec<_>>())
    );
    assert!(
        output_dir
            .join("people")
            .join("jane")
            .join("index.html")
            .exists(),
        "expected people/jane/index.html"
    );

    // And the recorded URLs are short and relative.
    let site: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(output_dir.join(".mbr").join("site.json")).expect("site.json"),
    )
    .expect("valid site.json");
    let mut urls: Vec<String> = site["markdown_files"]
        .as_array()
        .expect("markdown_files")
        .iter()
        .map(|f| f["url_path"].as_str().unwrap_or("").to_string())
        .collect();
    urls.sort();
    assert_eq!(
        urls,
        vec!["/", "/people/jane/", "/people/john/"],
        "url_path values must be short and repo-relative"
    );
}

// ============================================================================
// Build output containment / cleanliness / determinism
// ============================================================================

/// Collects every path under `dir` (not following symlinks).
#[cfg(unix)]
fn walk_paths_shallow_of_symlinks(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() && !meta.file_type().is_symlink() {
                stack.push(path.clone());
            }
            found.push(path);
        }
    }
    found
}

/// A directory symlink pointing outside the repository root makes
/// `repo::scan_folder` canonicalize the target, `pathdiff` produce `../…`, and
/// `url_path::path_to_url` keep the `..`. Joining that onto `--output` used to
/// write pages, links.json and assets outside the requested output directory.
#[cfg(unix)]
#[tokio::test]
async fn test_build_does_not_write_outside_output_dir_for_escaping_url_paths() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let base = tmp.path().canonicalize().expect("canonicalize");

    // Content that lives outside the repository root.
    let outside = base.join("outside").join("docs");
    fs::create_dir_all(&outside).expect("outside dir");
    fs::write(outside.join("a.md"), "# Escaped\n\n[home](/)\n").expect("write a.md");
    fs::write(outside.join("pic.png"), b"\x89PNG\r\n\x1a\nfake").expect("write pic.png");

    // Repository root with a directory symlink pointing at it.
    let root = base.join("repo");
    fs::create_dir_all(root.join(".mbr")).expect("repo/.mbr");
    fs::write(root.join("index.md"), "# Home").expect("write index.md");
    std::os::unix::fs::symlink(&outside, root.join("work")).expect("symlink work");

    // Output nested two levels deep so a `../..` escape has room to land.
    let output_dir = base.join("deep").join("out");

    let config = mbr::Config {
        root_dir: root.clone(),
        oembed_timeout_ms: 0,
        ..Default::default()
    };
    let builder =
        mbr::build::Builder::new(config, output_dir.clone()).expect("Failed to create builder");

    // The build must still succeed: escaping entries are skipped, not fatal.
    builder.build().await.expect("Build failed");

    // Nothing may have been written next to (rather than inside) the output.
    let deep = base.join("deep");
    let stray: Vec<String> = walk_paths_shallow_of_symlinks(&deep)
        .into_iter()
        .filter(|p| !p.starts_with(&output_dir))
        .map(|p| p.display().to_string())
        .collect();
    assert!(
        stray.is_empty(),
        "build wrote outside {}:\n  {}",
        output_dir.display(),
        stray.join("\n  ")
    );

    // And the ordinary page still built.
    assert!(
        output_dir.join("index.html").exists(),
        "the in-root home page must still be generated"
    );
}

/// Build mode for the `repo/content` + `repo/static` layout: the peer overlay's
/// assets must land *inside* the output directory at their served URLs, and the
/// build must write nothing beside it.
///
/// This is the build-mode half of the peer-overlay regression. Once the scanner
/// started indexing the overlay, two passes placed the same files —
/// `place_assets` from `other_files` and `handle_static_folder` from its own
/// walk — so this also pins that they cooperate instead of colliding.
#[cfg(unix)]
#[tokio::test]
async fn test_build_places_peer_static_folder_assets_inside_output() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let base = tmp.path().canonicalize().expect("canonicalize");

    let content = base.join("project").join("content");
    fs::create_dir_all(content.join(".mbr")).expect("content/.mbr");
    fs::write(content.join("index.md"), "# Home").expect("write index.md");

    let static_dir = base.join("project").join("static");
    fs::create_dir_all(static_dir.join("videos")).expect("static/videos");
    let video_bytes: &[u8] = b"\x00\x00\x00\x20ftypisom fake mp4";
    fs::write(static_dir.join("videos/demo.mp4"), video_bytes).expect("write video");
    fs::write(static_dir.join("pic.png"), b"\x89PNG\r\n\x1a\nfake").expect("write pic");

    // Nested so a `../..` escape would have somewhere visible to land.
    let output_dir = base.join("deep").join("out");

    let config = mbr::Config {
        root_dir: content.clone(),
        static_folder: "../static".to_string(),
        oembed_timeout_ms: 0,
        ..Default::default()
    };
    let builder =
        mbr::build::Builder::new(config, output_dir.clone()).expect("Failed to create builder");
    builder.build().await.expect("Build failed");

    // The assets landed at their served URLs, inside the output, and readable —
    // reading through the placed entry proves a symlink actually resolves.
    let video = output_dir.join("videos/demo.mp4");
    assert!(
        video.exists(),
        "the peer static folder's video must be placed at videos/demo.mp4"
    );
    assert_eq!(
        fs::read(&video).expect("read placed video"),
        video_bytes,
        "the placed asset must resolve to the original bytes"
    );
    assert!(
        output_dir.join("pic.png").exists(),
        "the peer static folder's image must be placed at pic.png"
    );

    // Nothing beside the output directory.
    let deep = base.join("deep");
    let stray: Vec<String> = walk_paths_shallow_of_symlinks(&deep)
        .into_iter()
        .filter(|p| !p.starts_with(&output_dir))
        .map(|p| p.display().to_string())
        .collect();
    assert!(
        stray.is_empty(),
        "build wrote outside {}:\n  {}",
        output_dir.display(),
        stray.join("\n  ")
    );

    // And nothing was written back into the source project either.
    assert!(
        !base.join("project").join("content").join("videos").exists(),
        "the build must not write assets back into the markdown root"
    );

    // site.json must not carry an escaping URL for anything.
    let site_json = fs::read_to_string(output_dir.join(".mbr/site.json")).expect("read site.json");
    assert!(
        !site_json.contains("\"../"),
        "site.json must not contain an escaping path"
    );
}

/// A markdown file inside an external static overlay is skipped, so the build
/// never gets a `../static/…` URL to join onto `--output`.
#[cfg(unix)]
#[tokio::test]
async fn test_build_skips_markdown_in_peer_static_folder() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let base = tmp.path().canonicalize().expect("canonicalize");

    let content = base.join("project").join("content");
    fs::create_dir_all(content.join(".mbr")).expect("content/.mbr");
    fs::write(content.join("index.md"), "# Home").expect("write index.md");

    let static_dir = base.join("project").join("static");
    fs::create_dir_all(&static_dir).expect("static dir");
    fs::write(static_dir.join("stray.md"), "# Stray").expect("write stray.md");
    fs::write(static_dir.join("pic.png"), b"\x89PNG\r\n\x1a\nfake").expect("write pic");

    let output_dir = base.join("deep").join("out");

    let config = mbr::Config {
        root_dir: content.clone(),
        static_folder: "../static".to_string(),
        oembed_timeout_ms: 0,
        ..Default::default()
    };
    let builder =
        mbr::build::Builder::new(config, output_dir.clone()).expect("Failed to create builder");
    builder.build().await.expect("Build failed");

    assert!(
        !output_dir.join("stray").exists(),
        "markdown in the external static overlay must not be rendered"
    );
    let stray: Vec<String> = walk_paths_shallow_of_symlinks(&base.join("deep"))
        .into_iter()
        .filter(|p| !p.starts_with(&output_dir))
        .map(|p| p.display().to_string())
        .collect();
    assert!(
        stray.is_empty(),
        "build wrote outside {}:\n  {}",
        output_dir.display(),
        stray.join("\n  ")
    );
    assert!(
        output_dir.join("pic.png").exists(),
        "the assets alongside it must still be placed"
    );
}

/// A leftover "old output" directory from an interrupted run must be swept
/// before the repository scan, otherwise its contents are indexed as real
/// content (every asset listed twice) and recreated inside the new output.
#[tokio::test]
async fn test_build_sweeps_orphaned_old_output_directory() {
    let repo = TestRepo::new();
    repo.create_markdown("note.md", "# Note");
    repo.create_static_file("images/pic.png", b"\x89PNG\r\n\x1a\nfake");

    // Leftover from a previous (interrupted) build, in the legacy PID shape.
    let orphan = repo.path().join("build.old.88888");
    fs::create_dir_all(orphan.join("images")).expect("orphan dir");
    fs::write(orphan.join("images/pic.png"), b"\x89PNG\r\n\x1a\nfake").expect("orphan asset");

    let output = build_site(&repo).await;

    assert!(
        !orphan.exists(),
        "orphaned {} should have been swept before scanning",
        orphan.display()
    );

    let site = fs::read_to_string(output.join(".mbr").join("site.json")).expect("site.json");
    let media = fs::read_to_string(output.join(".mbr").join("media.json")).expect("media.json");
    assert!(
        !site.contains("build.old."),
        "site.json must not index the orphaned build directory"
    );
    assert!(
        !media.contains("build.old."),
        "media.json must not index the orphaned build directory"
    );

    // The orphan must not be recreated inside the new output either.
    assert!(
        !output.join("build.old.88888").exists(),
        "place_assets must not recreate the orphan inside the output"
    );

    // The real asset is still there exactly once.
    let media_json: serde_json::Value = serde_json::from_str(&media).expect("valid media.json");
    let urls: Vec<&str> = media_json["other_files"]
        .as_array()
        .expect("other_files")
        .iter()
        .filter_map(|f| f["url_path"].as_str())
        .collect();
    assert_eq!(
        urls.iter().filter(|u| u.ends_with("pic.png")).count(),
        1,
        "pic.png should be indexed once, got {urls:?}"
    );
}

/// A completed build must not leave its own "old output" directory behind.
#[tokio::test]
async fn test_rebuild_leaves_no_old_output_directory() {
    let repo = TestRepo::new();
    repo.create_markdown("note.md", "# Note");

    build_site(&repo).await;
    build_site(&repo).await;

    let leftovers: Vec<String> = fs::read_dir(repo.path())
        .expect("read repo dir")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".mbr-old") || n.contains("build.old."))
        .collect();
    assert!(
        leftovers.is_empty(),
        "a normal rebuild must clean up after itself, found: {leftovers:?}"
    );
}

/// Every way of writing an internal link must produce a backlink.
///
/// `OutboundLink.to` holds the raw markdown destination, so inverting it
/// directly turned `[beta](beta.md)` into a backlink on `/beta.md/` — a URL no
/// page has — and the link vanished from the target's `links.json`. Only the
/// trailing-slash and `[[wikilink]]` spellings survived, which made the bug
/// easy to miss: the two forms this project's own docs favour both worked,
/// while the plainest markdown link, the one every other renderer accepts,
/// silently did not.
#[tokio::test]
async fn test_build_backlinks_cover_every_internal_link_style() {
    let repo = TestRepo::new();
    repo.create_markdown(
        "alpha.md",
        "---\ntitle: Alpha\n---\n\nExtension [b1](beta.md), slash [b2](beta/), wiki [[Beta]].\n",
    );
    repo.create_markdown("beta.md", "---\ntitle: Beta\n---\n\nTarget.\n");

    let output = build_site(&repo).await;

    let links: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(output.join("beta").join("links.json")).expect("beta links.json"),
    )
    .expect("valid links.json");

    let texts: Vec<&str> = links["inbound"]
        .as_array()
        .expect("inbound array")
        .iter()
        .map(|l| l["text"].as_str().expect("link text"))
        .collect();

    assert!(
        texts.contains(&"b1"),
        "an extension-style [text](beta.md) link must produce a backlink: {texts:?}"
    );
    assert!(
        texts.contains(&"b2"),
        "a trailing-slash link must produce a backlink: {texts:?}"
    );
    assert!(
        texts.contains(&"Beta"),
        "a [[wikilink]] must produce a backlink: {texts:?}"
    );
    for link in links["inbound"].as_array().expect("inbound array") {
        assert_eq!(
            link["from"].as_str(),
            Some("/alpha/"),
            "every backlink here comes from /alpha/"
        );
    }
}

/// `site.json` must not carry the media catalog: it ships in `media.json`, and
/// duplicating it makes site.json overwhelmingly media on asset-heavy repos.
/// Mirrors the server-mode assertion in `tests/server_integration.rs`.
#[tokio::test]
async fn test_build_site_json_excludes_other_files() {
    let repo = TestRepo::new();
    repo.create_markdown("note.md", "# Note");
    repo.create_static_file("images/pic.png", b"\x89PNG\r\n\x1a\nfake");
    repo.create_static_file("docs/report.pdf", b"%PDF-1.4 fake");

    let output = build_site(&repo).await;

    let site: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(output.join(".mbr").join("site.json")).expect("site.json"),
    )
    .expect("valid site.json");
    assert!(
        site["other_files"].is_null(),
        "site.json should NOT contain other_files in build mode"
    );
    assert!(
        site["markdown_files"].is_array(),
        "site.json must still list markdown_files"
    );

    // media.json remains the media catalog.
    let media: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(output.join(".mbr").join("media.json")).expect("media.json"),
    )
    .expect("valid media.json");
    let entries = media["other_files"].as_array().expect("other_files");
    assert_eq!(
        entries.len(),
        2,
        "media.json should still list both assets, got {entries:?}"
    );
}

/// Two builds of an identical repository must produce identical JSON, so
/// committed output does not churn in `git diff` and content-hash caches are
/// not invalidated by a no-op rebuild.
///
/// Every map that reaches the generated JSON used to be hash-ordered, and
/// tera's `preserve_order` feature makes serde_json object key order equal
/// *insertion* order — so a randomly-seeded `HashMap` reshuffled the output on
/// each run. Four sources had to be fixed, and this test covers all four:
///   * `MarkdownFiles`/`OtherFiles` (`src/repo.rs`) serialized by iterating a
///     randomly-seeded `papaya::HashMap`; they now sort by `url_path`.
///   * `tags_data` (`src/build.rs`) was a std `HashMap`; it is now a `BTreeMap`.
///   * `SimpleMetadata` (`src/markdown.rs`) was a std `HashMap`, so each page's
///     `frontmatter` object reordered its keys per build; it is now a
///     `BTreeMap`.
///   * every page's `links.json` `inbound` list (`src/build.rs`
///     `write_link_files`) was built by inverting the papaya `build_link_index`;
///     it is now run through `link_index::sort_inbound_links`.
///
/// The fixture therefore needs *several* files, *several* assets, *several*
/// frontmatter keys and a page with *several distinct backlink sources* — with
/// one of anything there is no order to get wrong. Byte equality across two runs
/// is on its own a weak test for randomness (two draws can coincide), so the
/// sortedness of each key sequence is asserted explicitly as well: that is
/// deterministic by construction rather than by luck.
#[tokio::test]
async fn test_build_is_byte_deterministic() {
    fn assert_sorted<T: Ord + Clone + std::fmt::Debug>(label: &str, keys: Vec<T>) {
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "{label} must be emitted in sorted order");
    }

    let tag_fields = ["topics", "people", "places", "projects", "sources", "tags"];

    let repo = TestRepo::new();
    let tag_frontmatter: String = tag_fields
        .iter()
        .map(|f| format!("{f}:\n  - alpha\n  - beta\n"))
        .collect();
    // Frontmatter keys are deliberately not written in alphabetical order.
    repo.create_markdown(
        "note.md",
        &format!(
            "---\ntitle: Note\ntype: reference\ndate: 2026-07-25\n\
             description: A note with several frontmatter keys\n\
             {tag_frontmatter}---\n\nBody text.\n"
        ),
    );
    // Three distinct pages link to note.md, so its backlink list has an order
    // to get wrong. Links use the trailing-slash directory form, which is what
    // `resolve_relative_url` resolves against the page URL.
    repo.create_markdown(
        "zeta.md",
        "---\ntitle: Zeta\ntype: note\n---\n\n[The Note](note/)\n",
    );
    repo.create_markdown(
        "docs/alpha.md",
        "---\ntitle: Alpha\ntags:\n  - alpha\n---\n\n[Note From Alpha](../note/)\n",
    );
    repo.create_markdown(
        "docs/beta.md",
        "---\ntitle: Beta\ntags:\n  - beta\n---\n\n[Note From Beta](../note/)\n",
    );
    repo.create_static_file("images/pic.png", b"\x89PNG\r\n\x1a\nfake");
    repo.create_static_file("images/other.png", b"\x89PNG\r\n\x1a\nfake2");
    repo.create_static_file("docs/report.pdf", b"%PDF-1.4 fake");

    // Build outside the repository so the first output is not scanned by the second.
    let out_dir = tempfile::TempDir::new().expect("temp output dir");

    let build_once = async |name: &str| -> std::path::PathBuf {
        let output_dir = out_dir.path().join(name);
        let config = mbr::Config {
            root_dir: repo.path().to_path_buf(),
            oembed_timeout_ms: 0,
            build_tag_pages: true,
            tag_sources: tag_fields
                .iter()
                .map(|f| mbr::config::TagSource {
                    field: f.to_string(),
                    label: None,
                    label_plural: None,
                })
                .collect(),
            ..Default::default()
        };
        let builder =
            mbr::build::Builder::new(config, output_dir.clone()).expect("Failed to create builder");
        builder.build().await.expect("Build failed");
        output_dir
    };

    let dir_a = build_once("a").await;
    let dir_b = build_once("b").await;

    let read_mbr = |dir: &Path, name: &str| {
        fs::read(dir.join(".mbr").join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
    };
    let (site_a, site_b) = (read_mbr(&dir_a, "site.json"), read_mbr(&dir_b, "site.json"));
    let (media_a, media_b) = (
        read_mbr(&dir_a, "media.json"),
        read_mbr(&dir_b, "media.json"),
    );

    let assert_same_bytes = |label: &str, a: &[u8], b: &[u8]| {
        assert!(
            a == b,
            "{label} must be byte-identical across two builds of the same repo\n\
             build a:\n{}\n\nbuild b:\n{}",
            String::from_utf8_lossy(a),
            String::from_utf8_lossy(b),
        );
    };
    assert_same_bytes("site.json", &site_a, &site_b);
    assert_same_bytes("media.json", &media_a, &media_b);

    // Every generated links.json, not just the two .mbr files: the backlink
    // lists live there, one file per page.
    let links_a = collect_links_files(&dir_a);
    let links_b = collect_links_files(&dir_b);
    assert!(
        !links_a.is_empty(),
        "the fixture must generate links.json files"
    );
    assert_eq!(
        links_a.keys().collect::<Vec<_>>(),
        links_b.keys().collect::<Vec<_>>(),
        "both builds must generate the same set of links.json files"
    );
    for (rel, bytes_a) in &links_a {
        assert_same_bytes(rel, bytes_a, &links_b[rel]);
    }

    let site: serde_json::Value = serde_json::from_slice(&site_a).expect("valid site.json");

    // Sortedness is what makes the equality above deterministic rather than
    // lucky. Assert it for each formerly hash-ordered sequence.

    // 1. tag_sources object keys (src/build.rs `tags_data`).
    let tag_source_keys: Vec<&str> = site["tag_sources"]
        .as_object()
        .expect("tag_sources object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(tag_source_keys.len(), tag_fields.len());
    assert_sorted("tag_sources keys", tag_source_keys);

    // 2. markdown_files order (src/repo.rs `MarkdownFiles::serialize`).
    let markdown_files = site["markdown_files"]
        .as_array()
        .expect("markdown_files array");
    assert_eq!(
        markdown_files.len(),
        4,
        "expected the four fixture markdown files, got {markdown_files:?}"
    );
    assert_sorted(
        "markdown_files url_paths",
        markdown_files
            .iter()
            .map(|f| f["url_path"].as_str().expect("url_path"))
            .collect(),
    );

    // 3. Per-page frontmatter keys (src/markdown.rs `SimpleMetadata`).
    let note = markdown_files
        .iter()
        .find(|f| f["url_path"] == "/note/")
        .expect("note.md in site.json");
    let frontmatter_keys: Vec<&str> = note["frontmatter"]
        .as_object()
        .expect("note frontmatter object")
        .keys()
        .map(String::as_str)
        .collect();
    // title/type/date/description plus one key per tag field.
    assert_eq!(
        frontmatter_keys.len(),
        4 + tag_fields.len(),
        "unexpected frontmatter keys: {frontmatter_keys:?}"
    );
    assert_sorted("frontmatter keys", frontmatter_keys);

    // media.json lists the assets, also in sorted order.
    let media: serde_json::Value = serde_json::from_slice(&media_a).expect("valid media.json");
    let other_files = media["other_files"].as_array().expect("other_files array");
    assert_eq!(
        other_files.len(),
        3,
        "expected the three fixture assets, got {other_files:?}"
    );
    assert_sorted(
        "other_files url_paths",
        other_files
            .iter()
            .map(|f| f["url_path"].as_str().expect("url_path"))
            .collect(),
    );

    // 4. Per-page inbound backlinks (src/build.rs `write_link_files`).
    let note_links: serde_json::Value = serde_json::from_slice(
        links_a
            .get("note/links.json")
            .expect("note/links.json in build output"),
    )
    .expect("valid note links.json");
    let inbound = note_links["inbound"].as_array().expect("inbound array");
    // Guard the fixture: with one backlink there is no order to get wrong, so
    // the byte comparison above would prove nothing about backlink ordering.
    // Note each source contributes exactly one entry — `finalize_render`
    // dedups outbound links by target, so a page cannot backlink twice.
    let distinct_sources: std::collections::BTreeSet<&str> = inbound
        .iter()
        .map(|l| l["from"].as_str().expect("from"))
        .collect();
    assert_eq!(
        distinct_sources.len(),
        3,
        "fixture must give /note/ three distinct inbound sources, got {inbound:?}"
    );
    assert_sorted(
        "note inbound backlinks",
        inbound
            .iter()
            .map(|l| {
                (
                    l["from"].as_str().expect("from"),
                    l["text"].as_str().expect("text"),
                    l["anchor"].as_str(),
                )
            })
            .collect(),
    );
}

/// Collects every generated `links.json` under `output_dir`, keyed by its path
/// relative to that root (with `/` separators so the key is the same on
/// Windows).
///
/// Symlinks are skipped: the builder symlinks asset directories on Unix, and
/// those point outside the output tree.
fn collect_links_files(output_dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, out: &mut std::collections::BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let file_type = entry.file_type().expect("file type");
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                walk(&path, root, out);
            } else if entry.file_name() == "links.json" {
                let rel = path
                    .strip_prefix(root)
                    .expect("path under output root")
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, fs::read(&path).expect("read links.json"));
            }
        }
    }

    let mut out = std::collections::BTreeMap::new();
    walk(output_dir, output_dir, &mut out);
    out
}
