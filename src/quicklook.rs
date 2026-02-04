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

/// Pre-allocation size for inline CSS string (64 KB).
const CSS_PREALLOC_BYTES: usize = 64 * 1024;

/// Pre-allocation size for inline JS string (512 KB).
const JS_PREALLOC_BYTES: usize = 512 * 1024;

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
    /// Base URL for converting relative paths (typically file:// URL of containing directory)
    pub base_url: Option<String>,
}

impl Default for QuickLookConfig {
    fn default() -> Self {
        Self {
            include_syntax_highlighting: true,
            include_mermaid: true,
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

    // QuickLook mode: server_mode=false, transcode disabled (transcode is server-only)
    // Use empty tag sources for QuickLook (no wikilink transformation)
    let render_result = rt
        .block_on(async {
            markdown::render(
                path.clone(),
                &root_path,
                0,
                link_config,
                false,                            // server_mode is false in QuickLook
                false,                            // transcode is disabled in QuickLook
                std::collections::HashSet::new(), // No tag sources in QuickLook
            )
            .await
        })
        .map_err(|e| QuickLookError::MarkdownRenderError {
            message: e.to_string(),
        })?;
    let frontmatter = render_result.frontmatter;
    let headings = render_result.headings;
    let html = render_result.html;

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
        &config,
    )
}

/// Search upward from the given path to find a repository root.
///
/// Searches for the same markers as Config::find_root_dir() to ensure
/// consistent root detection between QuickLook and server modes.
fn find_config_root(path: &Path) -> Option<PathBuf> {
    // Directory markers (same as Config::find_root_dir)
    const DIR_MARKERS: &[&str] = &[".mbr", ".git", ".zk", ".obsidian"];
    // File markers (same as Config::find_root_dir)
    const FILE_MARKERS: &[&str] = &["book.toml", "mkdocs.yml", "docusaurus.config.js"];

    let start_dir = if path.is_file() { path.parent()? } else { path };

    // Search for directory markers
    for marker in DIR_MARKERS {
        if let Some(root) = search_folder_in_ancestors(start_dir, marker) {
            return Some(root);
        }
    }

    // Search for file markers
    for marker in FILE_MARKERS {
        if let Some(root) = search_file_in_ancestors(start_dir, marker) {
            return Some(root);
        }
    }

    None
}

fn search_folder_in_ancestors(start_dir: &Path, folder_name: &str) -> Option<PathBuf> {
    start_dir
        .ancestors()
        .find(|ancestor| ancestor.join(folder_name).is_dir())
        .map(|p| p.to_path_buf())
}

fn search_file_in_ancestors(start_dir: &Path, file_name: &str) -> Option<PathBuf> {
    start_dir
        .ancestors()
        .find(|ancestor| ancestor.join(file_name).is_file())
        .map(|p| p.to_path_buf())
}

/// Resolve an asset path, checking the direct path first, then falling back to static folder.
///
/// This mirrors the logic in `path_resolver.rs` for consistent behavior between
/// server mode and QuickLook previews.
fn resolve_asset_path(root_path: &Path, static_folder: &str, url_path: &str) -> PathBuf {
    // url_path is like "/images/photo.jpg" - remove leading slash for join
    let relative_path = url_path.trim_start_matches('/');

    // Check direct path first
    let direct = root_path.join(relative_path);
    if direct.exists() {
        return direct;
    }

    // Fallback to static folder
    if !static_folder.is_empty() {
        let static_path = root_path.join(static_folder).join(relative_path);
        if static_path.exists() {
            return static_path;
        }
    }

    // Neither exists - return direct path (will 404, but that's expected)
    direct
}

/// Convert root-relative URLs (starting with /) to mbrfile:// URLs.
/// This is necessary because WKWebView's loadHTMLString() cannot access file:// URLs.
/// The Swift side registers a WKURLSchemeHandler for the mbrfile:// scheme that
/// serves local files from disk.
///
/// Uses the same fallback logic as the server: checks the direct path first,
/// then falls back to the static folder if configured.
fn convert_root_relative_urls(html: &str, root_path: &Path, static_folder: &str) -> String {
    use regex::Regex;

    // Match src="/.../", href="/.../", poster="/.../..." with double quotes
    // We use two separate patterns since Rust's regex crate doesn't support backreferences
    let re_double = Regex::new(r#"(src|href|poster)="(/[^"]*)""#).unwrap();
    let re_single = Regex::new(r#"(src|href|poster)='(/[^']*)'"#).unwrap();

    // First pass: handle double-quoted attributes
    let result = re_double.replace_all(html, |caps: &regex::Captures| {
        let attr = &caps[1];
        let url_path = &caps[2];

        // Resolve the path with static folder fallback
        let resolved = resolve_asset_path(root_path, static_folder, url_path);
        let resolved_str = resolved.to_str().unwrap_or(url_path);

        format!("{}=\"mbrfile://{}\"", attr, resolved_str)
    });

    // Second pass: handle single-quoted attributes
    re_single
        .replace_all(&result, |caps: &regex::Captures| {
            let attr = &caps[1];
            let url_path = &caps[2];

            // Resolve the path with static folder fallback
            let resolved = resolve_asset_path(root_path, static_folder, url_path);
            let resolved_str = resolved.to_str().unwrap_or(url_path);

            format!("{}='mbrfile://{}'", attr, resolved_str)
        })
        .to_string()
}

/// Render the QuickLook HTML template with inlined assets.
fn render_quicklook_template(
    markdown_html: &str,
    frontmatter: HashMap<String, serde_json::Value>,
    headings: Vec<markdown::HeadingInfo>,
    root_path: &Path,
    base_url: &str,
    ql_config: &QuickLookConfig,
    config: &Config,
) -> Result<String, QuickLookError> {
    // Convert root-relative URLs to absolute file:// URLs for QuickLook
    // Uses static_folder fallback logic to find files in the correct location
    let markdown_html = convert_root_relative_urls(markdown_html, root_path, &config.static_folder);

    // Load custom theme CSS if available
    let custom_theme = load_custom_theme(root_path);
    let custom_user_css = load_custom_user_css(root_path);

    // Build inline CSS using configured theme
    let inline_css = build_inline_css(ql_config, &config.theme, &custom_theme, &custom_user_css);

    // Build inline JavaScript
    let inline_js = build_inline_js(ql_config);

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
    theme: &str,
    custom_theme: &Option<String>,
    custom_user_css: &Option<String>,
) -> String {
    let mut css = String::with_capacity(CSS_PREALLOC_BYTES);

    // Base CSS (pico.min.css) - use configured theme
    if let Some(pico_css) = embedded_pico::get_pico_css(theme)
        && let Ok(pico_str) = std::str::from_utf8(pico_css)
    {
        css.push_str(pico_str);
        css.push('\n');
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
    if config.include_syntax_highlighting
        && let Ok(hljs_css) = std::str::from_utf8(embedded_hljs::HLJS_DARK_CSS)
    {
        css.push_str(hljs_css);
        css.push('\n');
    }

    // QuickLook-specific overrides
    css.push_str(QUICKLOOK_CSS);

    css
}

/// Build the inline JavaScript string.
fn build_inline_js(config: &QuickLookConfig) -> String {
    let mut js = String::with_capacity(JS_PREALLOC_BYTES);

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

/* Hide info panel - doesn't work in QuickLook context */
.info-trigger,
.info-panel,
#info-panel-toggle {
    display: none !important;
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
        let css = build_inline_css(&config, "default", &None, &None);

        // Should include QuickLook-specific overrides
        assert!(css.contains("browse-trigger"));
        assert!(css.contains("display: none"));
    }

    #[test]
    fn test_quicklook_uses_configured_theme() {
        // Test that theme from config is used
        let temp_dir = tempfile::tempdir().unwrap();
        let mbr_dir = temp_dir.path().join(".mbr");
        std::fs::create_dir(&mbr_dir).unwrap();

        // Create config with amber theme
        std::fs::write(mbr_dir.join("config.toml"), r#"theme = "amber""#).unwrap();

        // Create a simple markdown file
        let file_path = temp_dir.path().join("test.md");
        std::fs::write(&file_path, "# Test").unwrap();

        let path = file_path.to_str().unwrap().to_string();
        let html = render_preview(path, None).unwrap();

        // Amber theme should include amber-specific CSS
        // The pico amber theme includes specific amber color values
        // Check for presence of amber primary color (--pico-primary: #...)
        assert!(
            html.contains("amber") || html.contains("#ff8c00") || html.contains("pico.amber"),
            "Expected amber theme CSS. Got different theme."
        );
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
        let result = convert_root_relative_urls(html, root, "");
        assert_eq!(
            result,
            r#"<img src="mbrfile:///Users/test/notes/images/test.png" alt="test">"#
        );
    }

    #[test]
    fn test_convert_root_relative_urls_single_quotes() {
        let html = r#"<source src='/videos/test.mp4' type="video/mp4">"#;
        let root = Path::new("/Users/test/notes");
        let result = convert_root_relative_urls(html, root, "");
        assert_eq!(
            result,
            r#"<source src='mbrfile:///Users/test/notes/videos/test.mp4' type="video/mp4">"#
        );
    }

    #[test]
    fn test_convert_root_relative_urls_href() {
        let html = r#"<a href="/docs/readme.md">Link</a>"#;
        let root = Path::new("/Users/test/notes");
        let result = convert_root_relative_urls(html, root, "");
        assert_eq!(
            result,
            r#"<a href="mbrfile:///Users/test/notes/docs/readme.md">Link</a>"#
        );
    }

    #[test]
    fn test_convert_root_relative_urls_poster() {
        let html = r#"<video poster="/images/thumb.jpg"></video>"#;
        let root = Path::new("/Users/test/notes");
        let result = convert_root_relative_urls(html, root, "");
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
        let result = convert_root_relative_urls(html, root, "");
        // Should remain unchanged
        assert_eq!(result, r#"<img src="./images/test.png" alt="test">"#);
    }

    #[test]
    fn test_convert_root_relative_urls_preserves_http() {
        // HTTP URLs should NOT be converted
        let html = r#"<img src="https://example.com/image.png">"#;
        let root = Path::new("/Users/test/notes");
        let result = convert_root_relative_urls(html, root, "");
        assert_eq!(result, r#"<img src="https://example.com/image.png">"#);
    }

    #[test]
    fn test_convert_urls_static_folder_fallback() {
        // Test that static folder fallback works when file only exists there
        let temp_dir = tempfile::tempdir().unwrap();
        let static_images = temp_dir.path().join("static/images");
        std::fs::create_dir_all(&static_images).unwrap();
        std::fs::write(static_images.join("photo.jpg"), b"image data").unwrap();

        let html = r#"<img src="/images/photo.jpg">"#;
        let result = convert_root_relative_urls(html, temp_dir.path(), "static");

        // Should resolve to static/images/photo.jpg since /images/photo.jpg doesn't exist
        let expected_path = temp_dir.path().join("static/images/photo.jpg");
        assert!(
            result.contains(&format!("mbrfile://{}", expected_path.display())),
            "Expected URL to use static folder path. Got: {}",
            result
        );
    }

    #[test]
    fn test_convert_urls_direct_path_preferred() {
        // Test that direct path is preferred over static folder when both exist
        let temp_dir = tempfile::tempdir().unwrap();

        // Create file at direct path
        let direct_images = temp_dir.path().join("images");
        std::fs::create_dir_all(&direct_images).unwrap();
        std::fs::write(direct_images.join("photo.jpg"), b"direct image").unwrap();

        // Also create file in static folder
        let static_images = temp_dir.path().join("static/images");
        std::fs::create_dir_all(&static_images).unwrap();
        std::fs::write(static_images.join("photo.jpg"), b"static image").unwrap();

        let html = r#"<img src="/images/photo.jpg">"#;
        let result = convert_root_relative_urls(html, temp_dir.path(), "static");

        // Should resolve to direct path since it exists
        let expected_path = temp_dir.path().join("images/photo.jpg");
        assert!(
            result.contains(&format!("mbrfile://{}", expected_path.display())),
            "Expected URL to use direct path. Got: {}",
            result
        );
        // Should NOT use static folder path
        assert!(
            !result.contains("static/images"),
            "Should not use static folder when direct path exists"
        );
    }

    #[test]
    fn test_convert_urls_neither_exists() {
        // Test that direct path is used when file exists in neither location
        let temp_dir = tempfile::tempdir().unwrap();

        let html = r#"<img src="/images/missing.jpg">"#;
        let result = convert_root_relative_urls(html, temp_dir.path(), "static");

        // Should still use direct path (will 404, but that's expected)
        let expected_path = temp_dir.path().join("images/missing.jpg");
        assert!(
            result.contains(&format!("mbrfile://{}", expected_path.display())),
            "Expected URL to use direct path even when missing. Got: {}",
            result
        );
    }

    #[test]
    fn test_resolve_asset_path_direct_exists() {
        let temp_dir = tempfile::tempdir().unwrap();
        let images = temp_dir.path().join("images");
        std::fs::create_dir_all(&images).unwrap();
        std::fs::write(images.join("test.png"), b"data").unwrap();

        let result = resolve_asset_path(temp_dir.path(), "static", "/images/test.png");
        assert_eq!(result, temp_dir.path().join("images/test.png"));
    }

    #[test]
    fn test_resolve_asset_path_static_fallback() {
        let temp_dir = tempfile::tempdir().unwrap();
        let static_images = temp_dir.path().join("static/images");
        std::fs::create_dir_all(&static_images).unwrap();
        std::fs::write(static_images.join("test.png"), b"data").unwrap();

        let result = resolve_asset_path(temp_dir.path(), "static", "/images/test.png");
        assert_eq!(result, temp_dir.path().join("static/images/test.png"));
    }

    #[test]
    fn test_resolve_asset_path_neither_exists() {
        let temp_dir = tempfile::tempdir().unwrap();

        let result = resolve_asset_path(temp_dir.path(), "static", "/images/missing.png");
        // Should return direct path
        assert_eq!(result, temp_dir.path().join("images/missing.png"));
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

    #[test]
    fn test_render_preview_with_static_folder_image() {
        // Create temp dir with .mbr folder
        let temp_dir = tempfile::tempdir().unwrap();
        let mbr_dir = temp_dir.path().join(".mbr");
        std::fs::create_dir(&mbr_dir).unwrap();

        // Create image in static/images/blog/
        let static_images = temp_dir.path().join("static/images/blog");
        std::fs::create_dir_all(&static_images).unwrap();
        std::fs::write(static_images.join("test.png"), b"fake image data").unwrap();

        // Create markdown file with root-relative image reference
        let file_path = temp_dir.path().join("article.md");
        std::fs::write(
            &file_path,
            "# Test Article\n\n![caption](/images/blog/test.png)\n",
        )
        .unwrap();

        let path = file_path.to_str().unwrap().to_string();
        let html = render_preview(path, None).unwrap();

        // The image should resolve to static/images/blog/test.png
        let expected_static_path = temp_dir.path().join("static/images/blog/test.png");
        assert!(
            html.contains(&format!("mbrfile://{}", expected_static_path.display())),
            "Expected image to use static folder path.\nHTML excerpt: {}",
            html.lines()
                .filter(|l| l.contains("img") || l.contains("mbrfile") || l.contains("/images"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    #[test]
    fn test_render_preview_with_git_only_repo() {
        // Repo with .git but no .mbr - common case!
        let temp_dir = tempfile::tempdir().unwrap();
        let git_dir = temp_dir.path().join(".git");
        std::fs::create_dir(&git_dir).unwrap();

        // Create image in static folder
        let static_images = temp_dir.path().join("static/images");
        std::fs::create_dir_all(&static_images).unwrap();
        std::fs::write(static_images.join("photo.jpg"), b"image").unwrap();

        // Create markdown in subdirectory
        let docs_dir = temp_dir.path().join("docs");
        std::fs::create_dir(&docs_dir).unwrap();
        let file_path = docs_dir.join("readme.md");
        std::fs::write(&file_path, "![photo](/images/photo.jpg)").unwrap();

        let html = render_preview(file_path.to_str().unwrap().to_string(), None).unwrap();

        let expected = temp_dir.path().join("static/images/photo.jpg");
        assert!(
            html.contains(&format!("mbrfile://{}", expected.display())),
            "Should find static folder in .git-only repo"
        );
    }

    #[test]
    fn test_find_config_root_with_git_only() {
        // Test that find_config_root finds .git folders too
        let temp_dir = tempfile::tempdir().unwrap();
        let git_dir = temp_dir.path().join(".git");
        std::fs::create_dir(&git_dir).unwrap();

        let subdir = temp_dir.path().join("docs/nested");
        std::fs::create_dir_all(&subdir).unwrap();

        let file_path = subdir.join("test.md");
        std::fs::write(&file_path, "# Test").unwrap();

        let found_root = find_config_root(&file_path);
        assert_eq!(found_root, Some(temp_dir.path().to_path_buf()));
    }

    #[test]
    fn test_find_config_root_mbr_takes_precedence() {
        // When both .mbr and .git exist, .mbr should take precedence
        let temp_dir = tempfile::tempdir().unwrap();
        let mbr_dir = temp_dir.path().join(".mbr");
        let git_dir = temp_dir.path().join(".git");
        std::fs::create_dir(&mbr_dir).unwrap();
        std::fs::create_dir(&git_dir).unwrap();

        let file_path = temp_dir.path().join("test.md");
        std::fs::write(&file_path, "# Test").unwrap();

        let found_root = find_config_root(&file_path);
        assert_eq!(found_root, Some(temp_dir.path().to_path_buf()));
    }

    #[test]
    fn test_find_config_root_with_book_toml() {
        // Test file marker (book.toml for mdbook)
        let temp_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            temp_dir.path().join("book.toml"),
            "[book]\ntitle = \"Test\"",
        )
        .unwrap();

        let subdir = temp_dir.path().join("src");
        std::fs::create_dir(&subdir).unwrap();
        let file_path = subdir.join("SUMMARY.md");
        std::fs::write(&file_path, "# Summary").unwrap();

        let found_root = find_config_root(&file_path);
        assert_eq!(found_root, Some(temp_dir.path().to_path_buf()));
    }

    #[test]
    #[ignore] // Run with: cargo test --features ffi -- --ignored --nocapture test_debug_real_file
    fn test_debug_real_file() {
        let file_path = "/Users/pwalsh/src/icl/website.worktree/2026-01-28-test/src/routes/blog/2026/ai-coding-agents-drawing-the-line/+page.md";

        if !std::path::Path::new(file_path).exists() {
            eprintln!("File not found, skipping debug test");
            return;
        }

        // Check what root is found
        let path = std::path::PathBuf::from(file_path);
        let root = find_config_root(&path);
        eprintln!("\n=== Root found: {:?} ===", root);

        // Check config
        if let Some(ref root_path) = root {
            let config = crate::config::Config::read(root_path).unwrap_or_default();
            eprintln!("=== Config static_folder: {:?} ===", config.static_folder);

            // Check if static folder exists
            let static_path = root_path.join(&config.static_folder);
            eprintln!(
                "=== Static folder exists: {} at {:?} ===",
                static_path.exists(),
                static_path
            );
        }

        // Render and check output
        let html = render_preview(file_path.to_string(), None).unwrap();

        eprintln!("\n=== Image-related lines in HTML ===");
        for line in html.lines() {
            let line_lower = line.to_lowercase();
            if line_lower.contains("<img")
                || line_lower.contains("mbrfile")
                || line_lower.contains("/images/blog")
            {
                eprintln!("{}", line.trim());
            }
        }

        // Extract all src attributes
        eprintln!("\n=== All src= attributes ===");
        let re = regex::Regex::new(r#"src="([^"]+)""#).unwrap();
        for cap in re.captures_iter(&html) {
            let src = &cap[1];
            if src.contains("images") || src.contains("mbrfile") {
                eprintln!("src=\"{}\"", src);
            }
        }
    }
}
