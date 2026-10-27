//! QuickLook preview rendering module.
//!
//! This module provides functionality for rendering markdown files to self-contained HTML
//! suitable for display in macOS QuickLook previews. The generated HTML includes all CSS
//! and JavaScript inline, with navigation features disabled.
//!
//! This module is exposed via UniFFI for Swift interop in macOS QuickLook extensions.
//!
//! # Local assets
//!
//! A preview is a *data-based* [`QLPreviewReply`][reply]: the extension hands
//! QuickLook one HTML document plus a dictionary of attachments, and QuickLook
//! renders it. There is no custom URL scheme and no `WKWebView` under the
//! extension's control, so the only local files a preview can show are the ones
//! named in that dictionary, addressed from the HTML as `cid:<id>`.
//!
//! That is why [`collect_preview_attachments`] is the security boundary now, and
//! a stronger one than the `mbrfile://` scheme handler it replaced: a `<script>`
//! in untrusted markdown could ask that handler for *any* path, whereas `cid:`
//! resolves against this list and nothing else. The list only ever contains
//! regular files the server would serve for the same URLs: canonically inside
//! the previewed repository or its configured static overlay (see
//! [`AssetResolver`]).
//!
//! [reply]: https://developer.apple.com/documentation/quicklook/qlpreviewreply

use mbr_core::assets;
use mbr_core::config::{self, Config};
use mbr_core::embedded_hljs;
use mbr_core::embedded_pico;
use mbr_core::link_transform::LinkTransformConfig;
use mbr_core::markdown;
use mbr_core::page_context::{self, UrlMode};
use mbr_core::path_resolver::{self, OwnedPathResolverConfig, ResolvedPath};
use mbr_core::relationships::{self, RelationTypeRegistry};
use mbr_core::repo;
use mbr_core::templates;
use mbr_core::url_path::{self, url_scheme};
use mbr_core::vault::VaultPath;
use percent_encoding::percent_decode_str;
use regex::Regex;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tera::{Context, Tera};
use thiserror::Error;

// Match every src="...", srcset="...", href="...", poster="..." attribute;
// which values name a local asset is decided per match by `asset_url_path`
// (per candidate, for a `srcset`). Two separate patterns (double- and
// single-quoted) since Rust's regex crate doesn't support backreferences.
// Compiled once: these are literal patterns that cannot fail to compile.
static ASSET_ATTRIBUTE_DOUBLE_QUOTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(srcset|src|href|poster)="([^"]*)""#)
        .expect("literal attribute regex is valid and cannot fail to compile")
});
static ASSET_ATTRIBUTE_SINGLE_QUOTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(srcset|src|href|poster)='([^']*)'"#)
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

/// Largest single local asset a preview will attach (16 MiB).
///
/// Attachments are *eager*: QuickLook wants the whole reply up front, so every
/// asset named by the page is read into memory before anything is displayed,
/// whether or not the reader ever scrolls to it. The `mbrfile://` scheme handler
/// this replaced was lazy, so the cap is new and is what keeps a note that
/// embeds a feature-length video from turning a spacebar preview into a
/// multi-gigabyte read. An asset past the cap keeps its original root-relative
/// URL and simply does not load - the same outcome as an asset that does not
/// exist.
const MAX_ATTACHMENT_BYTES: u64 = 16 * 1024 * 1024;

/// Largest total a preview's attachments may add up to (64 MiB).
///
/// Per-asset capping alone is not enough: a gallery page with two hundred
/// in-budget images is still a quarter-gigabyte reply. Assets are attached in
/// document order, so the ones nearest the top - the ones a preview actually
/// shows - are the ones that fit.
const MAX_TOTAL_ATTACHMENT_BYTES: u64 = 64 * 1024 * 1024;

/// Most markdown a thumbnail renders (32 KiB), cut back to a line boundary.
///
/// A thumbnail only ever shows the top of the page, but WebKit lays out the
/// whole document before it can snapshot any of it, and highlight.js colors
/// every code block in it. 32 KiB is ten-plus screens of prose - far more than
/// fits in the snapshot's one page - so the cap is invisible on the thumbnail
/// and keeps a 1 MB note as fast as a short one.
///
/// Cutting can leave a code fence or a frontmatter block unclosed. The first
/// only changes text below the visible page. The second needs a frontmatter
/// block longer than the cap, at which point the note has no usable cover
/// image either way.
const MAX_THUMBNAIL_SOURCE_BYTES: usize = 32 * 1024;

/// Frontmatter keys that may name a thumbnail's cover image, in priority order.
///
/// `cover` first because it says what it is for; `image` is the common
/// OpenGraph-style key (and a person note's portrait), so it is the fallback.
const COVER_IMAGE_KEYS: &[&str] = &["cover", "image"];

/// Extensions a cover image may have to be used as the thumbnail itself.
///
/// The Swift side hands the file to `QLThumbnailReply(imageFileURL:)`, which
/// decodes it with ImageIO, so this is the set ImageIO reads. SVG is absent on
/// purpose: ImageIO does not rasterize it, and an undecodable cover would leave
/// the file with no thumbnail at all instead of the rendered page.
const COVER_IMAGE_EXTENSIONS: &[&str] = &[
    "avif", "bmp", "gif", "heic", "heif", "jpeg", "jpg", "png", "tif", "tiff", "webp",
];

/// What thumbnails inline. Highlighting stays on so code looks the way it does
/// in the preview. Mermaid is off: inlining it makes the page ~5.7 MB instead
/// of ~380 KB, and WebKit parses all of it on every thumbnail. Measured on a
/// warm web view, that took a thumbnail from 6-16 ms to 38-145 ms - before
/// waiting for any diagram to finish rendering. A diagram shows as its source.
const THUMBNAIL_CONFIG: QuickLookConfig = QuickLookConfig {
    include_syntax_highlighting: true,
    include_mermaid: false,
};

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

/// One local file a preview needs, and the name the HTML refers to it by.
///
/// `id` is the key of a `QLPreviewReply.attachments` entry; the HTML addresses
/// it as `cid:{id}`. `path` is an absolute, canonical path to a regular file
/// inside the previewed repository - [`collect_preview_attachments`] has already
/// proved that, so the Swift side reads it without re-deriving containment.
///
/// The file's *content type* is deliberately absent: Swift asks the system for
/// it (`UTType(filenameExtension:)`), which knows more types than any table
/// either side of the FFI could carry, and keeps one from drifting from the
/// other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewAttachment {
    /// Attachment identifier, referenced from the HTML as `cid:{id}`.
    pub id: String,
    /// Absolute canonical path of the file to attach.
    pub path: String,
}

/// A rendered preview: one HTML document plus the local files it references.
///
/// This is the whole input to a data-based `QLPreviewReply`. The two halves must
/// travel together - the HTML's `cid:` URLs mean nothing without the list, and
/// the list means nothing without the HTML.
#[derive(Debug, Clone)]
pub struct PreviewDocument {
    /// Self-contained HTML, with local assets addressed as `cid:{id}`.
    pub html: String,
    /// Every local file the HTML refers to, in document order.
    pub attachments: Vec<PreviewAttachment>,
}

/// What a QuickLook thumbnail should show for a markdown file.
///
/// Exposed via UniFFI as an enum with associated values; the Swift thumbnail
/// extension turns `Image` into `QLThumbnailReply(imageFileURL:)` and `Page`
/// into a snapshot of the rendered HTML.
#[derive(Debug, Clone)]
pub enum ThumbnailDocument {
    /// The note's frontmatter cover image, used as the thumbnail directly.
    ///
    /// `path` is absolute, canonical, and already proven to be a regular image
    /// file inside the note's repository, no larger than
    /// [`MAX_ATTACHMENT_BYTES`] - see [`thumbnail_cover_image`].
    Image { path: String },
    /// The rendered page, built exactly like a preview (same HTML, inline CSS
    /// and attachments) from at most [`MAX_THUMBNAIL_SOURCE_BYTES`] of source.
    Page { document: PreviewDocument },
}

/// Configuration options for QuickLook rendering.
#[derive(Debug, Clone)]
pub struct QuickLookConfig {
    /// Whether to include syntax highlighting (increases HTML size significantly)
    pub include_syntax_highlighting: bool,
    /// Whether to include mermaid diagram support
    pub include_mermaid: bool,
}

impl Default for QuickLookConfig {
    fn default() -> Self {
        Self {
            include_syntax_highlighting: true,
            include_mermaid: true,
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

/// Render a markdown file to a self-contained QuickLook preview.
///
/// This function:
/// 1. Finds the `.mbr/` config folder (if present) for custom themes
/// 2. Parses markdown with frontmatter extraction
/// 3. Renders through a QuickLook-specific template
/// 4. Inlines all CSS and JavaScript for self-contained HTML
/// 5. Disables navigation features (search, browse, next/prev links)
/// 6. Turns root-relative asset URLs into `cid:` references plus attachments
///
/// # Arguments
///
/// * `file_path` - Path to the markdown file to render
/// * `config_root` - Optional path to the root directory containing `.mbr/` folder.
///   If None, searches upward from the file's directory.
///
/// # Returns
///
/// A [`PreviewDocument`] ready to become a `QLPreviewReply`.
pub fn render_preview(
    file_path: String,
    config_root: Option<String>,
) -> Result<PreviewDocument, QuickLookError> {
    render_preview_with_config(file_path, config_root, QuickLookConfig::default())
}

/// Render a markdown file with custom configuration options.
pub fn render_preview_with_config(
    file_path: String,
    config_root: Option<String>,
    ql_config: QuickLookConfig,
) -> Result<PreviewDocument, QuickLookError> {
    let target = QuickLookTarget::locate(&file_path, config_root)?;

    // Non-markdown files must never reach the markdown parser: smart quotes,
    // emphasis and list markers would silently rewrite the user's source.
    match preview_mode_for(&target.path, &target.config.markdown_extensions) {
        PreviewMode::Markdown => {}
        mode => {
            return render_text_preview(
                &target.path,
                mode,
                &target.root_path,
                &target.base_dir_url(),
                &ql_config,
                &target.config,
            );
        }
    }

    let source =
        std::fs::read_to_string(&target.path).map_err(|e| QuickLookError::FileReadError {
            message: format!("{}: {}", target.path.display(), e),
        })?;
    let mut rendered = target.render_markdown(&source)?;
    let card = target.contact_card_context(&source, &mut rendered);

    render_quicklook_template(
        &rendered.html,
        rendered.frontmatter,
        rendered.headings,
        card,
        &target.root_path,
        &target.base_dir_url(),
        &ql_config,
        &target.config,
    )
}

/// Render what a QuickLook *thumbnail* of a markdown file should show.
///
/// A frontmatter cover image (see [`COVER_IMAGE_KEYS`]) that names a local
/// image inside the repository wins and is returned as
/// [`ThumbnailDocument::Image`]. Otherwise the page is rendered exactly as the
/// preview renders it - same template, theme and attachments - from the first
/// [`MAX_THUMBNAIL_SOURCE_BYTES`] of the note, with [`THUMBNAIL_CONFIG`].
///
/// Only markdown is accepted. The thumbnail extension claims markdown types
/// alone, so the system keeps drawing its own thumbnails for source and text
/// files; anything else reaching this is an error rather than a `<pre>` page.
pub fn render_thumbnail(
    file_path: String,
    config_root: Option<String>,
) -> Result<ThumbnailDocument, QuickLookError> {
    let target = QuickLookTarget::locate(&file_path, config_root)?;

    if preview_mode_for(&target.path, &target.config.markdown_extensions) != PreviewMode::Markdown {
        return Err(QuickLookError::MarkdownRenderError {
            message: format!("Not a markdown file: {file_path}"),
        });
    }

    let (bytes, truncated) = read_capped(&target.path, MAX_THUMBNAIL_SOURCE_BYTES)?;
    let bytes = if truncated {
        cut_at_line_boundary(&bytes)
    } else {
        &bytes[..]
    };
    // Lossy for the same reason as a text preview: a thumbnail that refuses a
    // note over one bad byte is worse than one with a replacement character.
    let source = String::from_utf8_lossy(bytes);

    // The frontmatter comes out of the same render the page uses, so there is
    // one YAML parse and one definition of what the frontmatter says.
    let mut rendered = target.render_markdown(&source)?;

    if let Some(path) = AssetResolver::new(&target.root_path, &target.config).and_then(|resolver| {
        thumbnail_cover_image(&rendered.frontmatter, &resolver, &target.base_dir_url())
    }) {
        return Ok(ThumbnailDocument::Image { path });
    }

    let card = target.contact_card_context(&source, &mut rendered);
    render_quicklook_template(
        &rendered.html,
        rendered.frontmatter,
        rendered.headings,
        card,
        &target.root_path,
        &target.base_dir_url(),
        &THUMBNAIL_CONFIG,
        &target.config,
    )
    .map(|document| ThumbnailDocument::Page { document })
}

/// The file being previewed, the repository it belongs to, and its config.
///
/// Shared by previews and thumbnails so both find the root, read the config and
/// render markdown the same way.
struct QuickLookTarget {
    path: PathBuf,
    root_path: PathBuf,
    config: Config,
}

impl QuickLookTarget {
    /// Resolve `file_path` and its repository root (`config_root`, or the
    /// nearest marker above the file when absent) and read the root's config.
    fn locate(file_path: &str, config_root: Option<String>) -> Result<Self, QuickLookError> {
        let path = PathBuf::from(file_path);

        if !path.exists() {
            return Err(QuickLookError::FileReadError {
                message: format!("File not found: {}", file_path),
            });
        }

        // Find root directory (with .mbr folder) for custom themes
        let root_path = config_root
            .map(PathBuf::from)
            .unwrap_or_else(|| config::find_root_dir(&path));

        // Load config for markdown extensions
        let config = Config::read(&root_path).unwrap_or_default();

        Ok(Self {
            path,
            root_path,
            config,
        })
    }

    /// The URL of the folder holding this file, ending in `/` (`/notes/` for
    /// both `notes/note.md` and `notes/index.md`, `/` for a note at the root):
    /// what the rendered HTML's relative URLs are relative to.
    ///
    /// A preview has no served, trailing-slash page URL; it is the file in its
    /// folder. [`Self::render_markdown`] therefore writes markdown links without
    /// `link_transform`'s `../` compensation, so a relative URL, markdown or raw
    /// HTML alike, means what it means on GitHub or in an editor.
    ///
    /// Computed on canonical paths so a root reached through a symlink (macOS's
    /// `/tmp` -> `/private/tmp`) agrees with the note's own path.
    fn base_dir_url(&self) -> String {
        let canonical = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let note = canonical(&self.path);
        let root = canonical(&self.root_path);
        // A note outside its root (only possible with an explicit
        // `config_root`) falls back to the root itself; containment of what the
        // URLs then name is still judged by `AssetResolver`.
        let relative_dir = note
            .parent()
            .and_then(|dir| dir.strip_prefix(&root).ok())
            .unwrap_or(Path::new(""));
        let dir = url_path::path_to_url(relative_dir);
        if dir.is_empty() {
            "/".to_string()
        } else {
            format!("/{dir}/")
        }
    }

    /// The contact card's template context for a `type: person` /
    /// `type: organization` note (`contact`, `contact_backlinks`, `has_h1`),
    /// empty for every other note. Takes the parsed contact out of `rendered`.
    ///
    /// The same [`page_context::insert_contact`] the server and the static
    /// build call, with what a single-file preview can know:
    ///
    /// - Relationships are the note's **own** declarations (re-read from its
    ///   frontmatter only for a contact), framed by
    ///   [`relationships::standalone_relationships`]: no other note is known,
    ///   so every neighbour — and a wikilinked `company` — is plain text.
    ///   None with `relationship_tracking` off, as on the server.
    /// - `contact_backlinks` is `0`, which renders no chip: the interactive
    ///   `<mbr-contact-backlinks>` needs a server's `links.json`.
    /// - `has_h1` keeps the card's headline exactly as `index.html` does: the
    ///   card names the note unless the body opens with its own H1, so the
    ///   preview has one title, never two or none.
    fn contact_card_context(
        &self,
        source: &str,
        rendered: &mut markdown::MarkdownRenderResult,
    ) -> HashMap<String, serde_json::Value> {
        let mut context = HashMap::new();
        let Some(contact) = rendered.contact.take() else {
            return context;
        };
        let registry = RelationTypeRegistry::from_types(&self.config.relationship_types);
        let relationships = if self.config.relationship_tracking {
            let title = rendered
                .frontmatter
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let declared =
                markdown::extract_metadata_from_bytes(&self.path, source.as_bytes()).relationships;
            relationships::standalone_relationships(title, declared, &registry)
        } else {
            Vec::new()
        };
        page_context::insert_contact(
            &mut context,
            Some(contact),
            &relationships,
            &registry,
            Some(0),
            &UrlMode::Absolute,
        );
        context.insert("has_h1".to_string(), rendered.has_h1.into());
        context
    }

    /// Render `source` (this file's markdown, or the top of it) to HTML.
    ///
    /// Synchronous `render_str` rather than the async `render`: with oEmbed
    /// disabled (timeout 0, as a preview always is) the two are the same
    /// pipeline, and this avoids building a tokio runtime per preview.
    fn render_markdown(
        &self,
        source: &str,
    ) -> Result<markdown::MarkdownRenderResult, QuickLookError> {
        let config = &self.config;

        let link_transform_config = LinkTransformConfig {
            markdown_extensions: config.markdown_extensions.clone(),
            index_file: config.index_file.clone(),
            // Always "index": a preview has no trailing-slash page URL (the
            // file is shown from its folder), so markdown links must not get
            // the `../` compensation a served non-index page needs. Relative
            // URLs then mean what they mean on GitHub or in an editor, the same
            // as raw HTML `<img src>`, which `link_transform` never touches,
            // and both resolve against `base_dir_url`.
            is_index_file: true,
            url_depth: None,
            // QuickLook previews a single file with no repo index, so body
            // wikilinks never resolve globally; the page URL is unused.
            current_page_url: String::new(),
            markdown_page_probe: None,
        };

        markdown::render_str(
            source,
            &self.path,
            markdown::RenderOptions {
                root_path: &self.root_path,
                // oEmbed is disabled for a fast, network-free preview.
                oembed_timeout_ms: 0,
                link_transform_config,
                oembed_cache: None,
                // QuickLook is not server mode, and transcoding is server-only.
                server_mode: false,
                transcode_enabled: false,
                // No tag sources in QuickLook (no wikilink transformation).
                valid_tag_sources: std::collections::HashSet::new(),
                // Hardcoded off: a QuickLook preview is a static, scriptless
                // panel with nothing to anchor a review selection to.
                review: markdown::ReviewLines::Omit,
                // Honor the user's mark_incomplete config (defaults to off in
                // this preview).
                mark_incomplete: config.mark_incomplete.unwrap_or(false),
                incomplete_markers: &config.incomplete_markers,
                // No repo wikilink index in QuickLook.
                wikilink_index: None,
            },
        )
        .map_err(|e| QuickLookError::MarkdownRenderError {
            message: e.to_string(),
        })
    }
}

/// The longest prefix of `bytes` that ends at a line break, or all of `bytes`
/// when it has none (one enormous line is still worth a thumbnail).
///
/// Cutting at `\n` keeps every line that survives whole, so markdown that
/// depends on line structure - headings, list markers, fences - is not split
/// mid-construct, and a UTF-8 sequence is never cut in half (`\n` cannot occur
/// inside one).
fn cut_at_line_boundary(bytes: &[u8]) -> &[u8] {
    bytes
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(bytes, |newline| &bytes[..=newline])
}

/// The cover image a thumbnail should use instead of the rendered page.
///
/// Tries each of [`COVER_IMAGE_KEYS`] in order and returns the first that names
/// a usable image: a string that is not a URL with a scheme, names a file the
/// server would serve, has one of [`COVER_IMAGE_EXTENSIONS`], and is non-empty
/// and no larger than [`MAX_ATTACHMENT_BYTES`]. The value is read exactly as an
/// `<img src>` in the page is ([`asset_url_path`]): a root-relative value
/// (`/images/x.png`) resolves like a root-relative asset (direct, then the
/// static folder, inside the root or an external overlay); any other value is
/// relative to the note's own folder (`base_dir_url`), as it is for a contact
/// portrait.
///
/// # Security
///
/// The value is attacker-controlled frontmatter and the returned path is read by
/// the thumbnail extension, whose sandbox can read anything the user can. So the
/// same boundary as attachments applies - [`AssetResolver`], on canonical
/// paths, never on the authored string - and the extension test runs on the
/// *canonical* name, so a `cover.png` symlink to a non-image is refused.
fn thumbnail_cover_image(
    frontmatter: &markdown::SimpleMetadata,
    resolver: &AssetResolver,
    base_dir_url: &str,
) -> Option<String> {
    COVER_IMAGE_KEYS
        .iter()
        .filter_map(|key| frontmatter.get(*key)?.as_str())
        .find_map(|value| resolve_cover_image(value.trim(), resolver, base_dir_url))
}

/// One candidate of [`thumbnail_cover_image`], resolved and vetted.
fn resolve_cover_image(
    value: &str,
    resolver: &AssetResolver,
    base_dir_url: &str,
) -> Option<String> {
    let resolved = resolver.resolve(&asset_url_path(base_dir_url, value)?)?;

    let is_image = resolved
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|extension| {
            COVER_IMAGE_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        });
    let size = std::fs::metadata(&resolved).ok()?.len();

    (is_image && size > 0 && size <= MAX_ATTACHMENT_BYTES)
        .then(|| resolved.to_str().map(str::to_string))
        .flatten()
}

/// Read up to `cap` bytes from `path`.
///
/// Returns the bytes and whether the file was longer than the cap. Uses
/// `Read::take` rather than reading the file and truncating, so a huge file is
/// never pulled into memory in the first place. One extra byte is requested so
/// "exactly at the cap" is distinguishable from "longer than the cap" without a
/// second `metadata()` syscall (and without the TOCTOU a stat-then-read has).
fn read_capped(path: &Path, cap: usize) -> Result<(Vec<u8>, bool), QuickLookError> {
    use std::io::Read;

    let file = std::fs::File::open(path).map_err(|e| QuickLookError::FileReadError {
        message: format!("{}: {}", path.display(), e),
    })?;

    let mut bytes = Vec::new();
    file.take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| QuickLookError::FileReadError {
            message: format!("{}: {}", path.display(), e),
        })?;

    let truncated = bytes.len() > cap;
    bytes.truncate(cap);
    Ok((bytes, truncated))
}

/// Wrap verbatim text in a `<pre>` block for the preview template.
///
/// Escaping uses `encode_quoted_attribute` even though this is element text.
/// Its escape set (`& < > " '`) is a superset of what element text needs, and
/// the extra two matter: [`collect_preview_attachments`] later runs regexes for
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
    base_dir_url: &str,
    ql_config: &QuickLookConfig,
    config: &Config,
) -> Result<PreviewDocument, QuickLookError> {
    let (bytes, truncated) = read_capped(path, MAX_TEXT_PREVIEW_BYTES)?;
    let text = String::from_utf8_lossy(&bytes);
    let html = text_to_pre_html(&text, mode, truncated);

    // There is no frontmatter in a text file, so title the preview with the
    // file name - otherwise the template falls back to a bare "Preview".
    let mut frontmatter = markdown::SimpleMetadata::new();
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        frontmatter.insert("title".to_string(), serde_json::Value::String(name.into()));
    }

    render_quicklook_template(
        &html,
        frontmatter,
        Vec::new(),     // no headings: a text preview has no table of contents
        HashMap::new(), // no contact card: a text file has no frontmatter
        root_path,
        base_dir_url,
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

/// Where a local asset URL leads: the file the **server** would serve for it.
///
/// A thin wrapper around [`path_resolver::resolve_request_path`] over the
/// repository's vaults ([`repo::local_vaults`]: the repository plus an external
/// static overlay such as SvelteKit's `static_folder = "../../static"`), so a
/// preview finds every asset the server finds - direct path, then the static
/// folder, inside the root or out of it - and refuses everything it refuses. It
/// used to be a parallel reimplementation that joined `static_folder` onto the
/// root and then demanded the result stay *inside* the root, which silently
/// refused every external overlay.
///
/// # Security
///
/// Every URL it is asked about comes from attacker-controlled markdown. What it
/// returns is the canonical path of an existing regular file that the resolver
/// placed inside the repository or the configured static overlay (itself vetted
/// by [`config::resolve_static_overlay`]): `..` above the root, absolute paths,
/// hidden segments (`.git/config`, `.env`) and symlinks out of either vault are
/// refused there, on canonical paths. Symlink **mounts** are deliberately off
/// (unlike the desktop server's default): a previewed folder is often one the
/// user merely downloaded, and the preview should not read through its links to
/// elsewhere on the disk.
struct AssetResolver {
    paths: OwnedPathResolverConfig,
}

impl AssetResolver {
    /// The resolver for the repository at `root_path`, configured by `config`;
    /// `None` when the root cannot be canonicalized, in which case nothing can
    /// be proven to be inside it.
    fn new(root_path: &Path, config: &Config) -> Option<Self> {
        let canonical_root = root_path.canonicalize().ok()?;
        let repo::LocalVaults {
            vault,
            static_vault,
        } = repo::local_vaults(&canonical_root, &config.static_folder, false);
        Some(Self {
            paths: OwnedPathResolverConfig {
                vault,
                static_vault,
                static_folder: config.static_folder.clone(),
                markdown_extensions: config.markdown_extensions.clone(),
                index_file: config.index_file.clone(),
                // Tag pages are not files.
                tag_sources: Vec::new(),
                exempt_hidden_dirs: config
                    .explicit_hidden_dirs
                    .iter()
                    .filter_map(|dir| VaultPath::from_relative_native(dir).ok())
                    .collect(),
            },
        })
    }

    /// The file a root-relative, decoded `url_path` (`/images/a.png`) names,
    /// or `None` when the server would not serve one there.
    ///
    /// Only a *file the URL names* counts: a static file, or a markdown file
    /// named with its extension (`/docs/readme.md`, attached as before). A page
    /// URL that the server renders (`/docs/` for `docs/index.md`, `/guide/` for
    /// `guide.md`), a directory listing or a redirect is not an asset.
    fn resolve(&self, url_path: &str) -> Option<PathBuf> {
        // The server's catch-all route captures the path without its leading
        // slash; a vault path with one is absolute and refused.
        let request = url_path.trim_start_matches('/');
        let key = match path_resolver::resolve_request_path(&self.paths.as_config(), request) {
            ResolvedPath::StaticFile(key) => key,
            ResolvedPath::MarkdownFile(key) if self.names_markdown_file(request) => key,
            _ => return None,
        };
        // Keys of a `LocalVault` are canonical paths; the resolver has stat'ed
        // them as files, and this re-check is what `cid_for` reads the size of.
        std::fs::metadata(&key)
            .is_ok_and(|metadata| metadata.is_file())
            .then_some(key)
    }

    /// Whether `request`'s last segment carries a markdown extension, i.e. it
    /// names the file itself rather than its page.
    fn names_markdown_file(&self, request: &str) -> bool {
        Path::new(request)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                repo::is_markdown_extension(extension, &self.paths.markdown_extensions)
            })
    }
}

/// The root-relative, decoded file path an attribute value names, or `None` when
/// it does not name a local file and must be left as authored.
///
/// `value` is the attribute exactly as it appears in the HTML, so it is first
/// HTML-unescaped (`a&amp;b.png`), then cut at its query or fragment, then
/// percent-decoded (`Pasted%20image.png`): the renderer encodes the URLs it
/// writes, and the file on disk has the decoded name. Root-relative values
/// (`/images/a.png`) are returned as they are; relative ones (`img/a.png`,
/// `../b.png`) are resolved against `base_dir_url` by [`resolve_relative_url`].
/// Anything with a scheme (`https:`, `data:`, `mailto:`, `cid:`), a
/// protocol-relative `//host/...`, and a bare `#fragment` or `?query` is left
/// alone.
///
/// This is *not* a containment check: the result is attacker-controlled and may
/// still name `/../../etc/passwd` after decoding. [`AssetResolver::resolve`]
/// judges it, as the server judges every request path.
fn asset_url_path(base_dir_url: &str, value: &str) -> Option<String> {
    let value = html_escape::decode_html_entities(value);
    if value.is_empty()
        || value.starts_with("//")
        || value.starts_with(['#', '?'])
        || url_scheme(&value).is_some()
    {
        return None;
    }

    let path = value.split(['?', '#']).next().unwrap_or_default();
    let decoded = percent_decode_str(path).decode_utf8().ok()?;
    if decoded.starts_with('/') {
        Some(decoded.into_owned())
    } else {
        resolve_relative_url(base_dir_url, &decoded)
    }
}

/// `relative` resolved lexically against `base_dir_url` (a folder URL ending in
/// `/`; anything after its last `/` is ignored, as a browser would), with `.`
/// segments dropped and `..` removing one directory.
///
/// `None` when `..` climbs above the root: such a URL names nothing in the
/// repository, so it is left as authored rather than clamped onto a file the
/// author did not name.
fn resolve_relative_url(base_dir_url: &str, relative: &str) -> Option<String> {
    let base_dir = base_dir_url.rsplit_once('/').map_or("", |(dir, _)| dir);
    let segments = base_dir
        .split('/')
        .chain(relative.split('/'))
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .try_fold(Vec::new(), |mut segments, segment| {
            if segment == ".." {
                segments.pop()?;
            } else {
                segments.push(segment);
            }
            Some(segments)
        })?;
    Some(format!("/{}", segments.join("/")))
}

/// Builds the attachment list for one preview while the HTML is rewritten.
///
/// Stateful because the two regex passes (double- then single-quoted attributes)
/// share one id space, one dedup table and one byte budget: the same image
/// referenced twice must become one attachment, not two copies of the file.
struct AttachmentCollector<'a> {
    resolver: &'a AssetResolver,
    /// The URL of the previewed file's folder; relative attribute values
    /// resolve against it (see [`asset_url_path`]).
    base_dir_url: &'a str,
    /// Resolved path -> already-assigned `cid` id.
    seen: HashMap<PathBuf, String>,
    attachments: Vec<PreviewAttachment>,
    attached_bytes: u64,
}

impl<'a> AttachmentCollector<'a> {
    fn new(resolver: &'a AssetResolver, base_dir_url: &'a str) -> Self {
        Self {
            resolver,
            base_dir_url,
            seen: HashMap::new(),
            attachments: Vec::new(),
            attached_bytes: 0,
        }
    }

    /// The `cid:` URL for a root-relative, decoded `url_path`, or `None` to leave
    /// the URL as authored.
    ///
    /// `None` means "this preview will not show that file", for any of the
    /// reasons that can apply: the server would not serve a file there (it does
    /// not exist, is not a regular file, lives outside the repository and its
    /// static overlay, or is hidden), its name is not valid UTF-8, or attaching
    /// it would blow the size budget.
    fn cid_for(&mut self, url_path: &str) -> Option<String> {
        let resolved = self.resolver.resolve(url_path)?;

        if let Some(id) = self.seen.get(&resolved) {
            return Some(format!("cid:{id}"));
        }

        let path = resolved.to_str()?.to_string();
        let size = std::fs::metadata(&resolved).ok()?.len();
        if size > MAX_ATTACHMENT_BYTES
            || self.attached_bytes.saturating_add(size) > MAX_TOTAL_ATTACHMENT_BYTES
        {
            return None;
        }

        // Positional, so the ids are stable for a given document and assertable
        // in tests. They are never shown to a reader.
        let id = format!("mbr-asset-{}", self.attachments.len());
        self.attached_bytes += size;
        self.attachments.push(PreviewAttachment {
            id: id.clone(),
            path,
        });
        self.seen.insert(resolved, id.clone());
        Some(format!("cid:{id}"))
    }

    /// The `cid:` URL for one authored URL (still HTML-escaped), or `None` to
    /// leave it as authored.
    fn cid_for_value(&mut self, value: &str) -> Option<String> {
        asset_url_path(self.base_dir_url, value).and_then(|url_path| self.cid_for(&url_path))
    }

    /// Rewrite one matched `src`/`srcset`/`href`/`poster` attribute, attaching
    /// its target. A `srcset` attaches each candidate URL on its own, through
    /// the same [`Self::cid_for`] (so containment, dedup and budgets apply per
    /// file), keeping descriptors and any candidate it cannot attach as
    /// authored ([`url_path::rewrite_srcset`], the renderer's own reading).
    fn rewrite(&mut self, caps: &regex::Captures<'_>, quote: char) -> String {
        let attr = &caps[1];
        let value = &caps[2];
        let target = if attr == "srcset" {
            url_path::rewrite_srcset(value, |url| self.cid_for_value(url))
        } else {
            self.cid_for_value(value)
                .map_or(Cow::Borrowed(value), Cow::Owned)
        };
        format!("{}={}{}{}", attr, quote, target, quote)
    }
}

/// Rewrite local asset URLs to `cid:` references and return the files they name.
///
/// A data-based `QLPreviewReply` carries local resources as attachments keyed by
/// id; `cid:{id}` is how HTML addresses one. The reply has no `<base>` (it is
/// served from QuickLook's own `x-apple-ql-id2://` URL), so relative URLs are
/// resolved here against `base_dir_url`, the note's folder: QuickLook renders
/// markdown links without the trailing-slash `../` compensation, so relative
/// markdown and raw-HTML URLs both mean what they do on GitHub or in an
/// editor. Each resulting URL is then resolved by the server's own resolver
/// ([`AssetResolver`]): the direct path first, then the static folder, inside
/// the root or an external overlay.
///
/// # Security
///
/// This is the boundary. The returned list is the *only* set of local files a
/// preview can read, so nothing may enter it that the server would not serve
/// from the repository or its static overlay - see [`AssetResolver`]. Without a
/// resolver (the root cannot be canonicalized) there is nothing to contain
/// against, so the HTML is returned untouched and nothing is attached, rather
/// than rewritten optimistically.
fn collect_preview_attachments(
    html: &str,
    resolver: Option<&AssetResolver>,
    base_dir_url: &str,
) -> (String, Vec<PreviewAttachment>) {
    let Some(resolver) = resolver else {
        return (html.to_string(), Vec::new());
    };

    let mut collector = AttachmentCollector::new(resolver, base_dir_url);

    // First pass: handle double-quoted attributes
    let result = ASSET_ATTRIBUTE_DOUBLE_QUOTED
        .replace_all(html, |caps: &regex::Captures| collector.rewrite(caps, '"'))
        .into_owned();

    // Second pass: handle single-quoted attributes
    let result = ASSET_ATTRIBUTE_SINGLE_QUOTED
        .replace_all(&result, |caps: &regex::Captures| {
            collector.rewrite(caps, '\'')
        })
        .into_owned();

    (result, collector.attachments)
}

/// Render the QuickLook HTML template with inlined assets.
///
/// `card` is [`QuickLookTarget::contact_card_context`]'s context: when it holds
/// a `contact`, the shared `_contact_card.html` partial is rendered and placed
/// at the top of `<main>`, as `index.html` places it, *before* attachments are
/// collected — so the portrait (`contact.image`, already link-transformed and
/// so relative to the note's folder like a body image) goes through
/// [`collect_preview_attachments`] like every other local URL, and nowhere else.
#[allow(clippy::too_many_arguments)]
fn render_quicklook_template(
    markdown_html: &str,
    frontmatter: markdown::SimpleMetadata,
    headings: Vec<markdown::HeadingInfo>,
    card: HashMap<String, serde_json::Value>,
    root_path: &Path,
    base_dir_url: &str,
    ql_config: &QuickLookConfig,
    config: &Config,
) -> Result<PreviewDocument, QuickLookError> {
    let template_error = |e: tera::Error| QuickLookError::TemplateRenderError {
        message: e.to_string(),
    };

    // Create Tera template engine with QuickLook template and the shared
    // card partial (plus the helpers it calls, e.g. `icon()`).
    let mut tera = Tera::default();
    templates::register_template_helpers(&mut tera);
    tera.add_raw_template("quicklook.html", QUICKLOOK_TEMPLATE)
        .map_err(template_error)?;

    // Build template context
    let mut context = Context::new();

    // Add frontmatter fields
    for (k, v) in &frontmatter {
        context.insert(k, v);
    }

    let card_html = if card.contains_key("contact") {
        tera.add_raw_template("_contact_card.html", templates::CONTACT_CARD_TEMPLATE)
            .map_err(template_error)?;
        let mut card_context = context.clone();
        for (k, v) in &card {
            card_context.insert(k, v);
        }
        tera.render("_contact_card.html", &card_context)
            .map_err(template_error)?
    } else {
        String::new()
    };
    let page_html = card_html + markdown_html;

    // Turn local asset URLs (root-relative, or relative to `base_dir_url`) into
    // cid: references and collect the files they name, wherever the server
    // would find them (the static folder included, in or out of the root).
    let resolver = AssetResolver::new(root_path, config);
    let (markdown_html, attachments) =
        collect_preview_attachments(&page_html, resolver.as_ref(), base_dir_url);

    // Load custom theme CSS if available
    let custom_theme = load_custom_theme(root_path);
    let custom_user_css = load_custom_user_css(root_path);

    // Build inline CSS using configured theme
    let inline_css = build_inline_css(ql_config, &config.theme, &custom_theme, &custom_user_css);

    // Build inline JavaScript
    let inline_js = build_inline_js(ql_config);

    // Add frontmatter as JSON
    let frontmatter_json = serde_json::to_string(&frontmatter).unwrap_or_else(|_| "{}".to_string());
    context.insert("frontmatter_json", &frontmatter_json);

    // Add headings for table of contents (as actual vector, not JSON string)
    context.insert("headings", &headings);

    // Add main content
    context.insert("markdown", &markdown_html);
    context.insert("inline_css", &inline_css);
    context.insert("inline_js", &inline_js);

    // Render template
    let html = tera
        .render("quicklook.html", &context)
        .map_err(template_error)?;

    Ok(PreviewDocument { html, attachments })
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
        css.push_str(embedded_text(assets::THEME_CSS));
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
        js.push_str(embedded_text(assets::MERMAID_JS));
        js.push('\n');
    }

    // QuickLook-specific initialization
    js.push_str(QUICKLOOK_JS);

    js
}

/// Text of a compiled-in asset, or `""` if it is not valid UTF-8.
///
/// Takes the asset itself rather than a route so QuickLook names only the
/// assets it inlines, never the whole [`assets::DEFAULT_FILES`] table.
fn embedded_text(asset: &'static [u8]) -> &'static str {
    std::str::from_utf8(asset).unwrap_or("")
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
    <!--
      Deliberately no <base>. A data-based preview is served from QuickLook's
      own x-apple-ql-id2:// URL, so a file:// base could never load anything
      anyway - and it would break the table of contents, because a bare
      "#heading" resolves against the base and would navigate away from the
      preview instead of scrolling within it. Local assets are cid: URLs,
      which are absolute and need no base.
    -->
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

    /// Just the HTML half of a preview, for the many tests that assert only on
    /// markup. Tests about local assets use [`render_preview`] directly, since
    /// for those the attachment list is half the answer.
    fn render_preview_html(file_path: String) -> Result<String, QuickLookError> {
        render_preview(file_path, None).map(|document| document.html)
    }

    /// The `cid:` URL an attachment list assigns to `path`, for asserting that
    /// the HTML and the attachments agree about a specific file.
    fn cid_of(attachments: &[PreviewAttachment], path: &Path) -> String {
        let wanted = path.to_str().expect("test paths are UTF-8");
        let attachment = attachments
            .iter()
            .find(|a| a.path == wanted)
            .unwrap_or_else(|| panic!("no attachment for {wanted}; got {attachments:?}"));
        format!("cid:{}", attachment.id)
    }

    #[test]
    fn test_render_simple_markdown() {
        let mut file = markdown_temp_file();
        writeln!(file, "# Hello World\n\nThis is a test.").unwrap();
        let path = file.path().to_str().unwrap().to_string();

        let html = render_preview_html(path).unwrap();

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

        let html = render_preview_html(path).unwrap();

        assert!(html.contains("Test Title"));
        assert!(html.contains("A test document"));
    }

    #[test]
    fn test_render_with_code_block() {
        let mut file = markdown_temp_file();
        writeln!(file, "```rust\nfn main() {{}}\n```").unwrap();
        let path = file.path().to_str().unwrap().to_string();

        let html = render_preview_html(path).unwrap();

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
        let html = render_preview_html(path).unwrap();

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
        };

        let html = render_preview_with_config(path, None, config).unwrap().html;

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
    /// The resolver a preview of a repository at `root` with `static_folder`
    /// (and every other setting at its default) would use.
    fn test_resolver(root: &Path, static_folder: &str) -> Option<AssetResolver> {
        let config = Config {
            static_folder: static_folder.to_string(),
            ..Config::default()
        };
        AssetResolver::new(root, &config)
    }

    /// The file a root-relative `url_path` resolves to in a preview.
    fn resolve_asset_path(root: &Path, static_folder: &str, url_path: &str) -> Option<PathBuf> {
        test_resolver(root, static_folder)?.resolve(url_path)
    }

    /// [`collect_preview_attachments`] for a repository at `root`.
    fn collect_attachments(
        html: &str,
        root: &Path,
        static_folder: &str,
        base_dir_url: &str,
    ) -> (String, Vec<PreviewAttachment>) {
        collect_preview_attachments(
            html,
            test_resolver(root, static_folder).as_ref(),
            base_dir_url,
        )
    }

    fn repo_with_file(relative_file: &str) -> (tempfile::TempDir, PathBuf) {
        let temp_dir = tempfile::tempdir().unwrap();
        let file = temp_dir.path().join("repo").join(relative_file);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"asset bytes").unwrap();
        let canonical_root = temp_dir.path().join("repo").canonicalize().unwrap();
        (temp_dir, canonical_root)
    }

    #[test]
    fn test_collect_attachments_double_quotes() {
        let (_tmp, root) = repo_with_file("images/test.png");
        let html = r#"<img src="/images/test.png" alt="test">"#;
        let (result, attachments) = collect_attachments(html, &root, "", "/");

        let cid = cid_of(&attachments, &root.join("images/test.png"));
        assert_eq!(result, format!(r#"<img src="{cid}" alt="test">"#));
        assert_eq!(attachments.len(), 1);
    }

    #[test]
    fn test_collect_attachments_single_quotes() {
        let (_tmp, root) = repo_with_file("videos/test.mp4");
        let html = r#"<source src='/videos/test.mp4' type="video/mp4">"#;
        let (result, attachments) = collect_attachments(html, &root, "", "/");

        let cid = cid_of(&attachments, &root.join("videos/test.mp4"));
        assert_eq!(result, format!(r#"<source src='{cid}' type="video/mp4">"#));
    }

    #[test]
    fn test_collect_attachments_href() {
        let (_tmp, root) = repo_with_file("docs/readme.md");
        let html = r#"<a href="/docs/readme.md">Link</a>"#;
        let (result, attachments) = collect_attachments(html, &root, "", "/");

        let cid = cid_of(&attachments, &root.join("docs/readme.md"));
        assert_eq!(result, format!(r#"<a href="{cid}">Link</a>"#));
    }

    #[test]
    fn test_collect_attachments_poster() {
        let (_tmp, root) = repo_with_file("images/thumb.jpg");
        let html = r#"<video poster="/images/thumb.jpg"></video>"#;
        let (result, attachments) = collect_attachments(html, &root, "", "/");

        let cid = cid_of(&attachments, &root.join("images/thumb.jpg"));
        assert_eq!(result, format!(r#"<video poster="{cid}"></video>"#));
    }

    #[test]
    fn test_collect_attachments_reuses_one_id_per_file() {
        // The same file named three ways in two quoting styles is still one
        // attachment: QuickLook would otherwise be handed the bytes repeatedly.
        let (_tmp, root) = repo_with_file("images/test.png");
        let html = concat!(
            r#"<img src="/images/test.png">"#,
            r#"<img src='/images/test.png'>"#,
            r#"<a href="/images/test.png">x</a>"#,
        );
        let (result, attachments) = collect_attachments(html, &root, "", "/");

        assert_eq!(attachments.len(), 1);
        let cid = cid_of(&attachments, &root.join("images/test.png"));
        assert_eq!(result.matches(&cid).count(), 3);
    }

    #[test]
    fn test_collect_attachments_unknown_root_attaches_nothing() {
        // A root that cannot be canonicalized gives nothing to contain against,
        // so the HTML must be left alone rather than rewritten optimistically.
        let html = r#"<img src="/images/test.png">"#;
        let (result, attachments) =
            collect_attachments(html, Path::new("/no/such/root"), "static", "/");
        assert_eq!(result, html);
        assert!(attachments.is_empty());
    }

    #[test]
    fn test_resolve_relative_url_against_base_dir_url() {
        // The base is the note's folder: `/notes/` for both `notes/note.md`
        // and `notes/index.md`, `/` for a note at the root.
        let cases = [
            ("/notes/", "img/a.png", Some("/notes/img/a.png")),
            ("/notes/", "./img/./a.png", Some("/notes/img/a.png")),
            ("/notes/", "../images/b.png", Some("/images/b.png")),
            ("/notes/", "./../images/b.png", Some("/images/b.png")),
            ("/a/b/", "../../c.png", Some("/c.png")),
            ("/", "a.png", Some("/a.png")),
            ("/", "docs/images/x.png", Some("/docs/images/x.png")),
            ("/notes/", "../../etc/passwd", None),
            ("/", "../a.png", None),
        ];
        for (base_dir_url, relative, expected) in cases {
            assert_eq!(
                resolve_relative_url(base_dir_url, relative).as_deref(),
                expected,
                "{relative} against {base_dir_url}"
            );
        }
    }

    #[test]
    fn test_asset_url_path_classifies_and_decodes() {
        let base = "/notes/";
        let cases = [
            // Local: root-relative as is, relative resolved against the folder.
            ("/images/b.png", Some("/images/b.png")),
            ("img/a.png", Some("/notes/img/a.png")),
            ("../images/b.png", Some("/images/b.png")),
            // Decoded the way the file is named on disk.
            ("Pasted%20image%201.png", Some("/notes/Pasted image 1.png")),
            ("/x/my%20photo.png", Some("/x/my photo.png")),
            ("caf%C3%A9.png", Some("/notes/café.png")),
            ("a&amp;b.png", Some("/notes/a&b.png")),
            // Query and fragment are not part of the file name; an encoded
            // `?` is.
            ("a.png?v=2#top", Some("/notes/a.png")),
            ("what%3F.png", Some("/notes/what?.png")),
            // Not local: left as authored.
            ("", None),
            ("#heading", None),
            ("?q=1", None),
            ("//example.com/x.png", None),
            ("https://example.com/x.png", None),
            ("data:image/png;base64,AAAA", None),
            ("mailto:me@example.com", None),
            ("cid:mbr-asset-0", None),
            ("../../x.png", None),
            ("bad%FF.png", None),
        ];
        for (value, expected) in cases {
            assert_eq!(asset_url_path(base, value).as_deref(), expected, "{value}");
        }
    }

    #[test]
    fn test_collect_attachments_resolves_relative_against_base_dir_url() {
        let (_tmp, root) = repo_with_file("notes/img/a.png");
        let html = r#"<img src="img/a.png"><img src='./img/a.png'>"#;
        let (result, attachments) = collect_attachments(html, &root, "", "/notes/");

        let cid = cid_of(&attachments, &root.join("notes/img/a.png"));
        assert_eq!(result, format!(r#"<img src="{cid}"><img src='{cid}'>"#));
        assert_eq!(attachments.len(), 1);
    }

    #[test]
    fn test_collect_attachments_relative_uses_static_folder_fallback() {
        let (_tmp, root) = repo_with_file("static/pics/c.png");
        let html = r#"<img src="pics/c.png">"#;
        let (result, attachments) = collect_attachments(html, &root, "static", "/");

        let cid = cid_of(&attachments, &root.join("static/pics/c.png"));
        assert_eq!(result, format!(r#"<img src="{cid}">"#));
    }

    #[test]
    fn test_collect_attachments_decodes_root_relative_paths() {
        // The renderer percent-encodes the URLs it writes, so before this a
        // root-relative image with a space in its name never loaded either.
        let (_tmp, root) = repo_with_file("images/my photo.png");
        let html = r#"<img src="/images/my%20photo.png">"#;
        let (result, attachments) = collect_attachments(html, &root, "", "/");

        let cid = cid_of(&attachments, &root.join("images/my photo.png"));
        assert_eq!(result, format!(r#"<img src="{cid}">"#));
    }

    #[test]
    fn test_collect_attachments_encoded_traversal_stays_contained() {
        // Decoding happens before containment, so `%2E%2E` is just `..` and
        // the canonical check still refuses it.
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(temp_dir.path().join("secret.txt"), b"secret").unwrap();
        let html = r#"<a href="/%2E%2E/secret.txt">x</a><a href="/..%2Fsecret.txt">y</a>"#;
        let (result, attachments) = collect_attachments(html, &root, "", "/");
        assert_eq!(result, html);
        assert!(attachments.is_empty());
    }

    #[test]
    fn test_collect_attachments_leaves_protocol_relative_alone() {
        // `//host/x.png` is a remote URL, even when a `host/x.png` exists in
        // the repository.
        let (_tmp, root) = repo_with_file("host/x.png");
        let html = r#"<img src="//host/x.png">"#;
        let (result, attachments) = collect_attachments(html, &root, "", "/");
        assert_eq!(result, html);
        assert!(attachments.is_empty());
    }

    #[test]
    fn test_collect_attachments_preserves_http() {
        // HTTP URLs should NOT be converted
        let html = r#"<img src="https://example.com/image.png">"#;
        let (_tmp, root) = repo_with_file("images/test.png");
        let (result, attachments) = collect_attachments(html, &root, "", "/");
        assert_eq!(result, r#"<img src="https://example.com/image.png">"#);
        assert!(attachments.is_empty());
    }

    #[test]
    fn test_collect_attachments_static_folder_fallback() {
        // Test that static folder fallback works when file only exists there
        let temp_dir = tempfile::tempdir().unwrap();
        let static_images = temp_dir.path().join("static/images");
        std::fs::create_dir_all(&static_images).unwrap();
        std::fs::write(static_images.join("photo.jpg"), b"image data").unwrap();

        let html = r#"<img src="/images/photo.jpg">"#;
        let (result, attachments) = collect_attachments(html, temp_dir.path(), "static", "/");

        // Should resolve to static/images/photo.jpg since /images/photo.jpg doesn't exist
        let expected_path = static_images.canonicalize().unwrap().join("photo.jpg");
        let cid = cid_of(&attachments, &expected_path);
        assert_eq!(result, format!(r#"<img src="{cid}">"#));
    }

    #[test]
    fn test_collect_attachments_direct_path_preferred() {
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
        let (_result, attachments) = collect_attachments(html, temp_dir.path(), "static", "/");

        // Should resolve to direct path since it exists
        let expected_path = direct_images.canonicalize().unwrap().join("photo.jpg");
        assert_eq!(
            attachments,
            vec![PreviewAttachment {
                id: "mbr-asset-0".to_string(),
                path: expected_path.to_str().unwrap().to_string(),
            }]
        );
    }

    #[test]
    fn test_collect_attachments_neither_exists() {
        // A URL that resolves to no file anywhere in the repo is left alone: a
        // cid: URL is a statement that the target is a contained, existing
        // asset, so one must not be minted for a path we never verified.
        let temp_dir = tempfile::tempdir().unwrap();

        let html = r#"<img src="/images/missing.jpg">"#;
        let (result, attachments) = collect_attachments(html, temp_dir.path(), "static", "/");

        assert_eq!(result, html);
        assert!(attachments.is_empty());
    }

    #[test]
    fn test_collect_attachments_skips_oversized_asset() {
        // Attachments are read eagerly, so one enormous file must not be pulled
        // into a spacebar preview. It keeps its authored URL and does not load.
        let (_tmp, root) = repo_with_file("images/small.png");
        let huge = root.join("images/huge.mp4");
        let file = std::fs::File::create(&huge).unwrap();
        file.set_len(MAX_ATTACHMENT_BYTES + 1).unwrap();
        drop(file);

        let html = r#"<img src="/images/small.png"><video src="/images/huge.mp4"></video>"#;
        let (result, attachments) = collect_attachments(html, &root, "", "/");

        let cid = cid_of(&attachments, &root.join("images/small.png"));
        assert_eq!(attachments.len(), 1, "only the small asset is attached");
        assert!(result.contains(&format!(r#"<img src="{cid}">"#)));
        assert!(
            result.contains(r#"<video src="/images/huge.mp4">"#),
            "oversized asset keeps its authored URL: {result}"
        );
    }

    #[test]
    fn test_collect_attachments_stops_at_total_budget() {
        // Each file is individually at the per-asset cap; the total budget is a
        // whole number of them, so the one after that must not fit. Document
        // order decides who does. (`set_len` makes these sparse, so the test
        // costs no real disk.)
        let (_tmp, root) = repo_with_file("images/a.bin");
        let fit = (MAX_TOTAL_ATTACHMENT_BYTES / MAX_ATTACHMENT_BYTES) as usize;
        let names: Vec<String> = (0..=fit).map(|i| format!("{i}.bin")).collect();
        for name in &names {
            let file = std::fs::File::create(root.join("images").join(name)).unwrap();
            file.set_len(MAX_ATTACHMENT_BYTES).unwrap();
        }

        let html: String = names
            .iter()
            .map(|name| format!(r#"<img src="/images/{name}">"#))
            .collect();
        let (result, attachments) = collect_attachments(&html, &root, "", "/");

        assert_eq!(attachments.len(), fit, "the last asset busts the budget");
        assert_eq!(
            attachments[0].path,
            root.join("images/0.bin").to_str().unwrap()
        );
        assert!(
            result.contains(&format!(r#"<img src="/images/{fit}.bin">"#)),
            "the asset that did not fit keeps its authored URL: {result}"
        );
    }

    #[test]
    fn test_collect_attachments_never_attaches_a_directory() {
        // `/` and `/images` both canonicalize to directories inside the root and
        // would pass a containment check that only compared prefixes.
        let (_tmp, root) = repo_with_file("images/test.png");

        let html = r#"<img src="/"><img src="/images"><img src="/images/">"#;
        let (result, attachments) = collect_attachments(html, &root, "", "/");

        assert!(attachments.is_empty(), "got {attachments:?}");
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
    fn test_collect_attachments_does_not_attach_escaping_file() {
        // End-to-end: rendered HTML naming an escaping symlink keeps its
        // original URL, so no file outside the repository is ever handed to
        // QuickLook.
        let (_tmp, root) = repo_with_file("images/ok.png");
        let outside_dir = tempfile::tempdir().unwrap();
        let secret = outside_dir.path().join("id_rsa");
        std::fs::write(&secret, b"PRIVATE KEY").unwrap();
        std::os::unix::fs::symlink(&secret, root.join("images/leak.png")).unwrap();

        let html = r#"<img src="/images/leak.png"><img src='/../id_rsa'>"#;
        let (result, attachments) = collect_attachments(html, &root, "static", "/");

        assert_eq!(result, html);
        assert!(attachments.is_empty(), "got {attachments:?}");
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
        let document = render_preview(path, None).unwrap();

        // The vid shortcode generates /videos/test.mp4, which must become an
        // attachment the HTML addresses by cid.
        let expected = videos_dir.canonicalize().unwrap().join("test.mp4");
        let cid = cid_of(&document.attachments, &expected);
        assert!(
            document.html.contains(&cid),
            "HTML should address the video attachment as {cid}.\nHTML excerpt: {}",
            document
                .html
                .lines()
                .filter(|l| l.contains("video") || l.contains("source") || l.contains("cid:"))
                .collect::<Vec<_>>()
                .join("\n")
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
        let document = render_preview(path, None).unwrap();

        // The image should resolve to static/images/blog/test.png (canonicalized:
        // the collector only attaches paths it has proven are inside the root)
        let expected_static_path = static_images.canonicalize().unwrap().join("test.png");
        let cid = cid_of(&document.attachments, &expected_static_path);
        assert!(
            document.html.contains(&cid),
            "Expected image to use static folder path.\nHTML excerpt: {}",
            document
                .html
                .lines()
                .filter(|l| l.contains("img") || l.contains("cid:") || l.contains("/images"))
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

        let document = render_preview(file_path.to_str().unwrap().to_string(), None).unwrap();

        let expected = static_images.canonicalize().unwrap().join("photo.jpg");
        let cid = cid_of(&document.attachments, &expected);
        assert!(
            document.html.contains(&cid),
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
        let html = render_preview_html(path.to_str().unwrap().to_string()).expect("render");
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
        // collect_attachments() runs over the rendered HTML. A source
        // file that merely mentions src="/..." must be shown, not rewritten.
        let (_dir, html) = preview_file("page.txt", b"<img src=\"/images/x.png\">\n");

        // `cid:` on its own appears in the inlined libraries, so match the
        // shape the rewriter would actually have produced.
        assert!(
            !html.contains(r#"src="cid:"#),
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
        assert_eq!(
            read_capped(&small, MAX_TEXT_PREVIEW_BYTES).unwrap(),
            (b"abc".to_vec(), false)
        );

        let big = dir.path().join("big.txt");
        std::fs::write(&big, vec![b'z'; MAX_TEXT_PREVIEW_BYTES + 10]).unwrap();
        let (bytes, truncated) = read_capped(&big, MAX_TEXT_PREVIEW_BYTES).unwrap();
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

    // MARK: relative asset URLs

    /// Markdown referencing local assets every way the renderer can write a
    /// relative URL, for `regression_repo`.
    const RELATIVE_ASSET_NOTE: &str = concat!(
        "rel ![a](img/a.png)\n\n",
        "dotrel ![a2](./img/a.png)\n\n",
        "same ![d](d.png)\n\n",
        "up ![b](../images/b.png)\n\n",
        "wiki ![[d.png]]\n\n",
        "spaces ![s](<Pasted image 1.png>)\n\n",
        "rootrel ![b2](/images/b.png)\n\n",
        "escape ![e](../../outside.png)\n\n",
        "link [doc](img/doc.pdf) and [page](other.md)\n\n",
        "remote ![r](https://example.com/r.png) ![p](//example.com/p.png) ",
        "[m](mailto:me@example.com) [f](#frag)\n\n",
        "data ![x](data:image/png;base64,AAAA)\n",
    );

    /// A repository with `note` (in `notes/`) holding `contents`, the assets
    /// [`RELATIVE_ASSET_NOTE`] names, a sibling note, and a file *outside* the
    /// repository. Returns the temp dir, canonical root and the note's path.
    fn regression_repo(note: &str, contents: &str) -> (tempfile::TempDir, PathBuf, String) {
        let (tmp, root, note) = thumbnail_repo(note, contents);
        for asset in [
            "notes/img/a.png",
            "notes/img/doc.pdf",
            "notes/d.png",
            "notes/Pasted image 1.png",
            "images/b.png",
            "notes/other.md",
        ] {
            write_file(&root.join(asset), asset.as_bytes());
        }
        write_file(&tmp.path().join("outside.png"), b"outside");
        (tmp, root, note)
    }

    fn src_of<'a>(html: &'a str, alt: &str) -> &'a str {
        let marker = format!(r#"alt="{alt}""#);
        let tag_start = html[..html.find(&marker).unwrap_or_else(|| panic!("no {marker}"))]
            .rfind("<img")
            .unwrap();
        let tag = &html[tag_start..];
        let src = &tag[tag.find(r#"src=""#).unwrap() + 5..];
        &src[..src.find('"').unwrap()]
    }

    #[test]
    fn test_render_preview_attaches_relative_assets() {
        let (_tmp, root, note) = regression_repo("notes/note.md", RELATIVE_ASSET_NOTE);
        let document = render_preview(note, None).unwrap();
        let html = &document.html;
        let cid = |path: &str| cid_of(&document.attachments, &root.join(path));

        assert_eq!(src_of(html, "a"), cid("notes/img/a.png"));
        assert_eq!(src_of(html, "a2"), cid("notes/img/a.png"));
        assert_eq!(src_of(html, "d"), cid("notes/d.png"));
        assert_eq!(src_of(html, "b"), cid("images/b.png"));
        assert_eq!(src_of(html, "d.png"), cid("notes/d.png"), "![[d.png]]");
        assert_eq!(src_of(html, "s"), cid("notes/Pasted image 1.png"));
        assert!(html.contains(&format!(r#"href="{}""#, cid("notes/img/doc.pdf"))));

        // One attachment per file, however it was referenced: a.png twice
        // relatively, b.png relatively and root-relatively, d.png twice.
        assert_eq!(src_of(html, "b2"), cid("images/b.png"));
        assert_eq!(document.attachments.len(), 5, "{:?}", document.attachments);

        // Escaping the root is left as authored (no `../` compensation is
        // added in a preview), not clamped or attached.
        assert_eq!(src_of(html, "e"), "../../outside.png");
        // A link to another page is not an asset; it is written relative to
        // the note's folder, as the author wrote it.
        assert!(html.contains(r#"href="other/""#), "page link untouched");
        // Remote, data, mail and fragment URLs are untouched.
        assert_eq!(src_of(html, "r"), "https://example.com/r.png");
        assert_eq!(src_of(html, "p"), "//example.com/p.png");
        assert_eq!(src_of(html, "x"), "data:image/png;base64,AAAA");
        assert!(html.contains(r#"href="mailto:me@example.com""#));
        assert!(html.contains(r##"href="#frag""##));
    }

    #[test]
    fn test_render_preview_index_and_non_index_notes_resolve_alike() {
        // Served, `notes/index.md` is `/notes/` and `notes/note.md` is
        // `/notes/note/`, and their links differ by a `../`. A preview is the
        // file in its folder, so both resolve every relative URL the same way.
        let contents = concat!(
            "![a](img/a.png)\n\n",
            "<img src=\"img/a.png\" alt=\"h\">\n\n",
            "up ![b](../images/b.png) [page](other.md)\n",
        );
        let rendered: Vec<_> = ["notes/index.md", "notes/note.md"]
            .into_iter()
            .map(|name| {
                let (tmp, root, note) = regression_repo(name, contents);
                let document = render_preview(note, None).unwrap();
                let cid = |path: &str| cid_of(&document.attachments, &root.join(path));
                assert_eq!(
                    src_of(&document.html, "a"),
                    cid("notes/img/a.png"),
                    "{name}"
                );
                assert_eq!(
                    src_of(&document.html, "h"),
                    cid("notes/img/a.png"),
                    "{name}"
                );
                assert_eq!(src_of(&document.html, "b"), cid("images/b.png"), "{name}");
                assert!(document.html.contains(r#"href="other/""#), "{name}");
                assert_eq!(document.attachments.len(), 2, "{name}");
                drop(tmp);
                (
                    src_of(&document.html, "a").to_string(),
                    src_of(&document.html, "b").to_string(),
                )
            })
            .collect();
        assert_eq!(rendered[0], rendered[1]);
    }

    #[test]
    fn test_render_preview_root_readme_attaches_raw_html_images() {
        // The reported bug: a root README's `<img>` in an HTML block (a
        // centered hero image) was resolved against `/README/` and never
        // attached, while the markdown image beside it was.
        let contents = concat!(
            "<p align=\"center\">\n",
            "  <img src=\"docs/images/x.png\" width=\"900\" alt=\"hero\">\n",
            "</p>\n\n",
            "Inline <img src=\"docs/images/x.png\" alt=\"inline\"> in a paragraph.\n\n",
            "![b](docs/images/x.png)\n",
        );
        let (_tmp, root, note) = thumbnail_repo("README.md", contents);
        write_file(&root.join("docs/images/x.png"), b"png bytes");

        let document = render_preview(note, None).unwrap();
        let cid = cid_of(&document.attachments, &root.join("docs/images/x.png"));
        assert_eq!(src_of(&document.html, "hero"), cid);
        assert_eq!(src_of(&document.html, "inline"), cid);
        assert_eq!(src_of(&document.html, "b"), cid);
        assert_eq!(document.attachments.len(), 1, "{:?}", document.attachments);
    }

    #[test]
    fn test_render_preview_subfolder_raw_html_resolves_against_folder() {
        let contents = concat!(
            "<div>\n<img src=\"img/a.png\" alt=\"a\">\n",
            "<img src='../images/b.png' alt='b'>\n</div>\n",
        );
        let (_tmp, root, note) = regression_repo("notes/note.md", contents);
        let document = render_preview(note, None).unwrap();
        let cid = |path: &str| cid_of(&document.attachments, &root.join(path));

        assert_eq!(src_of(&document.html, "a"), cid("notes/img/a.png"));
        assert!(
            document
                .html
                .contains(&format!("src='{}'", cid("images/b.png"))),
            "{}",
            document.html
        );
        assert_eq!(document.attachments.len(), 2, "{:?}", document.attachments);
    }

    #[test]
    fn test_render_preview_root_readme_attaches_picture_srcset() {
        // The README's hero: dark mode reads the `<source srcset>`, which was
        // never attached, so the preview showed a broken image in dark mode.
        let contents = concat!(
            "<p align=\"center\">\n",
            "  <picture>\n",
            "    <source media=\"(prefers-color-scheme: dark)\" srcset=\"docs/images/screens/hero-dark.png\">\n",
            "    <img src=\"docs/images/screens/hero-light.png\" width=\"900\" alt=\"hero\">\n",
            "  </picture>\n",
            "</p>\n",
        );
        let (_tmp, root, note) = thumbnail_repo("README.md", contents);
        write_file(&root.join("docs/images/screens/hero-dark.png"), b"dark");
        write_file(&root.join("docs/images/screens/hero-light.png"), b"light");

        let document = render_preview(note, None).unwrap();
        let cid = |path: &str| cid_of(&document.attachments, &root.join(path));
        assert_eq!(
            src_of(&document.html, "hero"),
            cid("docs/images/screens/hero-light.png")
        );
        assert!(
            document.html.contains(&format!(
                r#"srcset="{}""#,
                cid("docs/images/screens/hero-dark.png")
            )),
            "{}",
            document.html
        );
        assert_eq!(document.attachments.len(), 2, "{:?}", document.attachments);
    }

    #[test]
    fn test_collect_attachments_srcset_partially_resolvable() {
        let (_tmp, root) = repo_with_file("img/a.png");
        let html = concat!(
            r#"<img srcset="img/a.png 1x, img/missing.png 2x,"#,
            "\n   ",
            r#"https://example.com/c.png 3x, ../../outside.png 4x">"#,
        );
        let (result, attachments) = collect_attachments(html, &root, "", "/");

        let cid = cid_of(&attachments, &root.join("img/a.png"));
        assert_eq!(
            result,
            format!(
                "<img srcset=\"{cid} 1x, img/missing.png 2x,\n   https://example.com/c.png 3x, ../../outside.png 4x\">"
            )
        );
        assert_eq!(attachments.len(), 1);
    }

    #[test]
    fn test_collect_attachments_srcset_single_quoted_and_deduped_with_src() {
        let (_tmp, root) = repo_with_file("img/a.png");
        let html = r#"<picture><source srcset='img/a.png 640w'><img src="img/a.png" srcset="/img/a.png 2x"></picture>"#;
        let (result, attachments) = collect_attachments(html, &root, "", "/");

        assert_eq!(attachments.len(), 1, "{attachments:?}");
        let cid = cid_of(&attachments, &root.join("img/a.png"));
        assert_eq!(
            result,
            format!(
                r#"<picture><source srcset='{cid} 640w'><img src="{cid}" srcset="{cid} 2x"></picture>"#
            )
        );
    }

    #[test]
    fn test_collect_attachments_srcset_nothing_local_is_untouched() {
        let (_tmp, root) = repo_with_file("img/a.png");
        let html = r#"<img srcset="https://x/a.png 1x, img/none.png 2x">"#;
        let (result, attachments) = collect_attachments(html, &root, "", "/");
        assert_eq!(result, html);
        assert!(attachments.is_empty());
    }

    #[test]
    fn test_render_preview_relative_symlink_out_of_repo_not_attached() {
        let (tmp, _root, note) = regression_repo("notes/note.md", "![l](img/link.png)\n");
        std::os::unix::fs::symlink(
            tmp.path().join("outside.png"),
            tmp.path().join("repo/notes/img/link.png"),
        )
        .unwrap();
        let document = render_preview(note, None).unwrap();
        assert!(
            document.attachments.is_empty(),
            "{:?}",
            document.attachments
        );
        assert_eq!(src_of(&document.html, "l"), "img/link.png");
    }

    #[test]
    fn test_thumbnail_page_attaches_relative_image() {
        let (_tmp, root, note) = regression_repo("notes/note.md", "# Hi\n\n![a](img/a.png)\n");
        let document = thumbnail_page(render_thumbnail(note, None).unwrap());
        assert_eq!(
            src_of(&document.html, "a"),
            cid_of(&document.attachments, &root.join("notes/img/a.png"))
        );
    }

    #[test]
    fn test_thumbnail_page_attaches_raw_html_image() {
        let contents = "# Hi\n\n<p align=\"center\"><img src=\"../images/b.png\" alt=\"b\"></p>\n";
        let (_tmp, root, note) = regression_repo("notes/note.md", contents);
        let document = thumbnail_page(render_thumbnail(note, None).unwrap());
        assert_eq!(
            src_of(&document.html, "b"),
            cid_of(&document.attachments, &root.join("images/b.png"))
        );
    }

    // MARK: thumbnails

    /// A repository (marked with `.mbr/`) holding `note.md` with `contents`.
    /// Returns the temp dir, the canonical repo root and the note's path.
    fn thumbnail_repo(note: &str, contents: &str) -> (tempfile::TempDir, PathBuf, String) {
        let temp_dir = tempfile::tempdir().unwrap();
        let repo = temp_dir.path().join("repo");
        std::fs::create_dir_all(repo.join(".mbr")).unwrap();
        let note_path = repo.join(note);
        std::fs::create_dir_all(note_path.parent().unwrap()).unwrap();
        std::fs::write(&note_path, contents).unwrap();
        let canonical_root = repo.canonicalize().unwrap();
        let note_path = note_path.to_str().unwrap().to_string();
        (temp_dir, canonical_root, note_path)
    }

    fn write_file(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// A person note the way the contacts docs write one, with a portrait
    /// beside it and relationships declared in both directions.
    const CONTACT_NOTE: &str = concat!(
        "---\n",
        "type: person\n",
        "title: Jane Doe\n",
        "image: jane.png\n",
        "company: \"[[Acme Corp]]\"\n",
        "emails:\n  work: jane@abc.example\n",
        "relationships:\n",
        "  - {type: spouse, to: \"[[John Doe]]\", label: second marriage}\n",
        "  - {type: parent, from: \"[[George]]\"}\n",
        "---\n",
        "Notes about Jane.\n",
    );

    /// The `<main>` element of a preview: assertions about titles must not see
    /// the inlined CSS and JavaScript.
    fn main_of(html: &str) -> &str {
        let start = html.find(r#"<main id="wrapper""#).expect("a <main>");
        let end = start + html[start..].find("</main>").expect("</main>");
        &html[start..end]
    }

    fn contact_repo(contents: &str) -> (tempfile::TempDir, PathBuf, String) {
        let (tmp, root, note) = thumbnail_repo("people/jane.md", contents);
        write_file(&root.join("people/jane.png"), b"portrait");
        write_file(&tmp.path().join("outside.png"), b"outside");
        (tmp, root, note)
    }

    #[test]
    fn test_contact_note_preview_renders_the_card_with_attached_portrait() {
        let (_tmp, root, note) = contact_repo(CONTACT_NOTE);
        let document = render_preview(note, None).unwrap();
        let html = main_of(&document.html);

        assert!(
            html.contains(r#"<header class="mbr-contact-card" data-kind="person">"#),
            "{html}"
        );
        // One title in the page body: the card's, since the note has no H1.
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains(r#"<h1 class="mbr-contact-card-name">Jane Doe</h1>"#));
        assert!(html.find("mbr-contact-card").unwrap() < html.find("Notes about Jane.").unwrap());
        assert!(html.contains(r#"<a href="mailto:jane@abc.example">jane@abc.example</a>"#));

        // The portrait, relative to the note's folder, is a cid: attachment.
        let portrait = root.join("people/jane.png");
        assert_eq!(
            src_of(html, "Jane Doe"),
            cid_of(&document.attachments, &portrait)
        );
        assert_eq!(document.attachments.len(), 1);

        // Own declarations, framed from this note, all plain text: there is no
        // index to resolve a neighbour (or the company) against.
        assert!(html.contains(r#"<li data-predicate="child"><span class="mbr-contact-card-label">Child</span><span class="mbr-contact-card-relation"><span>George</span></span></li>"#), "{html}");
        assert!(html.contains(r#"<span>John Doe</span> <span class="mbr-contact-card-relation-note">second marriage</span>"#));
        assert!(html.contains(r#"<span class="mbr-contact-card-label">Employer</span><span class="mbr-contact-card-relation"><span>Acme Corp</span>"#));
        assert!(
            html.contains("<span>Acme Corp</span></p>"),
            "company is text: {html}"
        );
        let card = &html[html.find("<header class=\"mbr-contact-card\"").unwrap()..];
        let card = &card[..card.find("</header>").unwrap()];
        assert!(
            !card.contains(r#"<a href="/"#) && !card.contains("&#x2F;people"),
            "{card}"
        );

        // Nothing that needs a server: the backlink chip is absent.
        assert!(!html.contains("<mbr-contact-backlinks"), "{html}");
        // The card's styles are in the inlined theme.
        assert!(document.html.contains(".mbr-contact-card-section"));
    }

    #[test]
    fn test_contact_card_portrait_outside_root_is_not_attached() {
        let (_tmp, _root, note) =
            contact_repo(&CONTACT_NOTE.replace("image: jane.png", "image: ../../outside.png"));
        let document = render_preview(note, None).unwrap();
        assert!(
            document.attachments.is_empty(),
            "{:?}",
            document.attachments
        );
        assert!(!src_of(&document.html, "Jane Doe").starts_with("cid:"));
    }

    #[test]
    fn test_contact_card_yields_headline_to_body_h1_and_tracking_switch() {
        let (_tmp, root, note) = contact_repo(
            &CONTACT_NOTE.replace("Notes about Jane.", "# Jane, in her own words\n\nNotes."),
        );
        let html = render_preview(note.clone(), None).unwrap().html;
        let html = main_of(&html);
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(!html.contains("mbr-contact-card-name"));
        assert!(html.contains("mbr-contact-card-relationships"));

        // `relationship_tracking = false` drops the section, as on the server.
        write_file(
            &root.join(".mbr/config.toml"),
            b"relationship_tracking = false\n",
        );
        let html = render_preview(note, None).unwrap().html;
        assert!(html.contains("mbr-contact-card"));
        assert!(!html.contains("mbr-contact-card-relationships"), "{html}");
    }

    #[test]
    fn test_non_contact_preview_has_no_card() {
        let (_tmp, _root, note) = contact_repo(&CONTACT_NOTE.replace("type: person", "type: note"));
        let html = render_preview(note, None).unwrap().html;
        let html = main_of(&html);
        assert!(!html.contains("mbr-contact-card"), "{html}");
        assert!(!html.contains("<h1"), "no generated title for a plain note");
    }

    #[test]
    fn test_contact_thumbnail_page_carries_the_card() {
        // No cover image (the portrait is not a cover when `image` is missing
        // or not an image), so the thumbnail is the rendered page, card included.
        let (_tmp, _root, note) = contact_repo(&CONTACT_NOTE.replace("image: jane.png\n", ""));
        let document = thumbnail_page(render_thumbnail(note, None).unwrap());
        assert!(document.html.contains("mbr-contact-card-name"));
    }

    fn thumbnail_image(document: ThumbnailDocument) -> String {
        match document {
            ThumbnailDocument::Image { path } => path,
            ThumbnailDocument::Page { .. } => panic!("expected a cover image, got a page"),
        }
    }

    fn thumbnail_page(document: ThumbnailDocument) -> PreviewDocument {
        match document {
            ThumbnailDocument::Page { document } => document,
            ThumbnailDocument::Image { path } => panic!("expected a page, got image {path}"),
        }
    }

    #[test]
    fn test_thumbnail_uses_relative_cover_image() {
        let (_tmp, root, note) =
            thumbnail_repo("posts/note.md", "---\ncover: images/cover.png\n---\n# Hi\n");
        write_file(&root.join("posts/images/cover.png"), b"png bytes");

        let path = thumbnail_image(render_thumbnail(note, None).unwrap());
        assert_eq!(
            path,
            root.join("posts/images/cover.png").to_str().unwrap(),
            "a relative cover resolves from the note's own directory"
        );
    }

    #[test]
    fn test_thumbnail_uses_root_relative_cover_image() {
        let (_tmp, root, note) = thumbnail_repo(
            "posts/note.md",
            "---\nimage: /images/cover.jpg\n---\n# Hi\n",
        );
        write_file(&root.join("images/cover.jpg"), b"jpg bytes");

        let path = thumbnail_image(render_thumbnail(note, None).unwrap());
        assert_eq!(path, root.join("images/cover.jpg").to_str().unwrap());
    }

    #[test]
    fn test_thumbnail_root_relative_cover_falls_back_to_static_folder() {
        let (_tmp, root, note) =
            thumbnail_repo("note.md", "---\ncover: /images/cover.webp\n---\n# Hi\n");
        write_file(&root.join("static/images/cover.webp"), b"webp bytes");

        let path = thumbnail_image(render_thumbnail(note, None).unwrap());
        assert_eq!(
            path,
            root.join("static/images/cover.webp").to_str().unwrap()
        );
    }

    #[test]
    fn test_thumbnail_cover_key_wins_over_image_and_falls_through() {
        let (_tmp, root, note) = thumbnail_repo(
            "note.md",
            "---\ncover: cover.png\nimage: portrait.png\n---\n# Hi\n",
        );
        write_file(&root.join("cover.png"), b"cover");
        write_file(&root.join("portrait.png"), b"portrait");
        let path = thumbnail_image(render_thumbnail(note.clone(), None).unwrap());
        assert_eq!(path, root.join("cover.png").to_str().unwrap());

        // An unusable `cover` is skipped, not fatal: `image` is tried next.
        std::fs::remove_file(root.join("cover.png")).unwrap();
        let path = thumbnail_image(render_thumbnail(note, None).unwrap());
        assert_eq!(path, root.join("portrait.png").to_str().unwrap());
    }

    #[test]
    fn test_thumbnail_cover_outside_repo_falls_back_to_page() {
        let (tmp, _root, note) =
            thumbnail_repo("note.md", "---\ncover: ../outside.png\n---\n# Hi\n");
        write_file(&tmp.path().join("outside.png"), b"png bytes");

        thumbnail_page(render_thumbnail(note, None).unwrap());
    }

    #[test]
    fn test_thumbnail_cover_symlink_out_of_repo_falls_back_to_page() {
        let (tmp, root, note) = thumbnail_repo("note.md", "---\ncover: cover.png\n---\n# Hi\n");
        let outside = tmp.path().join("outside.png");
        write_file(&outside, b"png bytes");
        std::os::unix::fs::symlink(&outside, root.join("cover.png")).unwrap();

        thumbnail_page(render_thumbnail(note, None).unwrap());
    }

    #[test]
    fn test_thumbnail_cover_symlink_to_non_image_falls_back_to_page() {
        // The extension is judged on the canonical name, not the authored one.
        let (_tmp, root, note) = thumbnail_repo("note.md", "---\ncover: cover.png\n---\n# Hi\n");
        write_file(&root.join("secret.txt"), b"not an image");
        std::os::unix::fs::symlink(root.join("secret.txt"), root.join("cover.png")).unwrap();

        thumbnail_page(render_thumbnail(note, None).unwrap());
    }

    #[test]
    fn test_thumbnail_non_image_cover_falls_back_to_page() {
        let (_tmp, root, note) = thumbnail_repo("note.md", "---\ncover: doc.pdf\n---\n# Hi\n");
        write_file(&root.join("doc.pdf"), b"%PDF");
        thumbnail_page(render_thumbnail(note, None).unwrap());

        // SVG is an image to a browser but not to ImageIO.
        let (_tmp, root, note) = thumbnail_repo("note.md", "---\ncover: art.svg\n---\n# Hi\n");
        write_file(&root.join("art.svg"), b"<svg/>");
        thumbnail_page(render_thumbnail(note, None).unwrap());
    }

    #[test]
    fn test_thumbnail_missing_or_empty_cover_falls_back_to_page() {
        let (_tmp, _root, note) = thumbnail_repo("note.md", "---\ncover: nope.png\n---\n# Hi\n");
        thumbnail_page(render_thumbnail(note, None).unwrap());

        let (_tmp, root, note) = thumbnail_repo("note.md", "---\ncover: empty.png\n---\n# Hi\n");
        write_file(&root.join("empty.png"), b"");
        thumbnail_page(render_thumbnail(note, None).unwrap());
    }

    #[test]
    fn test_thumbnail_oversized_cover_falls_back_to_page() {
        let (_tmp, root, note) = thumbnail_repo("note.md", "---\ncover: big.png\n---\n# Hi\n");
        let big = std::fs::File::create(root.join("big.png")).unwrap();
        // Sparse: costs no disk, but stats as one byte over the cap.
        big.set_len(MAX_ATTACHMENT_BYTES + 1).unwrap();

        thumbnail_page(render_thumbnail(note, None).unwrap());
    }

    #[test]
    fn test_thumbnail_remote_or_non_string_cover_falls_back_to_page() {
        for frontmatter in [
            "cover: https://example.com/cover.png",
            "cover: data:image/png;base64,AAAA",
            "cover: [a.png, b.png]",
            "cover: 42",
            "cover: \"   \"",
        ] {
            let (_tmp, _root, note) =
                thumbnail_repo("note.md", &format!("---\n{frontmatter}\n---\n# Hi\n"));
            thumbnail_page(render_thumbnail(note, None).unwrap());
        }
    }

    #[test]
    fn test_thumbnail_page_matches_preview_of_short_note() {
        // Under the cap a thumbnail page is the preview page, built with the
        // thumbnail's asset choices: same template, theme and attachments.
        let (_tmp, root, note) =
            thumbnail_repo("note.md", "# Title\n\nSome text.\n\n![x](/images/a.png)\n");
        write_file(&root.join("images/a.png"), b"png bytes");

        let page = thumbnail_page(render_thumbnail(note.clone(), None).unwrap());
        let preview = render_preview_with_config(note, None, THUMBNAIL_CONFIG).unwrap();
        assert_eq!(page.html, preview.html);
        assert_eq!(page.attachments, preview.attachments);
        assert_eq!(page.attachments.len(), 1);
    }

    #[test]
    fn test_thumbnail_page_leaves_out_mermaid() {
        let (_tmp, _root, note) = thumbnail_repo("note.md", "# Title\n");
        let page = thumbnail_page(render_thumbnail(note.clone(), None).unwrap());
        let preview = render_preview(note, None).unwrap();
        assert!(
            page.html.len() + 1024 * 1024 < preview.html.len(),
            "the thumbnail should not inline the mermaid library ({} vs {} bytes)",
            page.html.len(),
            preview.html.len()
        );
    }

    #[test]
    fn test_thumbnail_caps_long_note_at_a_line_boundary() {
        let line = "Lorem ipsum dolor sit amet, consectetur adipiscing elit.\n";
        let filler = line.repeat(MAX_THUMBNAIL_SOURCE_BYTES / line.len() + 10);
        let contents = format!("# Top of the note\n\n{filler}\nTHE-VERY-END\n");
        let (_tmp, _root, note) = thumbnail_repo("note.md", &contents);

        let page = thumbnail_page(render_thumbnail(note, None).unwrap());
        assert!(page.html.contains("Top of the note"));
        assert!(
            !page.html.contains("THE-VERY-END"),
            "text past the cap must not be rendered"
        );
    }

    #[test]
    fn test_thumbnail_keeps_whole_short_note() {
        let (_tmp, _root, note) = thumbnail_repo("note.md", "# Top\n\nmiddle\n\nTHE-VERY-END");
        let page = thumbnail_page(render_thumbnail(note, None).unwrap());
        assert!(page.html.contains("THE-VERY-END"));
    }

    #[test]
    fn test_cut_at_line_boundary() {
        assert_eq!(cut_at_line_boundary(b"one\ntwo\nthr"), b"one\ntwo\n");
        assert_eq!(cut_at_line_boundary(b"one\n"), b"one\n");
        // No line break at all: one long line is kept rather than dropped.
        assert_eq!(cut_at_line_boundary(b"no newline"), b"no newline");
        assert_eq!(cut_at_line_boundary(b""), b"");
        // A multi-byte character straddling the cap is cut whole.
        let cut = "héllo\nwörld".as_bytes();
        let cut = &cut[..cut.len() - 3];
        assert_eq!(
            std::str::from_utf8(cut_at_line_boundary(cut)).unwrap(),
            "héllo\n"
        );
    }

    #[test]
    fn test_thumbnail_rejects_non_markdown() {
        let (_tmp, root, _note) = thumbnail_repo("note.md", "# Hi\n");
        let script = root.join("script.py");
        write_file(&script, b"print('hi')\n");

        let error = render_thumbnail(script.to_str().unwrap().to_string(), None).unwrap_err();
        assert!(matches!(error, QuickLookError::MarkdownRenderError { .. }));
    }

    #[test]
    fn test_thumbnail_missing_file_is_an_error() {
        let error = render_thumbnail("/nonexistent/note.md".to_string(), None).unwrap_err();
        assert!(matches!(error, QuickLookError::FileReadError { .. }));
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
        let config = mbr_core::config::Config::read(&root).unwrap_or_default();
        eprintln!("=== Config static_folder: {:?} ===", config.static_folder);

        // Check if static folder exists
        let static_path = root.join(&config.static_folder);
        eprintln!(
            "=== Static folder exists: {} at {:?} ===",
            static_path.exists(),
            static_path
        );

        // Render and check output
        let document = render_preview(file_path.to_string(), None).unwrap();
        let html = &document.html;

        eprintln!("\n=== Attachments ===");
        for attachment in &document.attachments {
            eprintln!("cid:{} -> {}", attachment.id, attachment.path);
        }

        eprintln!("\n=== Image-related lines in HTML ===");
        for line in html.lines() {
            let line_lower = line.to_lowercase();
            if line_lower.contains("<img")
                || line_lower.contains("cid:")
                || line_lower.contains("/images/blog")
            {
                eprintln!("{}", line.trim());
            }
        }

        // Extract all src attributes
        eprintln!("\n=== All src= attributes ===");
        let re = regex::Regex::new(r#"src="([^"]+)""#).unwrap();
        for cap in re.captures_iter(html) {
            let src = &cap[1];
            if src.contains("images") || src.contains("cid:") {
                eprintln!("src=\"{}\"", src);
            }
        }
    }
}
