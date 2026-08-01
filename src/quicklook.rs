//! QuickLook preview rendering module.
//!
//! This module provides functionality for rendering markdown files to self-contained HTML
//! suitable for display in macOS QuickLook previews. The generated HTML includes all CSS
//! and JavaScript inline, with navigation features disabled.
//!
//! This module is exposed via UniFFI for Swift interop in macOS QuickLook extensions.

use crate::config::{self, Config};
use crate::embedded_hljs;
use crate::embedded_pico;
use crate::link_transform::LinkTransformConfig;
use crate::markdown;
use crate::server::DEFAULT_FILES;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tera::{Context, Tera};
use thiserror::Error;

// Match src="/.../", href="/.../", poster="/.../..." attributes. Two separate
// patterns (double- and single-quoted) since Rust's regex crate doesn't
// support backreferences. Compiled once: these are literal patterns that
// cannot fail to compile.
static ROOT_RELATIVE_DOUBLE_QUOTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(src|href|poster)="(/[^"]*)""#)
        .expect("literal attribute regex is valid and cannot fail to compile")
});
static ROOT_RELATIVE_SINGLE_QUOTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(src|href|poster)='(/[^']*)'"#)
        .expect("literal attribute regex is valid and cannot fail to compile")
});

/// Pre-allocation size for inline CSS string (64 KB).
const CSS_PREALLOC_BYTES: usize = 64 * 1024;

/// Pre-allocation size for inline JS string (512 KB).
const JS_PREALLOC_BYTES: usize = 512 * 1024;

/// Most bytes read from a non-markdown file for a text preview (1 MiB).
///
/// QuickLook previews must feel instant and the whole document is inlined into
/// one HTML string, so an unbounded read would let a multi-gigabyte log file
/// stall Finder and blow up memory. Anything past this is dropped and the
/// preview says so. 1 MiB is ~15k lines of source - far more than anyone reads
/// in a spacebar preview.
const MAX_TEXT_PREVIEW_BYTES: usize = 1024 * 1024;

/// Largest text a preview will ask highlight.js to colorize (256 KiB).
///
/// Highlighting is regex-driven and runs in the WebView on the main thread; on
/// a file this size it already costs noticeably more than the rest of the
/// preview combined. Past the threshold the content still renders, just
/// verbatim, which is the same fallback used for unrecognized extensions.
const MAX_HIGHLIGHT_BYTES: usize = 256 * 1024;

/// Extensions always previewed as markdown, whatever the repo config says.
///
/// This is the same list `MBR.app` claims in its `Info.plist` (see the
/// `infoPlist` / `UTImportedTypeDeclarations` blocks in `flake.nix`) and the
/// two must stay in step: an extension the app registers for but that is not
/// here would open in mbr and then render as unparsed source.
///
/// It is a *union* with `config.markdown_extensions` rather than a fallback.
/// `markdown_extensions` defaults to just `["md"]` and its real job is
/// deciding which files become pages of the site; a single-file preview of a
/// `.mkdn` should still be rendered markdown even in a repo that would not
/// publish it.
const MARKDOWN_PREVIEW_EXTENSIONS: &[&str] = &[
    "markdown", "md", "mdoc", "mdown", "mdtext", "mdtxt", "mdwn", "mkd", "mkdn",
];

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

/// How a QuickLook preview should render a file.
///
/// The app registers as a viewer for `public.plain-text`, which is a supertype
/// of `public.source-code`, so QuickLook hands this module `.py`, `.sh`, `.log`
/// and friends - not just markdown. Deciding what to do with them is kept as a
/// pure function of the file name so it can be tested without touching disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewMode {
    /// Run the full markdown pipeline.
    Markdown,
    /// Show the bytes verbatim, colorized by highlight.js as this language.
    HighlightedText(&'static str),
    /// Show the bytes verbatim with no highlighting.
    PlainText,
}

/// Decide how `path` should be previewed.
///
/// Markdown wins if the extension is in `markdown_extensions` *or* in
/// [`MARKDOWN_PREVIEW_EXTENSIONS`]; otherwise a shipped highlight.js grammar is
/// used when one matches the extension, and everything else - including a file
/// with no extension at all - renders verbatim.
pub fn preview_mode_for(path: &Path, markdown_extensions: &[String]) -> PreviewMode {
    // No extension (`README`, `Makefile`, dotfiles): nothing to key off, so
    // verbatim. Guessing markdown here would mangle the very files QuickLook
    // is most likely to be showing from a source tree.
    let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
        return PreviewMode::PlainText;
    };

    let is_markdown = MARKDOWN_PREVIEW_EXTENSIONS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(extension))
        || markdown_extensions
            .iter()
            .any(|configured| configured.eq_ignore_ascii_case(extension));

    if is_markdown {
        return PreviewMode::Markdown;
    }

    match embedded_hljs::language_for_extension(extension) {
        Some(language) => PreviewMode::HighlightedText(language),
        None => PreviewMode::PlainText,
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
        config::find_root_dir(&path)
    };

    // Load config for markdown extensions
    let config = Config::read(&root_path).unwrap_or_default();

    // Non-markdown files must never reach the markdown parser: smart quotes,
    // emphasis and list markers would silently rewrite the user's source.
    match preview_mode_for(&path, &config.markdown_extensions) {
        PreviewMode::Markdown => {}
        mode => return render_text_preview(&path, mode, &root_path, &ql_config, &config),
    }

    // Determine if this is an index file (affects link transformation)
    let is_index_file = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| name == config.index_file);

    let link_config = LinkTransformConfig {
        markdown_extensions: config.markdown_extensions.clone(),
        index_file: config.index_file.clone(),
        is_index_file,
        url_depth: None,
        // QuickLook previews a single file with no repo index, so body
        // wikilinks never resolve globally; the page URL is unused.
        current_page_url: String::new(),
    };

    // Create a minimal tokio runtime for async markdown rendering
    // Note: oEmbed is disabled (timeout=0) for faster preview
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| QuickLookError::MarkdownRenderError {
            message: e.to_string(),
        })?;

    // QuickLook mode: server_mode=false, transcode disabled (transcode is server-only).
    // Use empty tag sources for QuickLook (no wikilink transformation) and
    // honor the user's mark_incomplete config (defaults to off in this preview).
    let mark_incomplete = config.mark_incomplete.unwrap_or(false);
    let incomplete_markers = config.incomplete_markers.clone();
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
                mark_incomplete,
                &incomplete_markers,
                None, // no repo wikilink index in QuickLook
            )
            .await
        })
        .map_err(|e| QuickLookError::MarkdownRenderError {
            message: e.to_string(),
        })?;
    let frontmatter = render_result.frontmatter;
    let headings = render_result.headings;
    let html = render_result.html;

    let base_url = preview_base_url(&ql_config, &root_path);

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

/// Base URL for relative asset resolution.
///
/// Uses the markdown repo root (not the file's own directory) so that
/// root-relative paths like `/videos/x.mp4` resolve the way they do on the
/// server.
fn preview_base_url(ql_config: &QuickLookConfig, root_path: &Path) -> String {
    ql_config.base_url.clone().unwrap_or_else(|| {
        root_path
            .to_str()
            .map(|s| format!("file://{}/", s))
            .unwrap_or_default()
    })
}

/// Read up to [`MAX_TEXT_PREVIEW_BYTES`] from `path`.
///
/// Returns the bytes and whether the file was longer than the cap. Uses
/// `Read::take` rather than reading the file and truncating, so a huge file is
/// never pulled into memory in the first place. One extra byte is requested so
/// "exactly at the cap" is distinguishable from "longer than the cap" without a
/// second `metadata()` syscall (and without the TOCTOU a stat-then-read has).
fn read_capped(path: &Path) -> Result<(Vec<u8>, bool), QuickLookError> {
    use std::io::Read;

    let file = std::fs::File::open(path).map_err(|e| QuickLookError::FileReadError {
        message: format!("{}: {}", path.display(), e),
    })?;

    let mut bytes = Vec::new();
    file.take(MAX_TEXT_PREVIEW_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| QuickLookError::FileReadError {
            message: format!("{}: {}", path.display(), e),
        })?;

    let truncated = bytes.len() > MAX_TEXT_PREVIEW_BYTES;
    bytes.truncate(MAX_TEXT_PREVIEW_BYTES);
    Ok((bytes, truncated))
}

/// Wrap verbatim text in a `<pre>` block for the preview template.
///
/// Escaping uses `encode_quoted_attribute` even though this is element text.
/// Its escape set (`& < > " '`) is a superset of what element text needs, and
/// the extra two matter: [`convert_root_relative_urls`] later runs regexes for
/// `src="/..."` / `src='/...'` over the rendered HTML, and a source file
/// containing that literal string would otherwise be rewritten mid-preview.
/// Escaping the quotes makes those regexes unable to match, so the file is
/// shown exactly as written. Browsers decode the entities back inside `<pre>`,
/// which is not a raw-text element, so nothing is visible to the reader.
fn text_to_pre_html(text: &str, mode: PreviewMode, truncated: bool) -> String {
    // `nohighlight` stops hljs.highlightAll() from language-guessing an
    // unlabelled block: auto-detection on prose or a log file produces
    // confident, wrong, and very colorful results.
    let code_class = match mode {
        PreviewMode::HighlightedText(language) if text.len() <= MAX_HIGHLIGHT_BYTES => {
            format!("language-{language}")
        }
        _ => "nohighlight".to_string(),
    };

    let escaped = html_escape::encode_quoted_attribute(text);
    let notice = if truncated {
        format!(
            "<p class=\"mbr-text-truncated\">Preview truncated at {} KB - open the file to see the rest.</p>",
            MAX_TEXT_PREVIEW_BYTES / 1024
        )
    } else {
        String::new()
    };

    format!(
        "<pre class=\"mbr-text-preview\"><code class=\"{code_class}\">{escaped}</code></pre>{notice}"
    )
}

/// Render a non-markdown file as a verbatim text preview.
///
/// Shares the template, theme loading and asset inlining with the markdown
/// path, so a text preview picks up the repo's `.mbr/theme.css` and the same
/// inlined highlight.js. Invalid UTF-8 is replaced rather than rejected: this
/// is a preview, and refusing to show a latin-1 log file would be worse than
/// showing it with a few replacement characters.
fn render_text_preview(
    path: &Path,
    mode: PreviewMode,
    root_path: &Path,
    ql_config: &QuickLookConfig,
    config: &Config,
) -> Result<String, QuickLookError> {
    let (bytes, truncated) = read_capped(path)?;
    let text = String::from_utf8_lossy(&bytes);
    let html = text_to_pre_html(&text, mode, truncated);

    // There is no frontmatter in a text file, so title the preview with the
    // file name - otherwise the template falls back to a bare "Preview".
    let mut frontmatter = markdown::SimpleMetadata::new();
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        frontmatter.insert("title".to_string(), serde_json::Value::String(name.into()));
    }

    let base_url = preview_base_url(ql_config, root_path);
    render_quicklook_template(
        &html,
        frontmatter,
        Vec::new(), // no headings: a text preview has no table of contents
        root_path,
        &base_url,
        ql_config,
        config,
    )
}

/// Find the repository/config root directory by searching upward for markers.
///
/// This is the UniFFI-exported wrapper around `config::find_root_dir()`.
/// It accepts and returns `String` for FFI compatibility with Swift.
pub fn find_config_root(file_path: String) -> String {
    let path = PathBuf::from(&file_path);
    config::find_root_dir(&path).to_string_lossy().into_owned()
}

/// Canonical form of `candidate`, but only if it exists and resolves inside
/// `canonical_root`.
///
/// `canonicalize` resolves `..` and every symlink, so its *result* is the only
/// trustworthy answer; containment is therefore checked on the resolved path and
/// never on the input string. `Path::starts_with` compares whole components, so
/// a sibling root like `/notes-evil` does not count as inside `/notes`.
///
/// This mirrors `safe_join` in `path_resolver.rs`, which guards the same class of
/// traversal for the server. QuickLook only ever rewrites URLs for files that
/// already exist, so unlike `safe_join` there is no "parent exists, leaf does
/// not" case to handle.
fn contained_canonical(canonical_root: &Path, candidate: &Path) -> Option<PathBuf> {
    let canonical = candidate.canonicalize().ok()?;
    canonical.starts_with(canonical_root).then_some(canonical)
}

/// Resolve a root-relative URL path to an on-disk asset, checking the direct path
/// first, then falling back to the static folder.
///
/// This mirrors the logic in `path_resolver.rs` for consistent behavior between
/// server mode and QuickLook previews.
///
/// # Security
///
/// `url_path` comes from attacker-controlled markdown. The returned path is always
/// the canonical path of a file that exists *inside* `canonical_root`; `..`
/// traversal, an absolute path smuggled in after the leading slash, and symlinks
/// pointing out of the repository all yield `None`. Callers must not emit an
/// `mbrfile://` URL when this returns `None`.
fn resolve_asset_path(
    canonical_root: &Path,
    static_folder: &str,
    url_path: &str,
) -> Option<PathBuf> {
    // url_path is like "/images/photo.jpg" - remove leading slashes for join
    let relative_path = url_path.trim_start_matches('/');

    // Check direct path first, then fall back to the static folder overlay.
    contained_canonical(canonical_root, &canonical_root.join(relative_path)).or_else(|| {
        if static_folder.is_empty() {
            return None;
        }
        contained_canonical(
            canonical_root,
            &canonical_root.join(static_folder).join(relative_path),
        )
    })
}

/// Rewrite one matched `src`/`href`/`poster` attribute into an `mbrfile://` URL.
///
/// Assets that do not resolve to an existing file inside the repository root keep
/// their original root-relative URL, which the WebView then simply fails to load.
fn rewrite_asset_attribute(
    caps: &regex::Captures<'_>,
    canonical_root: &Path,
    static_folder: &str,
    quote: char,
) -> String {
    let attr = &caps[1];
    let url_path = &caps[2];

    let target = resolve_asset_path(canonical_root, static_folder, url_path)
        .and_then(|resolved| resolved.to_str().map(|s| format!("mbrfile://{}", s)))
        .unwrap_or_else(|| url_path.to_string());

    format!("{}={}{}{}", attr, quote, target, quote)
}

/// Convert root-relative URLs (starting with /) to mbrfile:// URLs.
/// This is necessary because WKWebView's loadHTMLString() cannot access file:// URLs.
/// The Swift side registers a WKURLSchemeHandler for the mbrfile:// scheme that
/// serves local files from disk.
///
/// Uses the same fallback logic as the server: checks the direct path first,
/// then falls back to the static folder if configured.
///
/// # Security
///
/// Only assets that canonically live inside `root_path` are rewritten. If
/// `root_path` itself cannot be canonicalized there is nothing to contain against,
/// so the HTML is returned untouched rather than rewritten optimistically.
fn convert_root_relative_urls(html: &str, root_path: &Path, static_folder: &str) -> String {
    // Canonicalize the root once: every containment check compares against it,
    // and per-attribute canonicalization of the root would be wasted syscalls.
    let Ok(canonical_root) = root_path.canonicalize() else {
        return html.to_string();
    };

    // First pass: handle double-quoted attributes
    let result = ROOT_RELATIVE_DOUBLE_QUOTED.replace_all(html, |caps: &regex::Captures| {
        rewrite_asset_attribute(caps, &canonical_root, static_folder, '"')
    });

    // Second pass: handle single-quoted attributes
    ROOT_RELATIVE_SINGLE_QUOTED
        .replace_all(&result, |caps: &regex::Captures| {
            rewrite_asset_attribute(caps, &canonical_root, static_folder, '\'')
        })
        .to_string()
}

/// Render the QuickLook HTML template with inlined assets.
fn render_quicklook_template(
    markdown_html: &str,
    frontmatter: markdown::SimpleMetadata,
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

/* Verbatim text / source previews.
   `white-space: pre` (not pre-wrap) so a long line scrolls instead of being
   re-flowed: a wrapped line is a different file than the one on disk. */
pre.mbr-text-preview {
    white-space: pre;
    overflow-x: auto;
    tab-size: 4;
    -moz-tab-size: 4;
}

pre.mbr-text-preview code {
    white-space: inherit;
    font-family: var(--pico-font-family-monospace, ui-monospace, SFMono-Regular, Menlo, monospace);
}

.mbr-text-truncated {
    font-style: italic;
    opacity: 0.7;
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

    /// A temp file that ends in `.md`.
    ///
    /// `NamedTempFile::new()` produces `.tmpXXXXXX`, which has no extension at
    /// all, and since previews are now routed by extension that would be
    /// treated as plain text. Markdown tests must therefore name their file.
    fn markdown_temp_file() -> NamedTempFile {
        tempfile::Builder::new()
            .suffix(".md")
            .tempfile()
            .expect("temp file")
    }

    #[test]
    fn test_render_simple_markdown() {
        let mut file = markdown_temp_file();
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
        let mut file = markdown_temp_file();
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
        let mut file = markdown_temp_file();
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
    fn test_find_root_dir_with_mbr() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mbr_dir = temp_dir.path().join(".mbr");
        std::fs::create_dir(&mbr_dir).unwrap();

        let subdir = temp_dir.path().join("docs");
        std::fs::create_dir(&subdir).unwrap();

        let file_path = subdir.join("test.md");
        std::fs::write(&file_path, "# Test").unwrap();

        let found_root = config::find_root_dir(&file_path);
        assert_eq!(found_root, temp_dir.path().to_path_buf());
    }

    #[test]
    fn test_find_config_root_ffi_wrapper() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mbr_dir = temp_dir.path().join(".mbr");
        std::fs::create_dir(&mbr_dir).unwrap();

        let subdir = temp_dir.path().join("docs");
        std::fs::create_dir(&subdir).unwrap();

        let file_path = subdir.join("test.md");
        std::fs::write(&file_path, "# Test").unwrap();

        let result = find_config_root(file_path.to_str().unwrap().to_string());
        assert_eq!(result, temp_dir.path().to_str().unwrap());
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
        let mut file = markdown_temp_file();
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

    /// Creates a temp workspace laid out as:
    ///
    /// ```text
    /// <tmp>/          <- "outside" the repo; escape targets live here
    ///   repo/         <- the previewed repository root
    ///     <relative_file>
    /// ```
    ///
    /// Returns the tempdir (which must stay alive for the test) and the
    /// canonical repo root. The canonical root is what the rewriter emits, and
    /// on macOS it differs from `tempdir.path()`
    /// (`/var/folders/...` -> `/private/var/folders/...`).
    fn repo_with_file(relative_file: &str) -> (tempfile::TempDir, PathBuf) {
        let temp_dir = tempfile::tempdir().unwrap();
        let file = temp_dir.path().join("repo").join(relative_file);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"asset bytes").unwrap();
        let canonical_root = temp_dir.path().join("repo").canonicalize().unwrap();
        (temp_dir, canonical_root)
    }

    #[test]
    fn test_convert_root_relative_urls_double_quotes() {
        let (_tmp, root) = repo_with_file("images/test.png");
        let html = r#"<img src="/images/test.png" alt="test">"#;
        let result = convert_root_relative_urls(html, &root, "");
        assert_eq!(
            result,
            format!(
                r#"<img src="mbrfile://{}/images/test.png" alt="test">"#,
                root.display()
            )
        );
    }

    #[test]
    fn test_convert_root_relative_urls_single_quotes() {
        let (_tmp, root) = repo_with_file("videos/test.mp4");
        let html = r#"<source src='/videos/test.mp4' type="video/mp4">"#;
        let result = convert_root_relative_urls(html, &root, "");
        assert_eq!(
            result,
            format!(
                r#"<source src='mbrfile://{}/videos/test.mp4' type="video/mp4">"#,
                root.display()
            )
        );
    }

    #[test]
    fn test_convert_root_relative_urls_href() {
        let (_tmp, root) = repo_with_file("docs/readme.md");
        let html = r#"<a href="/docs/readme.md">Link</a>"#;
        let result = convert_root_relative_urls(html, &root, "");
        assert_eq!(
            result,
            format!(
                r#"<a href="mbrfile://{}/docs/readme.md">Link</a>"#,
                root.display()
            )
        );
    }

    #[test]
    fn test_convert_root_relative_urls_poster() {
        let (_tmp, root) = repo_with_file("images/thumb.jpg");
        let html = r#"<video poster="/images/thumb.jpg"></video>"#;
        let result = convert_root_relative_urls(html, &root, "");
        assert_eq!(
            result,
            format!(
                r#"<video poster="mbrfile://{}/images/thumb.jpg"></video>"#,
                root.display()
            )
        );
    }

    #[test]
    fn test_convert_root_relative_urls_unknown_root_rewrites_nothing() {
        // A root that cannot be canonicalized gives nothing to contain against,
        // so the HTML must be left alone rather than rewritten optimistically.
        let html = r#"<img src="/images/test.png">"#;
        let result = convert_root_relative_urls(html, Path::new("/no/such/root"), "static");
        assert_eq!(result, html);
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
        let expected_path = static_images.canonicalize().unwrap().join("photo.jpg");
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
        let expected_path = direct_images.canonicalize().unwrap().join("photo.jpg");
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
        // A URL that resolves to no file anywhere in the repo is left alone: an
        // mbrfile:// URL is a statement that the target is a contained, existing
        // asset, so one must not be minted for a path we never verified.
        let temp_dir = tempfile::tempdir().unwrap();

        let html = r#"<img src="/images/missing.jpg">"#;
        let result = convert_root_relative_urls(html, temp_dir.path(), "static");

        assert_eq!(result, html);
    }

    #[test]
    fn test_resolve_asset_path_direct_exists() {
        let (_tmp, root) = repo_with_file("images/test.png");

        let result = resolve_asset_path(&root, "static", "/images/test.png");
        assert_eq!(result, Some(root.join("images/test.png")));
    }

    #[test]
    fn test_resolve_asset_path_static_fallback() {
        let (_tmp, root) = repo_with_file("static/images/test.png");

        let result = resolve_asset_path(&root, "static", "/images/test.png");
        assert_eq!(result, Some(root.join("static/images/test.png")));
    }

    #[test]
    fn test_resolve_asset_path_neither_exists() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path().canonicalize().unwrap();

        assert_eq!(
            resolve_asset_path(&root, "static", "/images/missing.png"),
            None
        );
    }

    // MARK: containment regression tests
    //
    // Every case below is a way for attacker-authored markdown to name a file
    // outside the previewed repository. None of them may produce a path.

    #[test]
    fn test_resolve_asset_path_rejects_dotdot_traversal() {
        let (_tmp, root) = repo_with_file("images/ok.png");
        // The escape target must exist, or the test would pass for the wrong reason.
        let outside = root.parent().unwrap().join("outside-secret.txt");
        std::fs::write(&outside, b"secret").unwrap();

        assert_eq!(
            resolve_asset_path(&root, "static", "/../outside-secret.txt"),
            None
        );
        assert_eq!(
            resolve_asset_path(&root, "static", "/images/../../outside-secret.txt"),
            None
        );
        assert_eq!(
            resolve_asset_path(&root, "static", "/../../../../../../../etc/passwd"),
            None
        );
        assert!(
            outside.exists(),
            "escape target must exist for this to prove anything"
        );
    }

    #[test]
    fn test_resolve_asset_path_rejects_absolute_path() {
        let (_tmp, root) = repo_with_file("images/ok.png");

        // Extra leading slashes are stripped, so an absolute path smuggled in
        // this way must still be joined onto the root and land outside it.
        assert_eq!(resolve_asset_path(&root, "static", "//etc/passwd"), None);
        assert_eq!(resolve_asset_path(&root, "static", "/etc/passwd"), None);
    }

    #[test]
    #[cfg(unix)]
    fn test_resolve_asset_path_rejects_symlink_escaping_root() {
        let (_tmp, root) = repo_with_file("images/ok.png");

        // A secret outside the repo, and a symlink inside the repo aimed at it.
        let outside_dir = tempfile::tempdir().unwrap();
        let secret = outside_dir.path().join("id_rsa");
        std::fs::write(&secret, b"PRIVATE KEY").unwrap();

        std::os::unix::fs::symlink(&secret, root.join("images/leak.png")).unwrap();
        std::os::unix::fs::symlink(outside_dir.path(), root.join("escape")).unwrap();

        // Leaf symlink out of the repo.
        assert_eq!(
            resolve_asset_path(&root, "static", "/images/leak.png"),
            None
        );
        // Intermediate directory symlink out of the repo.
        assert_eq!(resolve_asset_path(&root, "static", "/escape/id_rsa"), None);
    }

    #[test]
    #[cfg(unix)]
    fn test_resolve_asset_path_rejects_sibling_root_prefix() {
        // `/notes-evil` must not count as inside `/notes` just because the
        // string starts the same way.
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("notes");
        let sibling = parent.path().join("notes-evil");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::write(sibling.join("secret.txt"), b"secret").unwrap();
        let root = root.canonicalize().unwrap();

        assert_eq!(
            resolve_asset_path(&root, "static", "/../notes-evil/secret.txt"),
            None
        );
    }

    #[test]
    fn test_resolve_asset_path_accepts_legitimate_sibling_image() {
        // The positive control for the tests above: an ordinary asset next to
        // the previewed document still resolves.
        let (_tmp, root) = repo_with_file("notes/diagram.png");

        assert_eq!(
            resolve_asset_path(&root, "static", "/notes/diagram.png"),
            Some(root.join("notes/diagram.png"))
        );
    }

    #[test]
    fn test_resolve_asset_path_static_fallback_cannot_escape() {
        // The static-folder fallback joins the same untrusted path a second
        // time, so it needs the same containment check as the direct path.
        // `/../../x` misses on the direct probe (<tmp>/../x) but the static
        // probe (<root>/static/../../x) lands on a real file outside the root.
        let (_tmp, root) = repo_with_file("static/images/ok.png");
        let outside = root.parent().unwrap().join("outside-static.txt");
        std::fs::write(&outside, b"secret").unwrap();

        assert_eq!(
            resolve_asset_path(&root, "static", "/../../outside-static.txt"),
            None
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_convert_root_relative_urls_does_not_mint_escaping_url() {
        // End-to-end: rendered HTML naming an escaping symlink keeps its
        // original URL, so no mbrfile:// URL pointing outside the repo is ever
        // handed to the WebView.
        let (_tmp, root) = repo_with_file("images/ok.png");
        let outside_dir = tempfile::tempdir().unwrap();
        let secret = outside_dir.path().join("id_rsa");
        std::fs::write(&secret, b"PRIVATE KEY").unwrap();
        std::os::unix::fs::symlink(&secret, root.join("images/leak.png")).unwrap();

        let html = r#"<img src="/images/leak.png"><img src='/../id_rsa'>"#;
        let result = convert_root_relative_urls(html, &root, "static");

        assert_eq!(result, html);
        assert!(!result.contains("mbrfile://"));
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

        // The image should resolve to static/images/blog/test.png (canonicalized:
        // the rewriter only emits paths it has proven are inside the root)
        let expected_static_path = static_images.canonicalize().unwrap().join("test.png");
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

        let expected = static_images.canonicalize().unwrap().join("photo.jpg");
        assert!(
            html.contains(&format!("mbrfile://{}", expected.display())),
            "Should find static folder in .git-only repo"
        );
    }

    #[test]
    fn test_find_root_dir_with_git_only() {
        // Test that find_root_dir finds .git folders too
        let temp_dir = tempfile::tempdir().unwrap();
        let git_dir = temp_dir.path().join(".git");
        std::fs::create_dir(&git_dir).unwrap();

        let subdir = temp_dir.path().join("docs/nested");
        std::fs::create_dir_all(&subdir).unwrap();

        let file_path = subdir.join("test.md");
        std::fs::write(&file_path, "# Test").unwrap();

        let found_root = config::find_root_dir(&file_path);
        assert_eq!(found_root, temp_dir.path().to_path_buf());
    }

    #[test]
    fn test_find_root_dir_mbr_takes_precedence() {
        // When both .mbr and .git exist, .mbr should take precedence
        let temp_dir = tempfile::tempdir().unwrap();
        let mbr_dir = temp_dir.path().join(".mbr");
        let git_dir = temp_dir.path().join(".git");
        std::fs::create_dir(&mbr_dir).unwrap();
        std::fs::create_dir(&git_dir).unwrap();

        let file_path = temp_dir.path().join("test.md");
        std::fs::write(&file_path, "# Test").unwrap();

        let found_root = config::find_root_dir(&file_path);
        assert_eq!(found_root, temp_dir.path().to_path_buf());
    }

    #[test]
    fn test_find_root_dir_with_book_toml() {
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

        let found_root = config::find_root_dir(&file_path);
        assert_eq!(found_root, temp_dir.path().to_path_buf());
    }

    // MARK: text preview mode selection

    #[test]
    fn test_preview_mode_markdown_extensions() {
        // Every extension MBR.app registers for must render as markdown, not
        // as source - including the ones the default config omits.
        let default_config = vec!["md".to_string()];
        for name in [
            "a.md",
            "a.markdown",
            "a.mkd",
            "a.mkdn",
            "a.mdown",
            "a.mdwn",
            "a.mdtxt",
            "a.mdtext",
            "a.mdoc",
            "A.MD",
            "Read.Me.Markdown",
        ] {
            assert_eq!(
                preview_mode_for(Path::new(name), &default_config),
                PreviewMode::Markdown,
                "{name} should preview as markdown"
            );
        }
    }

    #[test]
    fn test_preview_mode_honors_configured_extension() {
        // A repo that calls its pages ".page" gets them rendered.
        assert_eq!(
            preview_mode_for(Path::new("a.page"), &["page".to_string()]),
            PreviewMode::Markdown
        );
    }

    #[test]
    fn test_preview_mode_source_extensions_highlight() {
        for (name, language) in [
            ("main.rs", "rust"),
            ("setup.py", "python"),
            ("build.sh", "bash"),
            ("data.json", "json"),
            ("app.tsx", "typescript"),
            ("flake.nix", "nix"),
        ] {
            assert_eq!(
                preview_mode_for(Path::new(name), &["md".to_string()]),
                PreviewMode::HighlightedText(language),
                "{name} should highlight as {language}"
            );
        }
    }

    #[test]
    fn test_preview_mode_plain_for_unknown_and_extensionless() {
        // No extension, an extension we ship no grammar for, and a dotfile all
        // fall back to verbatim rather than being guessed at.
        for name in [
            "notes.txt",
            "server.log",
            "Makefile",
            "README",
            ".gitignore",
        ] {
            assert_eq!(
                preview_mode_for(Path::new(name), &["md".to_string()]),
                PreviewMode::PlainText,
                "{name} should preview as plain text"
            );
        }
    }

    // MARK: text preview rendering

    /// Writes `contents` to `<tmp>/<name>` and previews it.
    fn preview_file(name: &str, contents: &[u8]) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(name);
        std::fs::write(&path, contents).expect("write fixture");
        let html = render_preview(path.to_str().unwrap().to_string(), None).expect("render");
        (dir, html)
    }

    #[test]
    fn test_text_preview_does_not_parse_markdown() {
        // The whole point of the text path: markdown syntax in a .txt file is
        // content, not markup.
        let (_dir, html) = preview_file("notes.txt", b"# Not A Heading\n\n*not emphasis*\n");

        assert!(html.contains("<pre class=\"mbr-text-preview\">"));
        assert!(
            html.contains("# Not A Heading"),
            "hash must survive verbatim"
        );
        assert!(
            html.contains("*not emphasis*"),
            "asterisks must survive verbatim"
        );
        assert!(
            !html.contains("<em>not emphasis</em>"),
            "text preview must not run the markdown parser"
        );
    }

    #[test]
    fn test_text_preview_escapes_html() {
        let (_dir, html) = preview_file("notes.txt", b"<script>alert('x' & \"y\")</script>\n");

        assert!(
            !html.contains("<script>alert"),
            "raw tag from file content must not reach the document"
        );
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("&amp;"));
        assert!(html.contains("&quot;"));
        assert!(html.contains("&#x27;"));
    }

    #[test]
    fn test_text_preview_preserves_whitespace_exactly() {
        let source = "a\tb\n    four spaces\n\n\ttrailing tab\t\n";
        let (_dir, html) = preview_file("notes.txt", source.as_bytes());

        let body = html
            .split("<pre class=\"mbr-text-preview\"><code class=\"nohighlight\">")
            .nth(1)
            .and_then(|rest| rest.split("</code></pre>").next())
            .expect("pre block");
        assert_eq!(body, source, "bytes must round-trip through the preview");
    }

    #[test]
    fn test_text_preview_does_not_rewrite_urls_in_content() {
        // convert_root_relative_urls() runs over the rendered HTML. A source
        // file that merely mentions src="/..." must be shown, not rewritten.
        let (_dir, html) = preview_file("page.txt", b"<img src=\"/images/x.png\">\n");

        assert!(
            !html.contains("mbrfile://"),
            "file content must not be treated as a document reference"
        );
        // The quotes are escaped (which is what defeats the rewriter); the
        // slashes are not, so the reader still sees the original path.
        assert!(html.contains("&quot;/images/x.png&quot;"));
    }

    #[test]
    fn test_source_preview_gets_language_class() {
        let (_dir, html) = preview_file("main.rs", b"fn main() {}\n");

        assert!(html.contains("<code class=\"language-rust\">"));
        // The grammar must actually be inlined - QuickLook has no network.
        assert!(html.contains("hljs"));
    }

    #[test]
    fn test_plain_preview_opts_out_of_highlighting() {
        // Without nohighlight, hljs.highlightAll() language-guesses prose.
        let (_dir, html) = preview_file("notes.txt", b"just some prose\n");
        assert!(html.contains("<code class=\"nohighlight\">"));
    }

    #[test]
    fn test_text_preview_titles_with_file_name() {
        let (_dir, html) = preview_file("notes.txt", b"hi\n");
        assert!(html.contains("<title>notes.txt</title>"));
    }

    // MARK: hostile inputs
    //
    // public.plain-text is a broad supertype, so QuickLook will hand this
    // module whatever is on the user's disk. None of these may panic.

    /// Marker for the truncation notice.
    ///
    /// Matching on the bare class name would be meaningless: the inlined
    /// QuickLook CSS declares `.mbr-text-truncated`, so it is present in every
    /// preview. Only the opening tag is unique to the notice itself.
    const TRUNCATION_NOTICE: &str = "<p class=\"mbr-text-truncated\">";

    #[test]
    fn test_text_preview_invalid_utf8_is_lossy_not_fatal() {
        // Lone continuation bytes: invalid UTF-8 in any encoding-aware reader.
        let (_dir, html) = preview_file("latin1.txt", b"caf\xe9 na\xefve\n\xff\xfe");

        assert!(html.contains("<pre class=\"mbr-text-preview\">"));
        assert!(
            html.contains('\u{FFFD}'),
            "undecodable bytes should become replacement characters"
        );
        assert!(html.contains("caf"), "decodable text should survive");
    }

    #[test]
    fn test_text_preview_empty_file() {
        let (_dir, html) = preview_file("empty.txt", b"");

        assert!(html.contains("<pre class=\"mbr-text-preview\">"));
        assert!(html.contains("</code></pre>"));
        assert!(!html.contains(TRUNCATION_NOTICE));
    }

    #[test]
    fn test_text_preview_file_with_no_extension() {
        let (_dir, html) = preview_file("Makefile", b"all:\n\techo hi\n");

        assert!(html.contains("<code class=\"nohighlight\">"));
        assert!(html.contains("echo hi"));
    }

    #[test]
    fn test_text_preview_caps_oversized_file() {
        // One byte over the cap is the boundary that must trip truncation.
        let oversized = vec![b'x'; MAX_TEXT_PREVIEW_BYTES + 1];
        let (_dir, html) = preview_file("huge.txt", &oversized);

        assert!(
            html.contains(TRUNCATION_NOTICE),
            "oversized preview must say it was truncated"
        );
        let body = html
            .split("<code class=\"nohighlight\">")
            .nth(1)
            .and_then(|rest| rest.split("</code>").next())
            .expect("pre block");
        assert_eq!(
            body.len(),
            MAX_TEXT_PREVIEW_BYTES,
            "no more than the cap may be inlined"
        );
    }

    #[test]
    fn test_text_preview_at_exactly_the_cap_is_not_truncated() {
        let exact = vec![b'x'; MAX_TEXT_PREVIEW_BYTES];
        let (_dir, html) = preview_file("exact.txt", &exact);

        assert!(
            !html.contains(TRUNCATION_NOTICE),
            "a file exactly at the cap is complete"
        );
    }

    #[test]
    fn test_large_source_file_skips_highlighting() {
        // Past MAX_HIGHLIGHT_BYTES the content still renders, just verbatim,
        // so hljs cannot stall the preview.
        let big = "// comment\n".repeat(MAX_HIGHLIGHT_BYTES / 11 + 100);
        assert!(big.len() > MAX_HIGHLIGHT_BYTES);
        let html = text_to_pre_html(&big, PreviewMode::HighlightedText("rust"), false);

        assert!(html.contains("<code class=\"nohighlight\">"));
        assert!(!html.contains("language-rust"));
    }

    #[test]
    fn test_text_to_pre_html_truncation_notice_only_when_truncated() {
        assert!(!text_to_pre_html("x", PreviewMode::PlainText, false).contains("truncated"));
        assert!(text_to_pre_html("x", PreviewMode::PlainText, true).contains("truncated"));
    }

    #[test]
    fn test_read_capped_reports_truncation() {
        let dir = tempfile::tempdir().unwrap();

        let small = dir.path().join("small.txt");
        std::fs::write(&small, b"abc").unwrap();
        assert_eq!(read_capped(&small).unwrap(), (b"abc".to_vec(), false));

        let big = dir.path().join("big.txt");
        std::fs::write(&big, vec![b'z'; MAX_TEXT_PREVIEW_BYTES + 10]).unwrap();
        let (bytes, truncated) = read_capped(&big).unwrap();
        assert!(truncated);
        assert_eq!(bytes.len(), MAX_TEXT_PREVIEW_BYTES);
    }

    #[test]
    fn test_text_preview_missing_file_is_an_error() {
        let result = render_preview("/nonexistent/file.txt".to_string(), None);
        assert!(matches!(
            result.unwrap_err(),
            QuickLookError::FileReadError { .. }
        ));
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
        let root = config::find_root_dir(&path);
        eprintln!("\n=== Root found: {:?} ===", root);

        // Check config
        let config = crate::config::Config::read(&root).unwrap_or_default();
        eprintln!("=== Config static_folder: {:?} ===", config.static_folder);

        // Check if static folder exists
        let static_path = root.join(&config.static_folder);
        eprintln!(
            "=== Static folder exists: {} at {:?} ===",
            static_path.exists(),
            static_path
        );

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
