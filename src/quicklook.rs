//! QuickLook preview rendering module.
//!
//! This module provides functionality for rendering markdown files to self-contained HTML
//! suitable for display in macOS QuickLook previews. The generated HTML includes all CSS
//! and JavaScript inline, with navigation features disabled.
//!
//! This module is exposed via UniFFI for Swift interop in macOS QuickLook extensions.

use crate::config::Config;
use crate::embedded_hljs;
use crate::embedded_pico;
use crate::link_transform::LinkTransformConfig;
use crate::markdown;
use crate::server::DEFAULT_FILES;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tera::{Context, Tera};
use thiserror::Error;

/// Errors that can occur during QuickLook preview rendering.
/// This type is exposed via UniFFI to Swift.
#[derive(Debug, Error)]
pub enum QuickLookError {
    #[error("Failed to read file: {message}")]
    FileReadError { message: String },

    #[error("Failed to render markdown: {message}")]
    MarkdownRenderError { message: String },

    #[error("Failed to render template: {message}")]
    TemplateRenderError { message: String },

    #[error("Failed to find config root: {message}")]
    ConfigError { message: String },

    #[error("Invalid path encoding")]
    InvalidPathEncoding,
}

/// Configuration options for QuickLook rendering.
#[derive(Debug, Clone)]
pub struct QuickLookConfig {
    /// Whether to include syntax highlighting (increases HTML size significantly)
    pub include_syntax_highlighting: bool,
    /// Whether to include mermaid diagram support
    pub include_mermaid: bool,
    /// Whether to include vidstack for video playback
    pub include_vidstack: bool,
    /// Base URL for converting relative paths (typically file:// URL of containing directory)
    pub base_url: Option<String>,
}

impl Default for QuickLookConfig {
    fn default() -> Self {
        Self {
            include_syntax_highlighting: true,
            include_mermaid: true,
            include_vidstack: true,
            base_url: None,
        }
    }
}

/// Render a markdown file to self-contained HTML for QuickLook preview.
///
/// This function:
/// 1. Finds the `.mbr/` config folder (if present) for custom themes
/// 2. Parses markdown with frontmatter extraction
/// 3. Renders through a QuickLook-specific template
/// 4. Inlines all CSS and JavaScript for self-contained HTML
/// 5. Disables navigation features (search, browse, next/prev links)
/// 6. Converts relative URLs to absolute file:// URLs
///
/// # Arguments
///
/// * `file_path` - Path to the markdown file to render
/// * `config_root` - Optional path to the root directory containing `.mbr/` folder.
///   If None, searches upward from the file's directory.
///
/// # Returns
///
/// Self-contained HTML string suitable for display in a WebView.
pub fn render_preview(
    file_path: String,
    config_root: Option<String>,
) -> Result<String, QuickLookError> {
    render_preview_with_config(file_path, config_root, QuickLookConfig::default())
}

/// Render a markdown file with custom configuration options.
pub fn render_preview_with_config(
    file_path: String,
    config_root: Option<String>,
    ql_config: QuickLookConfig,
) -> Result<String, QuickLookError> {
    let path = PathBuf::from(&file_path);

    if !path.exists() {
        return Err(QuickLookError::FileReadError {
            message: format!("File not found: {}", file_path),
        });
    }

    // Find root directory (with .mbr folder) for custom themes
    let root_path = if let Some(root) = config_root {
        PathBuf::from(root)
    } else {
        find_config_root(&path).unwrap_or_else(|| {
            path.parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."))
        })
    };

    // Load config for markdown extensions
    let config = Config::read(&root_path).unwrap_or_default();

    // Determine if this is an index file (affects link transformation)
    let is_index_file = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| name == config.index_file);

    let link_config = LinkTransformConfig {
        markdown_extensions: config.markdown_extensions.clone(),
        index_file: config.index_file.clone(),
        is_index_file,
    };

    // Create a minimal tokio runtime for async markdown rendering
    // Note: oEmbed is disabled (timeout=0) for faster preview
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| QuickLookError::MarkdownRenderError {
            message: e.to_string(),
        })?;

    let (frontmatter, headings, html) = rt
        .block_on(async { markdown::render(path.clone(), &root_path, 0, link_config).await })
        .map_err(|e| QuickLookError::MarkdownRenderError {
            message: e.to_string(),
        })?;

    // Calculate base URL for relative asset resolution
    // Use root_path (markdown repo root) to properly resolve root-relative paths like /videos/
    let base_url = ql_config.base_url.clone().unwrap_or_else(|| {
        root_path
            .to_str()
            .map(|s| format!("file://{}/", s))
            .unwrap_or_default()
    });

    // Render through QuickLook template
    render_quicklook_template(
        &html,
        frontmatter,
        headings,
        &root_path,
        &base_url,
        &ql_config,
    )
}

/// Search upward from the given path to find a directory containing `.mbr/`.
fn find_config_root(path: &Path) -> Option<PathBuf> {
    let mut current = if path.is_file() { path.parent()? } else { path };

    loop {
        let mbr_path = current.join(".mbr");
        if mbr_path.is_dir() {
            return Some(current.to_path_buf());
        }

        match current.parent() {
            Some(parent) => current = parent,
            None => return None,
        }
    }
}

/// Convert root-relative URLs (starting with /) to mbrfile:// URLs.
/// This is necessary because WKWebView's loadHTMLString() cannot access file:// URLs.
/// The Swift side registers a WKURLSchemeHandler for the mbrfile:// scheme that
/// serves local files from disk.
fn convert_root_relative_urls(html: &str, root_path: &Path) -> String {
    let root_str = root_path.to_str().unwrap_or("");

    // Replace src="/... with src="mbrfile://{root}/...
    // Replace src='/... with src='mbrfile://{root}/...
    // Same for href and poster attributes
    //
    // Note: The patterns match the start of root-relative paths (e.g., src="/)
    // and replace with the mbrfile scheme plus the root path, preserving the rest.
    let mut result = html.to_string();

    // Handle double-quoted attributes
    result = result.replace(r#"src="/"#, &format!(r#"src="mbrfile://{}/"#, root_str));
    result = result.replace(r#"href="/"#, &format!(r#"href="mbrfile://{}/"#, root_str));
    result = result.replace(
        r#"poster="/"#,
        &format!(r#"poster="mbrfile://{}/"#, root_str),
    );

    // Handle single-quoted attributes
    result = result.replace(r#"src='/"#, &format!(r#"src='mbrfile://{}/"#, root_str));
    result = result.replace(r#"href='/"#, &format!(r#"href='mbrfile://{}/"#, root_str));
    result = result.replace(
        r#"poster='/"#,
        &format!(r#"poster='mbrfile://{}/"#, root_str),
    );

    result
}

/// Render the QuickLook HTML template with inlined assets.
fn render_quicklook_template(
    markdown_html: &str,
    frontmatter: HashMap<String, String>,
    headings: Vec<markdown::HeadingInfo>,
    root_path: &Path,
    base_url: &str,
    config: &QuickLookConfig,
) -> Result<String, QuickLookError> {
    // Convert root-relative URLs to absolute file:// URLs for QuickLook
    let markdown_html = convert_root_relative_urls(markdown_html, root_path);

    // Load custom theme CSS if available
    let custom_theme = load_custom_theme(root_path);
    let custom_user_css = load_custom_user_css(root_path);

    // Build inline CSS
    let inline_css = build_inline_css(config, &custom_theme, &custom_user_css);

    // Build inline JavaScript
    let inline_js = build_inline_js(config);

    // Create Tera template engine with QuickLook template
    let mut tera = Tera::default();
    tera.add_raw_template("quicklook.html", QUICKLOOK_TEMPLATE)
        .map_err(|e| QuickLookError::TemplateRenderError {
            message: e.to_string(),
        })?;

    // Build template context
    let mut context = Context::new();

    // Add frontmatter fields
    for (k, v) in &frontmatter {
        context.insert(k, v);
    }

    // Add frontmatter as JSON
    let frontmatter_json = serde_json::to_string(&frontmatter).unwrap_or_else(|_| "{}".to_string());
    context.insert("frontmatter_json", &frontmatter_json);

    // Add headings for table of contents (as actual vector, not JSON string)
    context.insert("headings", &headings);

    // Add main content
    context.insert("markdown", &markdown_html);
    context.insert("inline_css", &inline_css);
    context.insert("inline_js", &inline_js);
    context.insert("base_url", &base_url);

    // Render template
    tera.render("quicklook.html", &context)
        .map_err(|e| QuickLookError::TemplateRenderError {
            message: e.to_string(),
        })
}

/// Load custom theme.css from .mbr/ folder if it exists.
fn load_custom_theme(root_path: &Path) -> Option<String> {
    let theme_path = root_path.join(".mbr/theme.css");
    std::fs::read_to_string(theme_path).ok()
}

/// Load custom user.css from .mbr/ folder if it exists.
fn load_custom_user_css(root_path: &Path) -> Option<String> {
    let user_css_path = root_path.join(".mbr/user.css");
    std::fs::read_to_string(user_css_path).ok()
}

/// Build the inline CSS string from embedded and custom sources.
fn build_inline_css(
    config: &QuickLookConfig,
    custom_theme: &Option<String>,
    custom_user_css: &Option<String>,
) -> String {
    let mut css = String::with_capacity(64 * 1024); // Pre-allocate for performance

    // Base CSS (pico.min.css) - use default theme for QuickLook
    if let Some(pico_css) = embedded_pico::get_pico_css("default") {
        if let Ok(pico_str) = std::str::from_utf8(pico_css) {
            css.push_str(pico_str);
            css.push('\n');
        }
    }

    // Theme CSS (custom or default)
    if let Some(custom) = custom_theme {
        css.push_str(custom);
    } else {
        css.push_str(get_embedded_file("/theme.css"));
    }
    css.push('\n');

    // User CSS
    if let Some(custom) = custom_user_css {
        css.push_str(custom);
        css.push('\n');
    }

    // Syntax highlighting CSS - use embedded_hljs module
    if config.include_syntax_highlighting {
        if let Ok(hljs_css) = std::str::from_utf8(embedded_hljs::HLJS_DARK_CSS) {
            css.push_str(hljs_css);
            css.push('\n');
        }
    }

    // VidStack CSS for video playback
    if config.include_vidstack {
        css.push_str(get_embedded_file("/vidstack.player.css"));
        css.push('\n');
        css.push_str(get_embedded_file("/vidstack.plyr.css"));
        css.push('\n');
    }

    // QuickLook-specific overrides
    css.push_str(QUICKLOOK_CSS);

    css
}

/// Build the inline JavaScript string.
fn build_inline_js(config: &QuickLookConfig) -> String {
    let mut js = String::with_capacity(512 * 1024); // Pre-allocate for large JS files

    // Syntax highlighting - use embedded_hljs module
    if config.include_syntax_highlighting {
        if let Ok(hljs_js) = std::str::from_utf8(embedded_hljs::HLJS_JS) {
            js.push_str(hljs_js);
            js.push('\n');
        }

        // Language packs from embedded_hljs
        let lang_modules: &[&[u8]] = &[
            embedded_hljs::HLJS_LANG_BASH,
            embedded_hljs::HLJS_LANG_CSS,
            embedded_hljs::HLJS_LANG_DOCKERFILE,
            embedded_hljs::HLJS_LANG_GO,
            embedded_hljs::HLJS_LANG_JAVA,
            embedded_hljs::HLJS_LANG_JAVASCRIPT,
            embedded_hljs::HLJS_LANG_JSON,
            embedded_hljs::HLJS_LANG_MARKDOWN,
            embedded_hljs::HLJS_LANG_NIX,
            embedded_hljs::HLJS_LANG_PYTHON,
            embedded_hljs::HLJS_LANG_RUBY,
            embedded_hljs::HLJS_LANG_RUST,
            embedded_hljs::HLJS_LANG_SCALA,
            embedded_hljs::HLJS_LANG_SQL,
            embedded_hljs::HLJS_LANG_TYPESCRIPT,
            embedded_hljs::HLJS_LANG_XML,
            embedded_hljs::HLJS_LANG_YAML,
        ];
        for lang_bytes in lang_modules {
            if let Ok(lang_js) = std::str::from_utf8(lang_bytes) {
                js.push_str(lang_js);
                js.push('\n');
            }
        }
    }

    // Mermaid diagrams
    if config.include_mermaid {
        js.push_str(get_embedded_file("/mermaid.min.js"));
        js.push('\n');
    }

    // VidStack video player
    if config.include_vidstack {
        js.push_str(get_embedded_file("/vidstack.player.js"));
        js.push('\n');
        js.push_str(get_embedded_file("/vid.js"));
        js.push('\n');
    }

    // QuickLook-specific initialization
    js.push_str(QUICKLOOK_JS);

    js
}

/// Get content of an embedded file by path.
fn get_embedded_file(path: &str) -> &'static str {
    for (name, content, _mime) in DEFAULT_FILES.iter() {
        if *name == path {
            return std::str::from_utf8(content).unwrap_or("");
        }
    }
    ""
}

/// QuickLook-specific CSS overrides.
const QUICKLOOK_CSS: &str = r##"
/* QuickLook-specific styles */

/* Hide navigation elements */
.browse-trigger,
mbr-browse,
mbr-search,
mbr-nav,
.breadcrumbs {
    display: none !important;
}

/* Disable non-anchor link clicks visually */
a[href]:not([href^="#"]) {
    cursor: default;
    text-decoration: underline;
}

/* Prevent text selection issues in QuickLook */
body {
    -webkit-user-select: text;
    user-select: text;
}

/* Ensure good contrast in both light and dark modes */
@media (prefers-color-scheme: dark) {
    :root {
        --pico-background-color: #1a1a2e;
    }
}

/* Info panel adjustments for QuickLook */
.info-panel {
    max-height: 50vh;
    overflow-y: auto;
}
"##;

/// QuickLook-specific JavaScript for initialization.
const QUICKLOOK_JS: &str = r##"
// QuickLook-specific initialization
document.addEventListener('DOMContentLoaded', function() {
    // Initialize syntax highlighting
    if (typeof hljs !== 'undefined') {
        hljs.highlightAll();
    }

    // Initialize mermaid diagrams
    if (typeof mermaid !== 'undefined') {
        mermaid.initialize({
            startOnLoad: true,
            theme: window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'default'
        });
    }

    // Intercept link clicks - only allow anchor links
    document.addEventListener('click', function(e) {
        const link = e.target.closest('a');
        if (link) {
            const href = link.getAttribute('href');
            if (href && !href.startsWith('#')) {
                // Prevent navigation for non-anchor links
                // In a real QuickLook extension, external links would open in browser
                // via webkit message handler
                e.preventDefault();
                e.stopPropagation();
            }
        }
    }, true);
});
"##;

/// QuickLook HTML template with all assets inlined.
const QUICKLOOK_TEMPLATE: &str = r##"<!doctype html>
<html lang="en">
<head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta name="color-scheme" content="light dark" />
    <base href="{{ base_url }}" />
    <title>{{ title | default(value="Preview") }}</title>
    <style>
{{ inline_css | safe }}
    </style>
</head>
<body>
    <header class="container">
        <nav role="navigation" aria-label="Main menu">
            <ul></ul>
            <ul>
                <li><strong>{% if title %}{{ title }}{% endif %}</strong></li>
            </ul>
            <ul>
                <li>
                    <label for="info-panel-toggle" class="info-trigger" aria-label="Open info panel">
                        <span class="info-icon">ℹ</span>
                    </label>
                </li>
            </ul>
        </nav>
    </header>
    <main id="wrapper" class="container">{{ markdown | safe }}</main>
    <input type="checkbox" id="info-panel-toggle" hidden />
    <aside class="info-panel">
        <label for="info-panel-toggle" class="info-panel-close" aria-label="Close info panel">×</label>
        <h3>Document Info</h3>
        {% if title %}<p><strong>Title:</strong> {{ title }}</p>{% endif %}
        {% if description %}<p><strong>Description:</strong> {{ description }}</p>{% endif %}
        {% if date %}<p><strong>Date:</strong> {{ date }}</p>{% endif %}
        {% if tags %}<p><strong>Tags:</strong> {{ tags }}</p>{% endif %}
        {% if headings %}
        <h4>Table of Contents</h4>
        <nav class="toc">
            <ul>
            {% for heading in headings %}
                <li class="toc-h{{ heading.level }}">
                    <a href="#{{ heading.id }}">{{ heading.text }}</a>
                </li>
            {% endfor %}
            </ul>
        </nav>
        {% endif %}
    </aside>
    <script>
{{ inline_js | safe }}
    </script>
</body>
</html>
"##;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_render_simple_markdown() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "# Hello World\n\nThis is a test.").unwrap();
        let path = file.path().to_str().unwrap().to_string();

        let html = render_preview(path, None).unwrap();

        assert!(html.contains("Hello World"));
        assert!(html.contains("This is a test"));
        assert!(html.contains("<!doctype html>"));
        assert!(html.contains("<style>"));
        assert!(html.contains("<script>"));
    }

    #[test]
    fn test_render_with_frontmatter() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "---\ntitle: Test Title\ndescription: A test document\n---\n\n# Content"
        )
        .unwrap();
        let path = file.path().to_str().unwrap().to_string();

        let html = render_preview(path, None).unwrap();

        assert!(html.contains("Test Title"));
        assert!(html.contains("A test document"));
    }

    #[test]
    fn test_render_with_code_block() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "```rust\nfn main() {{}}\n```").unwrap();
        let path = file.path().to_str().unwrap().to_string();

        let html = render_preview(path, None).unwrap();

        // Should include syntax highlighting CSS
        assert!(html.contains("hljs"));
        assert!(html.contains("fn main()"));
    }

    #[test]
    fn test_file_not_found() {
        let result = render_preview("/nonexistent/file.md".to_string(), None);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            QuickLookError::FileReadError { .. }
        ));
    }

    #[test]
    fn test_find_config_root() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mbr_dir = temp_dir.path().join(".mbr");
        std::fs::create_dir(&mbr_dir).unwrap();

        let subdir = temp_dir.path().join("docs");
        std::fs::create_dir(&subdir).unwrap();

        let file_path = subdir.join("test.md");
        std::fs::write(&file_path, "# Test").unwrap();

        let found_root = find_config_root(&file_path);
        assert_eq!(found_root, Some(temp_dir.path().to_path_buf()));
    }

    #[test]
    fn test_quicklook_css_includes_overrides() {
        let config = QuickLookConfig::default();
        let css = build_inline_css(&config, &None, &None);

        // Should include QuickLook-specific overrides
        assert!(css.contains("browse-trigger"));
        assert!(css.contains("display: none"));
    }

    #[test]
    fn test_quicklook_js_includes_initialization() {
        let config = QuickLookConfig::default();
        let js = build_inline_js(&config);

        // Should include initialization code
        assert!(js.contains("DOMContentLoaded"));
        assert!(js.contains("hljs.highlightAll"));
    }

    #[test]
    fn test_minimal_config() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "# Simple").unwrap();
        let path = file.path().to_str().unwrap().to_string();

        let config = QuickLookConfig {
            include_syntax_highlighting: false,
            include_mermaid: false,
            include_vidstack: false,
            base_url: None,
        };

        let html = render_preview_with_config(path, None, config).unwrap();

        // Should still render but without extras
        assert!(html.contains("Simple"));
        // Should NOT contain the actual hljs library code (pattern from hljs.js)
        // The init code that references hljs is guarded and always present
        assert!(!html.contains("registerLanguage"));
    }

    #[test]
    fn test_convert_root_relative_urls_double_quotes() {
        let html = r#"<img src="/images/test.png" alt="test">"#;
        let root = Path::new("/Users/test/notes");
        let result = convert_root_relative_urls(html, root);
        assert_eq!(
            result,
            r#"<img src="mbrfile:///Users/test/notes/images/test.png" alt="test">"#
        );
    }

    #[test]
    fn test_convert_root_relative_urls_single_quotes() {
        let html = r#"<source src='/videos/test.mp4' type="video/mp4">"#;
        let root = Path::new("/Users/test/notes");
        let result = convert_root_relative_urls(html, root);
        assert_eq!(
            result,
            r#"<source src='mbrfile:///Users/test/notes/videos/test.mp4' type="video/mp4">"#
        );
    }

    #[test]
    fn test_convert_root_relative_urls_href() {
        let html = r#"<a href="/docs/readme.md">Link</a>"#;
        let root = Path::new("/Users/test/notes");
        let result = convert_root_relative_urls(html, root);
        assert_eq!(
            result,
            r#"<a href="mbrfile:///Users/test/notes/docs/readme.md">Link</a>"#
        );
    }

    #[test]
    fn test_convert_root_relative_urls_poster() {
        let html = r#"<video poster="/images/thumb.jpg"></video>"#;
        let root = Path::new("/Users/test/notes");
        let result = convert_root_relative_urls(html, root);
        assert_eq!(
            result,
            r#"<video poster="mbrfile:///Users/test/notes/images/thumb.jpg"></video>"#
        );
    }

    #[test]
    fn test_convert_root_relative_urls_preserves_relative() {
        // Relative paths (not starting with /) should NOT be converted
        let html = r#"<img src="./images/test.png" alt="test">"#;
        let root = Path::new("/Users/test/notes");
        let result = convert_root_relative_urls(html, root);
        // Should remain unchanged
        assert_eq!(result, r#"<img src="./images/test.png" alt="test">"#);
    }

    #[test]
    fn test_convert_root_relative_urls_preserves_http() {
        // HTTP URLs should NOT be converted
        let html = r#"<img src="https://example.com/image.png">"#;
        let root = Path::new("/Users/test/notes");
        let result = convert_root_relative_urls(html, root);
        assert_eq!(result, r#"<img src="https://example.com/image.png">"#);
    }

    #[test]
    fn test_render_with_vid_shortcode() {
        // Create temp dir with .mbr folder and videos folder
        let temp_dir = tempfile::tempdir().unwrap();
        let mbr_dir = temp_dir.path().join(".mbr");
        std::fs::create_dir(&mbr_dir).unwrap();

        let videos_dir = temp_dir.path().join("videos");
        std::fs::create_dir(&videos_dir).unwrap();

        // Create a dummy video file
        std::fs::write(videos_dir.join("test.mp4"), b"dummy video").unwrap();

        // Create markdown with vid shortcode
        let file_path = temp_dir.path().join("test.md");
        std::fs::write(
            &file_path,
            r#"# Video Test

{{ vid(path="test.mp4", caption="Test") }}
"#,
        )
        .unwrap();

        let path = file_path.to_str().unwrap().to_string();
        let html = render_preview(path, None).unwrap();

        // The vid shortcode should generate /videos/test.mp4 which should be converted
        // to mbrfile:// URLs
        eprintln!("\n=== Generated HTML for video sections ===");
        for line in html.lines() {
            if line.contains("video")
                || line.contains("source")
                || line.contains("/videos")
                || line.contains("mbrfile")
                || line.contains("poster")
            {
                eprintln!("{}", line);
            }
        }
        eprintln!("=== End HTML ===\n");

        // Verify mbrfile:// URLs are present
        assert!(
            html.contains("mbrfile://"),
            "HTML should contain mbrfile:// URLs for video sources"
        );
    }
}
