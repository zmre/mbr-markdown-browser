use serde::{Deserialize, Serialize, Serializer};
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
};

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};

use crate::errors::ConfigError;
use crate::task_query::IncludeFilter;

const DEFAULT_PORT: u16 = 5200;
const DEFAULT_OEMBED_TIMEOUT_MS: u64 = 500;
const DEFAULT_OEMBED_CACHE_SIZE: usize = 2 * 1024 * 1024; // 2 MB
/// Default budget for the in-memory media metadata cache: 64 MB.
///
/// Deliberately independent of [`DEFAULT_OEMBED_CACHE_SIZE`]: the oembed cache
/// holds short text metadata, while this one holds full JPEG cover images
/// (a PDF cover renders up to 1200 px wide). Sharing the 2 MB oembed budget fit
/// only a couple of dozen covers before FIFO eviction, and setting
/// `oembed_cache_size = 0` to turn off link previews silently disabled media
/// caching as well.
const DEFAULT_MEDIA_CACHE_SIZE: usize = 64 * 1024 * 1024; // 64 MB
const DEFAULT_UPLOAD_MAX_BYTES: usize = 25 * 1024 * 1024; // 25 MiB (26214400)

/// Serde default for [`Config::media_cache_size`].
fn default_media_cache_size() -> usize {
    DEFAULT_MEDIA_CACHE_SIZE
}

/// Configuration for a single sort field in multi-level sorting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SortField {
    /// Field to sort by: "title", "filename", "created", "modified", or any frontmatter field
    pub field: String,
    /// Sort order: "asc" or "desc"
    #[serde(default = "default_sort_order")]
    pub order: String,
    /// Comparison type: "string" or "numeric"
    #[serde(default = "default_sort_compare")]
    pub compare: String,
}

fn default_sort_order() -> String {
    "asc".to_string()
}

fn default_sort_compare() -> String {
    "string".to_string()
}

fn default_link_tracking() -> bool {
    true
}

fn default_relationship_tracking() -> bool {
    true
}

fn default_tasks_enabled() -> bool {
    true
}

fn default_tasks_stamp_done() -> bool {
    true
}

/// Default for [`Config::tasks_default_include`].
///
/// Checkboxes only, not the wire default of [`IncludeFilter::All`]: the panel
/// widens to `all` on its own when a tasks-only query comes back empty, so a
/// repository that uses `TODO:` markers and no checkboxes still opens on its
/// work — while one that uses both is not made to read them mixed together.
fn default_tasks_default_include() -> IncludeFilter {
    IncludeFilter::Tasks
}

/// Default markers that flag work as incomplete.
///
/// A marker is highlighted anywhere in a line, case-sensitively and with a
/// conditional word boundary on each side; see [`crate::tasks::MarkerRule`]. A
/// block that *starts* with one is wrapped whole, any other occurrence is
/// wrapped on its own, and the first wrapper on a source line also carries an
/// `id="mbr-marker-{line}"` deep-link anchor.
pub fn default_incomplete_markers() -> Vec<String> {
    vec![
        "TK".to_string(),
        "TODO:".to_string(),
        "FIXME".to_string(),
        "XXXX".to_string(),
    ]
}

fn default_build_tag_pages() -> bool {
    true
}

fn default_sidebar_style() -> String {
    "panel".to_string()
}

const DEFAULT_SIDEBAR_MAX_ITEMS: usize = 100;

fn default_sidebar_max_items() -> usize {
    DEFAULT_SIDEBAR_MAX_ITEMS
}

fn default_graph_depth() -> usize {
    2
}

/// Whether the native menu bar is shown in the window.
///
/// Only Linux ever consults this. macOS puts the menu in the system-wide bar
/// at the top of the screen, where it costs the window nothing, and Windows
/// treats an in-window menu bar as the native convention; on both, the value is
/// ignored. On Linux the bar is a `GtkMenuBar` packed into the window itself,
/// which under a tiling compositor with no global-menu protocol is a strip of
/// chrome above every page.
///
/// Hiding it costs discoverability and nothing else: `muda` attaches the
/// accelerator group to the *window*, not to the bar, so every shortcut keeps
/// working while the bar is hidden. See `browser::menu_bar_starts_visible`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MenuBarVisibility {
    /// Follow the platform convention: hidden on Linux, shown elsewhere.
    #[default]
    Auto,
    /// Always start with the bar visible.
    Always,
    /// Never show the bar. F10 does not reveal it.
    Never,
}

/// Serde default for [`Config::gui_menu_bar`].
fn default_gui_menu_bar() -> MenuBarVisibility {
    MenuBarVisibility::Auto
}

fn default_upload_max_bytes() -> usize {
    DEFAULT_UPLOAD_MAX_BYTES
}

/// Configuration for a tag source - a frontmatter field that contains tags.
///
/// # Examples
///
/// Basic tag source:
/// ```toml
/// tag_sources = [
///     { field = "tags" }
/// ]
/// ```
///
/// Tag source with custom labels:
/// ```toml
/// tag_sources = [
///     { field = "taxonomy.performers", label = "Performer", label_plural = "Performers" }
/// ]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TagSource {
    /// The frontmatter field to extract tags from.
    /// Supports dot-notation for nested fields (e.g., "taxonomy.tags").
    pub field: String,

    /// Singular label for the tag source (e.g., "Tag", "Performer").
    /// Auto-derived from field name if not specified.
    #[serde(default)]
    pub label: Option<String>,

    /// Plural label for the tag source (e.g., "Tags", "Performers").
    /// Auto-derived from field name if not specified.
    #[serde(default)]
    pub label_plural: Option<String>,
}

impl TagSource {
    /// Returns the singular label for this tag source.
    ///
    /// Priority:
    /// 1. Explicit `label` field
    /// 2. Title-cased field name (last segment for dot-notation)
    pub fn singular_label(&self) -> String {
        if let Some(ref label) = self.label {
            return label.clone();
        }

        // Extract last segment for dot-notation (taxonomy.tags -> tags)
        let field_name = self.field.rsplit('.').next().unwrap_or(&self.field);

        // Title case the field name
        title_case(field_name)
    }

    /// Returns the plural label for this tag source.
    ///
    /// Priority:
    /// 1. Explicit `label_plural` field
    /// 2. Singular label + "s"
    pub fn plural_label(&self) -> String {
        if let Some(ref label) = self.label_plural {
            return label.clone();
        }

        // Simple pluralization: add "s"
        format!("{}s", self.singular_label())
    }

    /// Returns the URL source identifier for this tag source.
    ///
    /// This is the normalized field name used in URLs.
    /// For dot-notation fields, uses the full path with dots (e.g., "taxonomy.performers").
    /// Lowercased for URL consistency and sanitized against path traversal.
    pub fn url_source(&self) -> String {
        crate::wikilink::sanitize_path_component(&self.field.to_lowercase())
    }
}

/// Simple title-case conversion for a field name.
///
/// Converts "tags" to "Tag", "performers" to "Performer", etc.
/// Removes trailing 's' for simple singular form.
fn title_case(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    // Remove trailing 's' for simple singular form
    let base = s.strip_suffix('s').unwrap_or(s);
    if base.is_empty() {
        return "S".to_string();
    }

    // Capitalize first letter
    let mut chars = base.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Returns the default tag sources configuration.
///
/// Default: a single source extracting from the "tags" frontmatter field.
pub fn default_tag_sources() -> Vec<TagSource> {
    vec![TagSource {
        field: "tags".to_string(),
        label: None,
        label_plural: None,
    }]
}

/// Converts tag sources to a HashSet of field names for wikilink matching.
///
/// The HashSet contains the field names from each TagSource, which are used
/// to detect valid tag link patterns like `[[Tags:rust]]` or `[text](tags:value)`.
pub fn tag_sources_to_set(sources: &[TagSource]) -> std::collections::HashSet<String> {
    sources.iter().map(|s| s.field.clone()).collect()
}

/// Converts tag sources to a Vec of URL source identifiers.
///
/// Each TagSource is converted to its lowercase URL identifier via `url_source()`.
/// This is used for path resolution to detect tag URLs like `/tags/rust/`.
pub fn tag_sources_to_url_sources(sources: &[TagSource]) -> Vec<String> {
    sources.iter().map(|s| s.url_source()).collect()
}

/// Configuration for a relation type used by typed note relationships.
///
/// A relation type names an edge predicate (e.g. "parent", "spouse") and
/// declares its semantics for automatic reverse-edge derivation:
/// - `symmetric = true` — the reverse reads the same (spouse, sibling).
/// - `inverse = Some("child")` — the reverse reads as the inverse (parent ↔
///   child). Symmetric and inverse are mutually exclusive.
///
/// Unknown relation types (not listed here) are tolerated: they are tracked
/// directed with no relabelling.
///
/// # Examples
///
/// ```toml
/// relationship_types = [
///     { name = "parent", inverse = "child", label = "Parent", label_plural = "Parents" },
///     { name = "child", inverse = "parent", label = "Child", label_plural = "Children" },
///     { name = "spouse", symmetric = true, label = "Spouse", label_plural = "Spouses" },
/// ]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RelationType {
    /// The relation-type name / predicate (e.g. "parent").
    pub name: String,

    /// Whether the relation is symmetric (the reverse reads the same).
    #[serde(default)]
    pub symmetric: bool,

    /// The inverse relation-type name, if this is one half of an inverse pair.
    #[serde(default)]
    pub inverse: Option<String>,

    /// Singular display label (e.g. "Parent"). Auto-derived from `name` if unset.
    #[serde(default)]
    pub label: Option<String>,

    /// Plural display label (e.g. "Parents"). Auto-derived from the singular
    /// label if unset. Set explicitly for irregular plurals (e.g. "Children").
    #[serde(default)]
    pub label_plural: Option<String>,
}

impl RelationType {
    /// Returns the singular label (explicit `label`, else title-cased `name`).
    pub fn singular_label(&self) -> String {
        self.label
            .clone()
            .unwrap_or_else(|| title_case_word(&self.name))
    }

    /// Returns the plural label (explicit `label_plural`, else singular + "s").
    pub fn plural_label(&self) -> String {
        self.label_plural
            .clone()
            .unwrap_or_else(|| format!("{}s", self.singular_label()))
    }
}

/// Title-cases a single word (capitalizes the first letter), preserving the
/// rest. Unlike [`title_case`], it does not strip a trailing plural "s".
fn title_case_word(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Returns the default relation types: genealogy defaults.
///
/// - `parent` ↔ `child` (inverse pair)
/// - `spouse` (symmetric)
/// - `sibling` (symmetric)
pub fn default_relationship_types() -> Vec<RelationType> {
    vec![
        RelationType {
            name: "parent".to_string(),
            symmetric: false,
            inverse: Some("child".to_string()),
            label: Some("Parent".to_string()),
            label_plural: Some("Parents".to_string()),
        },
        RelationType {
            name: "child".to_string(),
            symmetric: false,
            inverse: Some("parent".to_string()),
            label: Some("Child".to_string()),
            label_plural: Some("Children".to_string()),
        },
        RelationType {
            name: "spouse".to_string(),
            symmetric: true,
            inverse: None,
            label: Some("Spouse".to_string()),
            label_plural: Some("Spouses".to_string()),
        },
        RelationType {
            name: "sibling".to_string(),
            symmetric: true,
            inverse: None,
            label: Some("Sibling".to_string()),
            label_plural: Some("Siblings".to_string()),
        },
    ]
}

impl Default for SortField {
    fn default() -> Self {
        Self {
            field: "title".to_string(),
            order: default_sort_order(),
            compare: default_sort_compare(),
        }
    }
}

/// Returns the default sort configuration: title ascending, string comparison.
pub fn default_sort_config() -> Vec<SortField> {
    vec![SortField::default()]
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IpArray(pub [u8; 4]);

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    pub root_dir: PathBuf,
    pub host: IpArray,
    pub port: u16,
    pub static_folder: String,
    pub markdown_extensions: Vec<String>,
    pub theme: String,
    pub index_file: String,
    pub ignore_dirs: Vec<String>,
    pub ignore_globs: Vec<String>,
    /// Directories to ignore in the file watcher. These directories will not trigger
    /// live reload events when files inside them change.
    pub watcher_ignore_dirs: Vec<String>,
    /// Timeout in milliseconds for fetching oembed/OpenGraph metadata from URLs.
    /// If the fetch doesn't complete in time, falls back to a plain link.
    /// Set to 0 to disable oembed fetching entirely (uses plain links for all URLs
    /// except YouTube and Giphy which are embedded without network calls).
    pub oembed_timeout_ms: u64,
    /// Maximum size in bytes for the oembed cache. The cache stores fetched page
    /// metadata to avoid redundant network requests when rendering multiple files.
    /// Set to 0 to disable caching entirely. Default: 2MB (2097152 bytes).
    pub oembed_cache_size: usize,
    /// Maximum size in bytes for the media metadata cache (server/GUI mode).
    ///
    /// Holds dynamically generated video/PDF metadata: cover images (JPEG
    /// payloads), chapters and captions. Sized separately from
    /// `oembed_cache_size` because a single cover can be hundreds of kilobytes,
    /// so the 2 MB oembed budget evicted covers almost immediately — and
    /// turning oembed caching off must not turn media caching off with it.
    /// Set to 0 to disable caching entirely. Default: 64MB (67108864 bytes).
    #[serde(default = "default_media_cache_size")]
    pub media_cache_size: usize,
    /// Optional template folder that overrides the default .mbr/ and compiled defaults.
    /// Files found here take precedence; missing files fall back to compiled defaults.
    #[serde(default)]
    pub template_folder: Option<PathBuf>,
    /// Sort configuration for file listings. Supports multi-level sorting by any field.
    /// Default: sort by title (falling back to filename), ascending, string comparison.
    #[serde(default = "default_sort_config")]
    pub sort: Vec<SortField>,
    /// Build concurrency: number of files to process in parallel during static builds.
    /// None = auto-detect based on CPU cores (2x cores, capped at 32).
    #[serde(default)]
    pub build_concurrency: Option<usize>,
    /// Enable dynamic video transcoding to serve lower-resolution variants (720p, 480p).
    /// Only active in server/GUI mode. Videos are transcoded on-demand as HLS segments
    /// and cached in memory. Default: false (disabled).
    #[serde(default)]
    pub transcode: bool,
    /// Skip internal link validation during static site builds.
    /// When true, the build will not check if internal links point to valid files.
    /// Default: false (link checking enabled).
    #[serde(default)]
    pub skip_link_checks: bool,
    /// Enable bidirectional link tracking (backlinks).
    /// When enabled, generates links.json endpoints/files for each page with inbound/outbound links.
    /// Server mode: lazy grep-based discovery on-demand with caching.
    /// Build mode: eager collection during render, inverted for inbound links.
    /// Default: true (enabled).
    #[serde(default = "default_link_tracking")]
    pub link_tracking: bool,
    /// Tag sources configuration for extracting tags from frontmatter fields.
    /// Supports dot-notation for nested fields (e.g., "taxonomy.tags").
    /// Default: extract from "tags" field.
    #[serde(default = "default_tag_sources")]
    pub tag_sources: Vec<TagSource>,
    /// Enable typed relationship tracking (named frontmatter relationships).
    /// When enabled, per-note relationships are exposed via links.json and
    /// site.json, and rendered in the info panel.
    /// Default: true (enabled). Cheap when unused.
    #[serde(default = "default_relationship_tracking")]
    pub relationship_tracking: bool,
    /// Relation types configuration for typed note relationships.
    /// Declares symmetric / inverse-pair semantics and display labels.
    /// Default: genealogy defaults (parent↔child, spouse, sibling).
    #[serde(default = "default_relationship_types")]
    pub relationship_types: Vec<RelationType>,
    /// Generate tag landing pages during static site builds.
    /// When enabled, creates /{source}/{value}/ pages for each tag value
    /// and /{source}/ index pages listing all tags.
    /// Default: true (enabled).
    #[serde(default = "default_build_tag_pages")]
    pub build_tag_pages: bool,
    /// Sidebar navigation style.
    /// - "panel": Three-pane modal browser (default, existing mbr-browse)
    /// - "single": Persistent single-column sidebar (new mbr-browse-single)
    #[serde(default = "default_sidebar_style")]
    pub sidebar_style: String,
    /// Maximum items per section in sidebar navigation.
    /// Default: 100. Only applies when sidebar_style = "single".
    #[serde(default = "default_sidebar_max_items")]
    pub sidebar_max_items: usize,
    /// Depth (hops) of the link/relationship neighborhood shown in the
    /// sidebar mini graph. Range 1-5.
    /// Default: 2.
    #[serde(default = "default_graph_depth")]
    pub graph_depth: usize,
    /// Whether the GUI window shows the native menu bar. `"auto"` (default)
    /// follows the platform convention — hidden on Linux, shown on macOS and
    /// Windows; `"always"` and `"never"` pin it.
    ///
    /// GUI mode only, and only Linux reads it; see [`MenuBarVisibility`] for
    /// why. On Linux, F10 toggles the bar at runtime unless this is `"never"`.
    #[serde(default = "default_gui_menu_bar")]
    pub gui_menu_bar: MenuBarVisibility,
    /// Text to prepend to all page titles (e.g., "My Site: ").
    /// Default: empty string (no prefix).
    #[serde(default)]
    pub title_prefix: String,
    /// Text to append to all page titles (e.g., " | My Site").
    /// Default: empty string (no suffix).
    #[serde(default)]
    pub title_suffix: String,
    /// Markers that flag work as incomplete, matched anywhere in a line and
    /// wrapped in `<span class="mbr-incomplete">…</span>`. A paragraph,
    /// heading, list item, or table cell whose *first* text starts with one is
    /// wrapped whole. Never matched inside code, image alt text, link
    /// destinations or frontmatter.
    /// Default: `["TK", "TODO", "FIXME", "XXX"]`.
    #[serde(default = "default_incomplete_markers")]
    pub incomplete_markers: Vec<String>,
    /// Enable incomplete-block marking. `None` = mode default (on for
    /// server/GUI, off for static build). CLI flags
    /// `--mark-incomplete` / `--no-mark-incomplete` force a value.
    #[serde(default)]
    pub mark_incomplete: Option<bool>,
    /// Enable the task browser (`POST /.mbr/tasks` and the task panel).
    ///
    /// Server/GUI only: the tasks index is built from live files, so static
    /// builds never expose it regardless of this value. When disabled, the
    /// endpoint returns 404 and the frontend does not offer the panel.
    /// Default: true (enabled). CLI flag `--no-tasks` disables it.
    #[serde(default = "default_tasks_enabled")]
    pub tasks_enabled: bool,
    /// Maintain the `@done(YYYY-MM-DD HH:MM)` annotation when a task's status
    /// is toggled through `POST /.mbr/task`: stamp it on completion, remove it
    /// when the task is reopened or canceled.
    ///
    /// Turn it off to have that endpoint rewrite nothing but the marker byte,
    /// leaving any `@done(...)` exactly as its author wrote it. Nothing else in
    /// mbr ever writes to a markdown file on its own.
    /// Default: true (stamp).
    #[serde(default = "default_tasks_stamp_done")]
    pub tasks_stamp_done: bool,
    /// Which entries the task panel's **Show** filter starts on: `"tasks"`
    /// (checkboxes), `"markers"` (`TODO:`-style lines), or `"all"`.
    ///
    /// Only the starting position — the user can change it in the ⚙ popover for
    /// the life of that panel, and the panel widens to `all` by itself when the
    /// configured default returns nothing (see `mbr-tasks-panel.ts`). That
    /// fallback is why the default is the narrow `"tasks"` rather than `"all"`:
    /// a repository with no checkboxes at all still opens on its markers, and
    /// one with both is not made to read them interleaved.
    ///
    /// Typed as the real [`IncludeFilter`] rather than a `String` so serde
    /// rejects a typo at startup, the way an out-of-range `graph_depth` is.
    /// Config file and `MBR_TASKS_DEFAULT_INCLUDE` only; there is no CLI flag.
    /// Default: `"tasks"`.
    #[serde(default = "default_tasks_default_include")]
    pub tasks_default_include: IncludeFilter,
    /// Glob patterns whose matching files are left out of the **task index**.
    ///
    /// Each pattern is matched against a markdown file's repository-relative
    /// path, always `/`-separated (`docs/templates/onboarding.md`). A file that
    /// matches any pattern contributes nothing to the task browser: no tasks, no
    /// folder in the folder pane, no counts.
    ///
    /// The use case is a folder of checklist *templates*, which is full of
    /// deliberately unchecked boxes that are nobody's actual work. Rendering is
    /// untouched — those files stay readable and their checkboxes stay
    /// clickable, because in-document checkboxes exist whether or not the task
    /// browser does.
    ///
    /// Patterns name **files**, not folders, so exclude a folder's contents
    /// (`templates/**`) rather than the folder (`templates`, which matches
    /// nothing). Default: empty, so nothing is excluded.
    ///
    /// Config file and `MBR_TASKS_IGNORE_GLOBS` only; there is no CLI flag.
    #[serde(default)]
    pub tasks_ignore_globs: Vec<String>,
    /// Enable the in-browser markdown editing endpoints in server/GUI mode:
    /// `/.mbr/raw` + `/.mbr/edit` (read/save an existing file) and the
    /// file-management endpoints `/.mbr/create` (new file), `/.mbr/move`
    /// (move/rename with repo-wide link rewrite), and `/.mbr/mkdir` (new
    /// folder). Off by default. Intended for private use (e.g. GUI on
    /// localhost). CLI flag `--edit` also enables it.
    #[serde(default)]
    pub edit_enabled: bool,
    /// Argon2 PHC hash of the shared editing token. Required when editing is
    /// enabled on a non-loopback host; requests must present the matching token
    /// as `Authorization: Bearer <token>`. Generate with
    /// `mbr --generate-edit-token`. Never sent to the frontend.
    #[serde(default)]
    pub edit_token_hash: Option<String>,
    /// Require the editing token even for loopback callers. Off by default:
    /// loopback edits are allowed without a token (still CSRF-protected).
    #[serde(default)]
    pub edit_require_token_on_loopback: bool,
    /// Maximum size in bytes for a single asset uploaded via `/.mbr/upload`
    /// (the in-browser editor's image uploader). Requests with a larger body are
    /// rejected with `413 Payload Too Large`. Default: 25 MiB (26214400 bytes).
    #[serde(default = "default_upload_max_bytes")]
    pub upload_max_bytes: usize,
}

impl std::fmt::Display for IpArray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [a, b, c, d] = self.0;
        write!(f, "{a}.{b}.{c}.{d}")
    }
}

impl<'de> Deserialize<'de> for IpArray {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let ip_str = String::deserialize(deserializer)?;
        let ip: IpAddr = ip_str.parse().map_err(serde::de::Error::custom)?;

        match ip {
            IpAddr::V4(v4) => Ok(IpArray(v4.octets())),
            IpAddr::V6(_) => Err(serde::de::Error::custom("IPv6 addresses are not supported")),
        }
    }
}

impl Serialize for IpArray {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let ip = std::net::Ipv4Addr::from(self.0);
        serializer.serialize_str(&ip.to_string())
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            root_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            host: IpArray([127, 0, 0, 1]),
            port: DEFAULT_PORT,
            static_folder: "static".to_string(),
            markdown_extensions: vec!["md".to_string()],
            theme: "default".to_string(),
            index_file: "index.md".to_string(),
            ignore_dirs: [
                "target",
                "result",
                "build",
                "node_modules",
                "ci",
                "templates",
                ".git",
                ".github",
                "dist",
                "out",
                "coverage",
            ]
            .into_iter()
            .map(|x| x.to_string())
            .collect(),
            ignore_globs: [
                "*.log", "*.bak", "*.lock", "*.sh", "*.css", "*.scss", "*.js", "*.ts",
            ]
            .into_iter()
            .map(|x| x.to_string())
            .collect(),
            watcher_ignore_dirs: [".direnv", ".git", "result", "target", "build"]
                .into_iter()
                .map(|x| x.to_string())
                .collect(),
            oembed_timeout_ms: DEFAULT_OEMBED_TIMEOUT_MS,
            oembed_cache_size: DEFAULT_OEMBED_CACHE_SIZE,
            media_cache_size: DEFAULT_MEDIA_CACHE_SIZE,
            template_folder: None,
            sort: default_sort_config(),
            build_concurrency: None, // Auto-detect based on CPU cores
            transcode: false,        // Disabled by default
            skip_link_checks: false, // Link checking enabled by default
            link_tracking: true,     // Bidirectional link tracking enabled by default
            tag_sources: default_tag_sources(),
            relationship_tracking: true, // Typed relationship tracking enabled by default
            relationship_types: default_relationship_types(),
            build_tag_pages: true, // Tag pages enabled by default
            sidebar_style: default_sidebar_style(),
            sidebar_max_items: default_sidebar_max_items(),
            graph_depth: default_graph_depth(),
            gui_menu_bar: default_gui_menu_bar(),
            title_prefix: String::new(),
            title_suffix: String::new(),
            incomplete_markers: default_incomplete_markers(),
            mark_incomplete: None,
            tasks_enabled: true, // Task browser enabled by default (server/GUI only)
            tasks_stamp_done: true, // Stamp @done(...) when a task is completed
            tasks_default_include: default_tasks_default_include(),
            tasks_ignore_globs: Vec::new(), // Opt-in: index every file's tasks by default
            edit_enabled: false,
            edit_token_hash: None,
            edit_require_token_on_loopback: false,
            upload_max_bytes: DEFAULT_UPLOAD_MAX_BYTES,
        }
    }
}

/// The user's home directory, read from the variable the platform actually sets.
///
/// Windows does not set `HOME`; the equivalent is `USERPROFILE`. Without this
/// the guards below never fire there, so a stray `C:\Users\you\.git` or
/// `.obsidian` would silently make the entire home directory the repo root and
/// trigger a scan of everything the user owns.
fn home_dir() -> Option<PathBuf> {
    let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(home_var).map(PathBuf::from)
}

/// Returns true if the given path is the user's home directory.
///
/// Compares canonical forms when both sides resolve, because callers do not
/// agree on path shape: [`find_root_dir`] walks raw ancestors, while
/// [`static_folder_anchor`] compares an already-canonicalized root. Without
/// this, a `$HOME` reached through a symlink (`/home/me` -> `/mnt/users/me`)
/// would slip past both guards.
fn is_home_dir(path: &Path) -> bool {
    let Some(home) = home_dir() else {
        return false;
    };
    if path == home {
        return true;
    }
    match (path.canonicalize(), home.canonicalize()) {
        (Ok(path), Ok(home)) => path == home,
        _ => false,
    }
}

/// True when `path` is the home directory *or* a directory that contains it.
///
/// [`is_home_dir`] on its own was enough while the overlay could climb only one
/// level: a root whose parent is `/home` would have to be `/home/notes`, which
/// nobody has. At two levels it is `/home/you/notes` — an ordinary layout whose
/// grandparent holds every account on the machine — and `is_home_dir` does not
/// fire on `/home`, because `/home` is not `$HOME`. Ancestors of `$HOME`
/// (`/home`, `/Users`, `C:\Users`) are therefore refused as an anchor too.
///
/// Nothing here helps when `HOME`/`USERPROFILE` is unset, as it can be for a
/// service account started without a login session; the filesystem-root guard
/// in [`static_folder_anchor`] is the only backstop then.
fn contains_home_dir(path: &Path) -> bool {
    if is_home_dir(path) {
        return true;
    }
    // `starts_with("")` is true of every path, and an empty parent is how a
    // *relative* root bottoms out — that case belongs to the filesystem-root
    // guard, not to this one.
    if path.as_os_str().is_empty() {
        return false;
    }
    let Some(home) = home_dir() else {
        return false;
    };
    if home.starts_with(path) {
        return true;
    }
    match (path.canonicalize(), home.canonicalize()) {
        (Ok(path), Ok(home)) => home.starts_with(path),
        _ => false,
    }
}

/// Search upward from the given path to find a repository root directory.
///
/// Searches for directory markers (`.mbr`, `.git`, `.zk`, `.obsidian`) then
/// file markers (`book.toml`, `mkdocs.yml`, `docusaurus.config.js`) in ancestor
/// directories. Falls back to the start path's directory if no markers found.
///
/// Skips matches at `$HOME` to avoid using the entire home directory as root
/// (e.g., when `~/.git` exists for a dotfiles repo).
pub fn find_root_dir(start_path: &Path) -> PathBuf {
    const DIR_MARKERS: &[&str] = &[".mbr", ".git", ".zk", ".obsidian"];
    const FILE_MARKERS: &[&str] = &["book.toml", "mkdocs.yml", "docusaurus.config.js"];

    let dir = if start_path.is_dir() {
        start_path
    } else {
        start_path.parent().unwrap_or(start_path)
    };

    for marker in DIR_MARKERS {
        if let Some(root) = dir
            .ancestors()
            .find(|a| a.join(marker).is_dir())
            .map(|p| p.to_path_buf())
        {
            // `continue`, not `break`: only *this* marker is disqualified. A
            // dotfiles `~/.git` must not stop the search for the `.obsidian`
            // that marks the actual vault further down.
            if is_home_dir(&root) {
                continue;
            }
            return root;
        }
    }

    for marker in FILE_MARKERS {
        if let Some(root) = dir
            .ancestors()
            .find(|a| a.join(marker).is_file())
            .map(|p| p.to_path_buf())
        {
            if is_home_dir(&root) {
                continue;
            }
            return root;
        }
    }

    dir.to_path_buf()
}

impl Config {
    /// Loads configuration for the repository containing `search_config_from`.
    ///
    /// Layers, lowest precedence first (figment's `merge` gives the *later*
    /// provider precedence):
    ///
    /// 1. Compiled-in defaults ([`Config::default`])
    /// 2. `.mbr/config.toml` in the discovered root
    /// 3. `MBR_*` environment variables
    ///
    /// Environment variables deliberately win over the config file: the file
    /// ships inside the markdown repository, so an operator serving a repo they
    /// did not author must be able to override it from the outside.
    pub fn read(search_config_from: &Path) -> Result<Self, crate::MbrError> {
        let default_config = Config::default();
        let root_dir = find_root_dir(search_config_from);
        // Kept rather than discarded: figment is the only thing that knows which
        // layer supplied each key, and `static_folder` is trusted differently
        // depending on that (see `reject_repo_supplied_absolute_static_folder`).
        let figment = Figment::new()
            .merge(Serialized::defaults(default_config))
            .merge(Toml::file(root_dir.join(".mbr/config.toml")))
            .merge(Env::prefixed("MBR_"));
        let mut config: Config = figment
            .extract()
            .map_err(|e| ConfigError::ParseFailed(Box::new(e)))?;
        tracing::debug!("Loaded config: {:?}", &config);
        config.root_dir = root_dir;
        config.reject_repo_supplied_absolute_static_folder(&figment)?;
        config.validate()?;
        config.log_external_static_folder();
        Ok(config)
    }

    /// Refuses an absolute `static_folder` that came from the repository's own
    /// `.mbr/config.toml`.
    ///
    /// [`Config::validate`] cannot make this call, and deliberately does not
    /// try: the identical value is legitimate from `MBR_STATIC_FOLDER`, which
    /// only whoever runs the server can set, and a `Config` carries no record of
    /// where its fields came from. Figment does — [`Figment::find_metadata`]
    /// returns the metadata of the provider that supplied the *winning* value
    /// for a key, and only the [`Toml`] provider reports a
    /// [`Source::File`](figment::Source::File). The env provider reports no
    /// source at all, so "has a file source" is exactly "came from the repo".
    fn reject_repo_supplied_absolute_static_folder(
        &self,
        figment: &Figment,
    ) -> Result<(), ConfigError> {
        if !is_rooted(Path::new(&self.static_folder)) {
            return Ok(());
        }

        let from_file = figment
            .find_metadata("static_folder")
            .and_then(|metadata| metadata.source.as_ref())
            .and_then(figment::Source::file_path);

        match from_file {
            Some(source) => Err(invalid_static_folder(
                &self.static_folder,
                &format!(
                    "an absolute path is only honored from the MBR_STATIC_FOLDER environment \
                     variable, not from a repository config file ({})",
                    source.display()
                ),
            )),
            None => Ok(()),
        }
    }

    /// Emits one line when the static overlay lands outside the markdown root,
    /// so an external static root is never silently in effect.
    ///
    /// A *peer* overlay is an ordinary layout and only rates INFO. Reaching two
    /// levels up is rare and much wider — the anchor is the root's grandparent,
    /// so the value could have named any directory under it — so that case is
    /// raised to WARN, which is visible at the default log level.
    fn log_external_static_folder(&self) {
        if let Ok(StaticOverlay::External(dir)) =
            resolve_static_overlay(&self.root_dir, &self.static_folder)
        {
            let within_peer = self
                .root_dir
                .parent()
                .is_some_and(|parent| dir.starts_with(parent));
            if within_peer {
                tracing::info!(
                    "static_folder {:?} resolves outside the markdown root ({}): serving and indexing assets from {}",
                    self.static_folder,
                    self.root_dir.display(),
                    dir.display()
                );
            } else {
                tracing::warn!(
                    "static_folder {:?} resolves two levels above the markdown root ({}): serving and indexing assets from {}",
                    self.static_folder,
                    self.root_dir.display(),
                    dir.display()
                );
            }
        }
    }

    /// Validates the configuration values.
    ///
    /// Checks that numeric configuration options are within valid bounds:
    /// - `port`: Must be 1-65535 (port 0 means "auto-assign", which isn't useful for display)
    /// - `sidebar_max_items`: Must be > 0
    /// - `graph_depth`: Must be between 1 and 5
    /// - `build_concurrency`: If set, must be > 0
    /// - `static_folder`: Must stay inside `root_dir`, or land under a directory
    ///   at most [`MAX_STATIC_FOLDER_ASCENT`] levels above it (see
    ///   [`Config::validate_static_folder`])
    /// - `tasks_ignore_globs`: Every pattern must compile
    ///
    /// Note: `oembed_cache_size` of 0 is valid (disables caching).
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.validate_static_folder()?;

        // Port 0 means "let OS pick a port" which isn't useful for a server URL
        if self.port == 0 {
            return Err(ConfigError::InvalidPort { port: self.port });
        }

        // sidebar_max_items of 0 would show no items
        if self.sidebar_max_items == 0 {
            return Err(ConfigError::InvalidSidebarMaxItems {
                value: self.sidebar_max_items,
            });
        }

        // graph_depth of 0 would show nothing; more than 5 hops fans out into
        // an unreadable graph (and an explosive number of links.json fetches)
        if !(1..=5).contains(&self.graph_depth) {
            return Err(ConfigError::InvalidGraphDepth {
                value: self.graph_depth,
            });
        }

        // build_concurrency of 0 would mean no parallelism (None means auto-detect)
        if matches!(self.build_concurrency, Some(0)) {
            return Err(ConfigError::InvalidBuildConcurrency { value: 0 });
        }

        // Task-ignore patterns are compiled once, at `TaskIndex` construction,
        // and a pattern that fails to compile there is dropped with a log line
        // nobody reads — silently indexing exactly the folder it was meant to
        // exclude. Fail here instead, while there is still a startup to abort.
        if let Some((pattern, reason)) = self.tasks_ignore_globs.iter().find_map(|pattern| {
            glob::Pattern::new(pattern)
                .err()
                .map(|e| (pattern.clone(), e.to_string()))
        }) {
            return Err(ConfigError::InvalidTasksIgnoreGlob { pattern, reason });
        }

        // Refuse to expose an unauthenticated writable endpoint to the network:
        // editing on a non-loopback host requires a token hash.
        if self.edit_enabled
            && !std::net::Ipv4Addr::from(self.host.0).is_loopback()
            && self.edit_token_hash.is_none()
        {
            return Err(ConfigError::EditingRequiresToken);
        }

        Ok(())
    }

    /// Rejects a `static_folder` that reaches further above the markdown root
    /// than [`MAX_STATIC_FOLDER_ASCENT`] levels, or that resolves to a directory
    /// containing the root. See [`resolve_static_overlay`] for the policy itself.
    fn validate_static_folder(&self) -> Result<(), ConfigError> {
        resolve_static_overlay(&self.root_dir, &self.static_folder).map(|_| ())
    }
}

/// Where the static overlay landed, once the policy has accepted it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaticOverlay {
    /// The overlay is disabled, or it resolves inside the markdown root.
    WithinRoot,
    /// The overlay resolves *outside* the markdown root and is permitted there.
    /// Carries the resolved (canonical, when it exists) directory.
    External(PathBuf),
}

/// Applies the `static_folder` policy, returning where the overlay resolved.
///
/// `static_folder` is merged from the repository's own `.mbr/config.toml`, so
/// for an untrusted repo it is attacker-controlled. The path resolver joins it
/// onto the root and serves files found beneath it, and the repo scanner indexes
/// it, so an unrestricted value turns the server into an arbitrary-file reader.
///
/// The policy, in order:
/// - **Empty**: the overlay is disabled — [`StaticOverlay::WithinRoot`].
/// - **Inside the root**: accepted (the `static/` default).
/// - **Rooted/absolute**: accepted, and rejected earlier by
///   [`Config::reject_repo_supplied_absolute_static_folder`] when it came from a
///   repository config file. A value that survives to here was set by the
///   operator via `MBR_STATIC_FOLDER` and is trusted as given.
/// - **Outside the root**: accepted only as a strict descendant of the *anchor*
///   — the directory at most [`MAX_STATIC_FOLDER_ASCENT`] levels above the root
///   that [`static_folder_anchor`] can reach without stepping into `$HOME` or
///   the filesystem root. One level makes the common `repo/content` +
///   `repo/static` layout work; two makes a framework layout work, where the
///   markdown root is pinned by the framework (SvelteKit's `<project>/src/routes`)
///   and the assets live at `<project>/static`.
/// - **Never an ancestor of the root**, even one the anchor would otherwise
///   admit. At two levels the root's own parent has become a legal descendant of
///   the anchor, so without this `static_folder = ".."` would be accepted — and
///   it serves every sibling of the root plus the markdown source itself as raw
///   static files.
///
/// The resolved directory is canonicalized when it exists, so a `static -> /etc`
/// symlink is judged by where it actually lands rather than by how innocent it
/// looks lexically. When it does not exist yet the resolution falls back to
/// lexical normalization, so an escape whose target has not been created is
/// caught too.
///
/// What two levels *does* concede, and what the `warn!` in
/// [`Config::log_external_static_folder`] exists to surface: an untrusted
/// `.mbr/config.toml` can now name any directory under the root's grandparent —
/// `<project>/.git` and its credentials, `<project>/node_modules`. One level
/// already reached `../.git`, so this widens an existing exposure rather than
/// opening a new class. The one genuinely new crossing is serving a repository
/// out of *another* user's home directory, which no `$HOME` check can catch
/// because `$HOME` is the operator's, not theirs.
///
/// This is the single definition of the policy on purpose: `Config::validate`
/// refuses a bad value at startup, and `Repo` uses the *same* call to decide
/// which second directory the scanner may descend into, so the two cannot drift
/// into the scanner accepting a root the validator would have refused.
pub fn resolve_static_overlay(
    root_dir: &Path,
    static_folder: &str,
) -> Result<StaticOverlay, ConfigError> {
    if static_folder.is_empty() {
        return Ok(StaticOverlay::WithinRoot);
    }

    let root = resolve_existing_or_lexical(root_dir);
    // `join` handles a rooted `static_folder` for us: an absolute path replaces
    // the base rather than being appended to it.
    let dir = resolve_existing_or_lexical(&root.join(static_folder));

    if dir.starts_with(&root) {
        return Ok(StaticOverlay::WithinRoot);
    }
    if is_rooted(Path::new(static_folder)) {
        return Ok(StaticOverlay::External(dir));
    }

    let (anchor, stop) = static_folder_anchor(&root);

    // When the climb was stopped on its first turn, `anchor` is the root itself
    // and nothing outside the root can satisfy this — which is exactly the
    // one-level behaviour for a root under `$HOME` or at the filesystem root.
    if !dir.starts_with(&anchor) {
        return Err(invalid_static_folder(static_folder, &stop.refusal(&anchor)));
    }
    // `dir == anchor` is implied by the ancestor test, since the anchor is an
    // ancestor of the root by construction, but it is spelled out so the rule
    // reads as "a strict descendant" without relying on that proof.
    if dir == anchor || root.starts_with(&dir) {
        return Err(invalid_static_folder(
            static_folder,
            "resolves to a directory that contains the markdown root, which would expose every \
             sibling of the root — and the markdown source itself; name a specific directory \
             instead",
        ));
    }

    Ok(StaticOverlay::External(dir))
}

/// How far above the markdown root an outside-the-root `static_folder` may reach.
///
/// One level covers `repo/content` + `repo/static`. Two covers the framework
/// layout this was raised for: SvelteKit puts routes at `<project>/src/routes`
/// (a route's filesystem path *is* its URL, so the markdown root has to live
/// there) while assets live at `<project>/static`.
///
/// Three is not on the table. At three levels the anchor for a repository in a
/// temp directory is the *system* temp directory — which is what
/// `test_validate_static_folder_symlink_escape_fails` relies on being out of
/// reach — and the same shape generalizes: a checkout's anchor becomes the
/// workspace folder holding every unrelated checkout.
const MAX_STATIC_FOLDER_ASCENT: usize = 2;

/// Why [`static_folder_anchor`] stopped climbing, so a refusal names the wall it
/// hit instead of reporting "outside the boundary" for four different reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AscentStop {
    /// Spent the whole ascent budget; the anchor is [`MAX_STATIC_FOLDER_ASCENT`]
    /// levels above the root.
    Budget,
    /// The markdown root has no parent — it *is* a filesystem root.
    RootHasNoParent,
    /// The next directory up is the filesystem root.
    FilesystemRoot,
    /// The next directory up is `$HOME`, or a directory containing it.
    HomeDir,
}

impl AscentStop {
    /// The reason text for a `static_folder` that landed outside `anchor`.
    ///
    /// Naming the anchor tells the operator what *would* have worked, which the
    /// old fixed-boundary messages could leave implicit because there was only
    /// ever one candidate.
    fn refusal(self, anchor: &Path) -> String {
        match self {
            Self::RootHasNoParent => {
                "resolves outside the markdown root, which has no parent directory".to_string()
            }
            Self::HomeDir => format!(
                "resolves outside {}, and the next directory up is the home directory",
                anchor.display()
            ),
            Self::FilesystemRoot => format!(
                "resolves outside {}, and the next directory up is the filesystem root",
                anchor.display()
            ),
            Self::Budget => format!(
                "resolves outside {}; an overlay may reach at most \
                 {MAX_STATIC_FOLDER_ASCENT} levels above the markdown root",
                anchor.display()
            ),
        }
    }
}

/// The highest directory an outside-the-root overlay may live under, and why the
/// climb stopped there.
///
/// Climbs at most [`MAX_STATIC_FOLDER_ASCENT`] levels, refusing at *every* level
/// to step into the filesystem root or into `$HOME` (or a directory containing
/// it). Checking at every level rather than only at the final anchor is what
/// makes the first turn of the loop reproduce the old one-level policy exactly,
/// so the ascent can only ever widen what was already accepted, never narrow it.
fn static_folder_anchor(root: &Path) -> (PathBuf, AscentStop) {
    let mut anchor = root;
    for _ in 0..MAX_STATIC_FOLDER_ASCENT {
        // Only reachable on the first turn: every later `anchor` is a directory
        // this loop has already proved has a parent.
        let Some(parent) = anchor.parent() else {
            return (anchor.to_path_buf(), AscentStop::RootHasNoParent);
        };
        // Tested before the home guard because `contains_home_dir` is true of
        // the filesystem root on every normal system — `$HOME` starts with `/`
        // — and "the next directory up is the home directory" is the wrong
        // thing to tell someone whose root is `/notes`.
        if parent.parent().is_none() {
            return (anchor.to_path_buf(), AscentStop::FilesystemRoot);
        }
        if contains_home_dir(parent) {
            return (anchor.to_path_buf(), AscentStop::HomeDir);
        }
        anchor = parent;
    }
    (anchor.to_path_buf(), AscentStop::Budget)
}

/// True when `path` begins at a filesystem root or a drive prefix.
///
/// Not `Path::is_absolute`: on Windows that is false for `/etc`, which is rooted
/// but prefixless, and a rooted path escapes the root just as thoroughly as a
/// fully-qualified one.
fn is_rooted(path: &Path) -> bool {
    use std::path::Component;
    matches!(
        path.components().next(),
        Some(Component::RootDir | Component::Prefix(_))
    )
}

/// Canonicalizes `path`, falling back to lexical normalization when it does not
/// exist. Never fails, so a not-yet-created path still gets a comparable form.
fn resolve_existing_or_lexical(path: &Path) -> PathBuf {
    path.canonicalize()
        .unwrap_or_else(|_| lexically_normalize(path))
}

/// Resolves `.` and `..` components without touching the filesystem.
///
/// Only sound for paths that are already canonical up to the missing tail, which
/// is how [`resolve_existing_or_lexical`] uses it: symlinks earlier in the path
/// have already been resolved, so popping a component cannot be misled by one.
fn lexically_normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    path.components()
        .fold(PathBuf::new(), |mut normalized, component| {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    // `pop` fails both at a filesystem root (where `..` is a
                    // no-op) and on an empty relative accumulator (where `..`
                    // must be preserved). Only the latter keeps the component.
                    if !normalized.pop() && !is_rooted(&normalized) {
                        normalized.push(Component::ParentDir);
                    }
                }
                other => normalized.push(other),
            }
            normalized
        })
}

/// Builds the error for a rejected `static_folder` value.
///
/// `ConfigError` has no dedicated variant for this, so the message is carried
/// by a synthetic `figment::Error` inside `ParseFailed` (see the followup note
/// in the review: a `ConfigError::InvalidStaticFolder` variant belongs in
/// `errors.rs`).
fn invalid_static_folder(value: &str, reason: &str) -> ConfigError {
    ConfigError::ParseFailed(Box::new(figment::Error::from(format!(
        "Invalid static_folder {value:?}: {reason}"
    ))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_title_case() {
        assert_eq!(title_case("tags"), "Tag");
        assert_eq!(title_case("performers"), "Performer");
        assert_eq!(title_case("category"), "Category");
        assert_eq!(title_case("Tag"), "Tag");
        assert_eq!(title_case("s"), "S");
        assert_eq!(title_case(""), "");
    }

    #[test]
    fn test_tag_source_singular_label_explicit() {
        let source = TagSource {
            field: "taxonomy.performers".to_string(),
            label: Some("Performer".to_string()),
            label_plural: None,
        };
        assert_eq!(source.singular_label(), "Performer");
    }

    #[test]
    fn test_tag_source_singular_label_derived() {
        let source = TagSource {
            field: "tags".to_string(),
            label: None,
            label_plural: None,
        };
        assert_eq!(source.singular_label(), "Tag");
    }

    #[test]
    fn test_tag_source_singular_label_derived_nested() {
        let source = TagSource {
            field: "taxonomy.performers".to_string(),
            label: None,
            label_plural: None,
        };
        assert_eq!(source.singular_label(), "Performer");
    }

    #[test]
    fn test_tag_source_plural_label_explicit() {
        let source = TagSource {
            field: "taxonomy.performers".to_string(),
            label: None,
            label_plural: Some("Performers".to_string()),
        };
        assert_eq!(source.plural_label(), "Performers");
    }

    #[test]
    fn test_tag_source_plural_label_derived() {
        let source = TagSource {
            field: "tags".to_string(),
            label: None,
            label_plural: None,
        };
        assert_eq!(source.plural_label(), "Tags");
    }

    #[test]
    fn test_tag_source_url_source() {
        let source = TagSource {
            field: "Tags".to_string(),
            label: None,
            label_plural: None,
        };
        assert_eq!(source.url_source(), "tags");

        let source = TagSource {
            field: "taxonomy.Performers".to_string(),
            label: None,
            label_plural: None,
        };
        assert_eq!(source.url_source(), "taxonomy.performers");
    }

    #[test]
    fn test_default_tag_sources() {
        let sources = default_tag_sources();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].field, "tags");
        assert_eq!(sources[0].singular_label(), "Tag");
        assert_eq!(sources[0].plural_label(), "Tags");
        assert_eq!(sources[0].url_source(), "tags");
    }

    #[test]
    fn test_config_default_has_tag_sources() {
        let config = Config::default();
        assert_eq!(config.tag_sources.len(), 1);
        assert_eq!(config.tag_sources[0].field, "tags");
        assert!(config.build_tag_pages);
    }

    #[test]
    fn test_config_default_has_relationship_types() {
        let config = Config::default();
        assert!(config.relationship_tracking);
        assert_eq!(config.relationship_types.len(), 4);
        let names: Vec<&str> = config
            .relationship_types
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert!(names.contains(&"parent"));
        assert!(names.contains(&"child"));
        assert!(names.contains(&"spouse"));
        assert!(names.contains(&"sibling"));
    }

    #[test]
    fn test_default_relationship_types_semantics() {
        let types = default_relationship_types();
        let parent = types.iter().find(|t| t.name == "parent").unwrap();
        assert!(!parent.symmetric);
        assert_eq!(parent.inverse.as_deref(), Some("child"));
        assert_eq!(parent.plural_label(), "Parents");

        let child = types.iter().find(|t| t.name == "child").unwrap();
        assert_eq!(child.inverse.as_deref(), Some("parent"));
        assert_eq!(child.plural_label(), "Children");

        let spouse = types.iter().find(|t| t.name == "spouse").unwrap();
        assert!(spouse.symmetric);
        assert!(spouse.inverse.is_none());
    }

    #[test]
    fn test_relation_type_label_derivation() {
        let rel = RelationType {
            name: "cousin".to_string(),
            symmetric: true,
            inverse: None,
            label: None,
            label_plural: None,
        };
        // Auto-derived from name.
        assert_eq!(rel.singular_label(), "Cousin");
        assert_eq!(rel.plural_label(), "Cousins");
    }

    #[test]
    fn test_relation_type_deserialization_minimal() {
        let json = r#"{"name": "friend", "symmetric": true}"#;
        let rel: RelationType = serde_json::from_str(json).unwrap();
        assert_eq!(rel.name, "friend");
        assert!(rel.symmetric);
        assert!(rel.inverse.is_none());
        assert!(rel.label.is_none());
    }

    #[test]
    fn test_tag_source_serialization() {
        let source = TagSource {
            field: "taxonomy.tags".to_string(),
            label: Some("Tag".to_string()),
            label_plural: Some("Tags".to_string()),
        };

        let json = serde_json::to_string(&source).unwrap();
        let parsed: TagSource = serde_json::from_str(&json).unwrap();
        assert_eq!(source, parsed);
    }

    #[test]
    fn test_tag_source_deserialization_minimal() {
        let json = r#"{"field": "tags"}"#;
        let source: TagSource = serde_json::from_str(json).unwrap();
        assert_eq!(source.field, "tags");
        assert!(source.label.is_none());
        assert!(source.label_plural.is_none());
    }

    // ==================== Config Validation Tests ====================

    #[test]
    fn test_validate_default_config_passes() {
        let config = Config::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_port_zero_fails() {
        let config = Config {
            port: 0,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ConfigError::InvalidPort { port: 0 }));
    }

    #[test]
    fn test_validate_valid_ports_pass() {
        // Test minimum valid port
        let config = Config {
            port: 1,
            ..Default::default()
        };
        assert!(config.validate().is_ok());

        // Test common ports
        let config = Config {
            port: 80,
            ..Default::default()
        };
        assert!(config.validate().is_ok());

        let config = Config {
            port: 443,
            ..Default::default()
        };
        assert!(config.validate().is_ok());

        // Test maximum valid port
        let config = Config {
            port: 65535,
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_sidebar_max_items_zero_fails() {
        let config = Config {
            sidebar_max_items: 0,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err,
            ConfigError::InvalidSidebarMaxItems { value: 0 }
        ));
    }

    #[test]
    fn test_validate_valid_sidebar_max_items_pass() {
        let config = Config {
            sidebar_max_items: 1,
            ..Default::default()
        };
        assert!(config.validate().is_ok());

        let config = Config {
            sidebar_max_items: 100,
            ..Default::default()
        };
        assert!(config.validate().is_ok());

        let config = Config {
            sidebar_max_items: 10000,
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_default_gui_menu_bar_is_auto() {
        let config = Config::default();
        assert_eq!(config.gui_menu_bar, MenuBarVisibility::Auto);
    }

    // Typed rather than a `String` so a misspelling is a startup error, not a
    // setting that is silently ignored for the life of the window.
    #[test]
    fn test_gui_menu_bar_parses_each_variant() {
        for (text, expected) in [
            ("auto", MenuBarVisibility::Auto),
            ("always", MenuBarVisibility::Always),
            ("never", MenuBarVisibility::Never),
        ] {
            let parsed: MenuBarVisibility = serde_json::from_str(&format!("\"{text}\"")).unwrap();
            assert_eq!(parsed, expected, "parsing {text}");
        }
    }

    #[test]
    fn test_gui_menu_bar_rejects_unknown_value() {
        let parsed: Result<MenuBarVisibility, _> = serde_json::from_str("\"sometimes\"");
        assert!(parsed.is_err(), "an unknown visibility must not parse");
    }

    // The config-file path, not just the bare enum: a `.mbr/config.toml` that
    // sets this must actually reach `Config`.
    #[test]
    fn test_gui_menu_bar_from_config_file() {
        let config: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string("gui_menu_bar = \"always\""))
            .extract()
            .unwrap();
        assert_eq!(config.gui_menu_bar, MenuBarVisibility::Always);
    }

    #[test]
    fn test_default_graph_depth_is_2() {
        let config = Config::default();
        assert_eq!(config.graph_depth, 2);
    }

    #[test]
    fn test_validate_graph_depth_zero_fails() {
        let config = Config {
            graph_depth: 0,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ConfigError::InvalidGraphDepth { value: 0 }));
    }

    #[test]
    fn test_validate_graph_depth_six_fails() {
        let config = Config {
            graph_depth: 6,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ConfigError::InvalidGraphDepth { value: 6 }));
    }

    #[test]
    fn test_validate_graph_depth_bounds_pass() {
        let config = Config {
            graph_depth: 1,
            ..Default::default()
        };
        assert!(config.validate().is_ok());

        let config = Config {
            graph_depth: 5,
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_build_concurrency_zero_fails() {
        let config = Config {
            build_concurrency: Some(0),
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err,
            ConfigError::InvalidBuildConcurrency { value: 0 }
        ));
    }

    #[test]
    fn test_validate_build_concurrency_none_passes() {
        let config = Config {
            build_concurrency: None, // Auto-detect
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_valid_build_concurrency_pass() {
        let config = Config {
            build_concurrency: Some(1),
            ..Default::default()
        };
        assert!(config.validate().is_ok());

        let config = Config {
            build_concurrency: Some(8),
            ..Default::default()
        };
        assert!(config.validate().is_ok());

        let config = Config {
            build_concurrency: Some(32),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_default_title_prefix_empty() {
        let config = Config::default();
        assert_eq!(config.title_prefix, "");
    }

    #[test]
    fn test_default_title_suffix_empty() {
        let config = Config::default();
        assert_eq!(config.title_suffix, "");
    }

    #[test]
    fn test_validate_editing_non_loopback_without_token_fails() {
        let config = Config {
            edit_enabled: true,
            host: IpArray([0, 0, 0, 0]),
            edit_token_hash: None,
            ..Default::default()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::EditingRequiresToken)
        ));
    }

    #[test]
    fn test_validate_editing_non_loopback_with_token_passes() {
        let config = Config {
            edit_enabled: true,
            host: IpArray([0, 0, 0, 0]),
            edit_token_hash: Some("$argon2id$fake".to_string()),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_editing_loopback_without_token_passes() {
        let config = Config {
            edit_enabled: true,
            host: IpArray([127, 0, 0, 1]),
            edit_token_hash: None,
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_oembed_cache_size_zero_is_valid() {
        // Zero means disabled, which is valid
        let config = Config {
            oembed_cache_size: 0,
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    /// The media metadata cache must have its own budget: it stores JPEG cover
    /// payloads, not the oembed cache's short text metadata.
    #[test]
    fn test_media_cache_size_default_is_independent_of_oembed() {
        let config = Config::default();
        assert_eq!(config.media_cache_size, 64 * 1024 * 1024);
        assert_ne!(
            config.media_cache_size, config.oembed_cache_size,
            "media cache must not inherit the oembed text-metadata budget"
        );
    }

    /// Disabling the oembed cache (`--oembed-cache-size 0`) must not disable
    /// media caching as a side effect.
    #[test]
    fn test_disabling_oembed_cache_keeps_media_cache_enabled() {
        let config = Config {
            oembed_cache_size: 0,
            ..Default::default()
        };
        assert!(config.validate().is_ok());
        assert_eq!(config.media_cache_size, 64 * 1024 * 1024);
    }

    /// A `.mbr/config.toml` written before this option existed must still load
    /// (falling back to the default), and an explicit value must win.
    #[test]
    fn test_media_cache_size_config_file_layering() {
        let without: Config = Figment::new()
            .merge(Serialized::defaults(Config::default()))
            .merge(Toml::string("port = 5201"))
            .extract()
            .expect("config without the key parses");
        assert_eq!(without.media_cache_size, 64 * 1024 * 1024);

        let with: Config = Figment::new()
            .merge(Serialized::defaults(Config::default()))
            .merge(Toml::string("media_cache_size = 1048576"))
            .extract()
            .expect("explicit value parses");
        assert_eq!(with.media_cache_size, 1024 * 1024);
    }

    // ==================== find_root_dir Edge Case Tests ====================

    #[test]
    fn test_find_root_dir_file_without_markers_returns_parent() {
        // Create a temp dir with no markers and a file inside it
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("test.md");
        std::fs::write(&file_path, "# Hello").unwrap();

        let root = find_root_dir(&file_path);

        // Should return the parent directory, not the file itself
        assert!(
            root.is_dir(),
            "root_dir should be a directory, got: {root:?}"
        );
        assert_eq!(root, tmp.path());
    }

    #[test]
    fn test_find_root_dir_directory_without_markers_returns_itself() {
        let tmp = tempfile::tempdir().unwrap();

        let root = find_root_dir(tmp.path());

        assert!(root.is_dir());
        // Should return the directory itself (or CWD if it's an ancestor)
        // Either way, it should be a directory
        assert!(
            root.is_dir(),
            "root_dir should be a directory, got: {root:?}"
        );
    }

    #[test]
    fn test_find_root_dir_with_git_marker_returns_marker_parent() {
        // When .git exists in a non-home ancestor, find_root_dir should use it
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("sub").join("dir");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();

        let file_path = nested.join("test.md");
        std::fs::write(&file_path, "# Hello").unwrap();

        let root = find_root_dir(&file_path);

        // Should find .git and return its parent (tmp root)
        assert_eq!(root, tmp.path().to_path_buf());
        assert!(root.is_dir());
    }

    #[test]
    fn test_is_home_dir() {
        // Reads `HOME` twice (here and inside `is_home_dir`), so it must not
        // interleave with the tests below that swap `HOME` for a temp dir.
        let _guard = env_lock();
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            assert!(is_home_dir(&home));
            assert!(!is_home_dir(Path::new("/tmp")));
        }
    }

    // ==================== static_folder Validation Tests ====================

    /// Builds `<tmp>/project/{content,static}` and returns the temp dir plus the
    /// markdown root (`content`).
    ///
    /// The nesting is deliberate. `Config::default()` uses the *current
    /// directory* as `root_dir`, which would make every boundary assertion below
    /// depend on where the checkout happens to live. Here the root's parent is
    /// `project` (a peer boundary), and `project`'s own parent is the temp dir,
    /// so neither `$HOME` nor the filesystem root is ever the boundary by
    /// accident.
    fn peer_layout() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project/content");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(tmp.path().join("project/static")).unwrap();
        (tmp, root)
    }

    fn config_with_static(root: &Path, static_folder: &str) -> Config {
        Config {
            root_dir: root.to_path_buf(),
            static_folder: static_folder.to_string(),
            ..Default::default()
        }
    }

    /// Retargeted twice: first from "any `..` is an escape" to a one-level
    /// boundary, now to a two-level one. `repo/content` + `repo/static` and
    /// `<project>/src/routes` + `<project>/static` are both real layouts, so
    /// climbing is legal up to [`MAX_STATIC_FOLDER_ASCENT`]. What this still
    /// covers is a hostile `.mbr/config.toml` reaching past that budget, or
    /// grabbing an ancestor of the root wholesale — `..` and `../..` are both
    /// ancestors of `project/content` and would expose the markdown source
    /// itself as raw static files.
    #[test]
    fn test_validate_static_folder_parent_escape_fails() {
        let (_tmp, root) = peer_layout();
        for value in [
            "..",
            "../..",
            "../../../assets",
            "assets/../../../..",
            "assets/../../..",
        ] {
            let err = match config_with_static(&root, value).validate() {
                Err(err) => err,
                Ok(()) => panic!("escaping static_folder must be rejected: {value:?}"),
            };
            assert!(
                format!("{err:?}").contains("static_folder"),
                "error should name static_folder, got: {err:?}"
            );
        }
    }

    /// A `static_folder` that resolves into `$HOME` is refused even though it is
    /// only one level up, because "one level up from a note directory in your
    /// home folder" is the whole home folder. This is the case that keeps
    /// `~/notes` + `static_folder = "../.ssh"` from being servable.
    #[test]
    fn test_validate_static_folder_into_home_fails() {
        let _guard = env_lock();
        let home = tempfile::tempdir().unwrap();
        // macOS hands out `/var/...` temp dirs that canonicalize to
        // `/private/var/...`; `is_home_dir` bridges the two forms, and this
        // keeps the fixture honest about which form it planted.
        let home_str = home.path().to_string_lossy().into_owned();
        let _env = EnvVars::set(&[("HOME", &home_str), ("USERPROFILE", &home_str)]);

        let root = home.path().join("notes");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(home.path().join(".ssh")).unwrap();

        for value in ["..", "../.ssh"] {
            assert!(
                config_with_static(&root, value).validate().is_err(),
                "static_folder {value:?} resolving into $HOME must be rejected"
            );
        }

        // The guard is about *leaving* the root, not about living under $HOME:
        // a normal in-root overlay still works for a vault in the home folder.
        std::fs::create_dir(root.join("static")).unwrap();
        assert!(
            config_with_static(&root, "static").validate().is_ok(),
            "an in-root static folder must still work under $HOME"
        );

        // One level deeper, the ascent budget alone would reach `$HOME` on the
        // second step. The climb has to stop at `<home>/project`, which still
        // leaves the ordinary peer layout working inside it.
        let nested = home.path().join("project/content");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir(home.path().join("project/static")).unwrap();
        assert!(
            config_with_static(&nested, "../static").validate().is_ok(),
            "a peer overlay one level below $HOME must still be accepted"
        );
        for value in ["../../static", "../../.ssh", "../.."] {
            assert!(
                config_with_static(&nested, value).validate().is_err(),
                "static_folder {value:?} must not climb through $HOME"
            );
        }
    }

    /// A markdown root sitting directly under `/` has no boundary above it that
    /// isn't the filesystem root itself, so nothing outside it is reachable.
    /// One level down, the ascent budget stops one short for the same reason.
    #[test]
    fn test_validate_static_folder_filesystem_root_boundary_fails() {
        let (root, nested) = if cfg!(windows) {
            (r"C:\notes", r"C:\srv\notes")
        } else {
            ("/notes", "/srv/notes")
        };
        assert!(
            config_with_static(Path::new(root), "../etc")
                .validate()
                .is_err(),
            "a root whose parent is the filesystem root must not reach outside itself"
        );
        assert!(
            config_with_static(Path::new(nested), "../../etc")
                .validate()
                .is_err(),
            "the climb must stop below the filesystem root rather than spending its budget"
        );
    }

    /// Retargeted: an absolute `static_folder` is no longer rejected by
    /// `validate`, because the identical value is legitimate from
    /// `MBR_STATIC_FOLDER`. The untrusted-repo half of the threat moved to
    /// load time — see `test_config_read_rejects_absolute_static_folder_from_toml`.
    /// What is asserted here is the Windows nuance that decision depends on:
    /// `/etc` is rooted even though `Path::is_absolute` says otherwise there.
    #[test]
    fn test_validate_static_folder_absolute_is_deferred_to_provenance() {
        assert!(is_rooted(Path::new("/etc")), "`/etc` is rooted everywhere");
        assert!(
            is_rooted(Path::new(r"C:\etc")) || !cfg!(windows),
            "a drive-prefixed path is rooted on Windows"
        );
        assert!(!is_rooted(Path::new("static")));
        assert!(!is_rooted(Path::new("../static")));

        let (_tmp, root) = peer_layout();
        let absolute = if cfg!(windows) { r"C:\etc" } else { "/etc" };
        for value in [absolute, "/etc"] {
            assert!(
                config_with_static(&root, value).validate().is_ok(),
                "validate must defer an absolute static_folder {value:?} to provenance"
            );
        }
    }

    /// Retargeted: a `static -> ../static` symlink is now the same thing as
    /// writing `static_folder = "../static"`, so it is allowed. The threat that
    /// remains — and that this still covers — is a symlink used to reach
    /// somewhere the literal value could never name, past the peer boundary.
    #[cfg(unix)]
    #[test]
    fn test_validate_static_folder_symlink_escape_fails() {
        let unrelated = tempfile::tempdir().unwrap();
        let secrets = unrelated.path().join("secrets");
        std::fs::create_dir(&secrets).unwrap();

        let (_tmp, root) = peer_layout();
        std::os::unix::fs::symlink(&secrets, root.join("static")).unwrap();

        assert!(
            config_with_static(&root, "static").validate().is_err(),
            "a static_folder symlinked past the peer boundary must be rejected"
        );
    }

    /// A symlink that lands on a legitimate peer is indistinguishable from
    /// naming that peer directly, so it must be accepted for the same reason
    /// `../static` is.
    #[cfg(unix)]
    #[test]
    fn test_validate_static_folder_symlink_to_peer_passes() {
        let (tmp, root) = peer_layout();
        std::os::unix::fs::symlink(tmp.path().join("project/static"), root.join("assets")).unwrap();

        assert!(
            config_with_static(&root, "assets").validate().is_ok(),
            "a static_folder symlinked to a peer of the root must be accepted"
        );
    }

    #[test]
    fn test_validate_static_folder_relative_passes() {
        let (_tmp, root) = peer_layout();
        for value in ["", "static", "assets", "public/assets", "./static"] {
            assert!(
                config_with_static(&root, value).validate().is_ok(),
                "relative static_folder {value:?} must be accepted"
            );
        }
    }

    /// The regression this whole change exists for: `repo/content` holding the
    /// markdown (and the `.mbr/` folder) with `repo/static` alongside it, named
    /// as `static_folder = "../static"`.
    #[test]
    fn test_validate_static_folder_peer_passes() {
        let (tmp, root) = peer_layout();
        std::fs::create_dir_all(tmp.path().join("project/static/videos")).unwrap();

        for value in ["../static", "../static/videos", "./../static"] {
            assert!(
                config_with_static(&root, value).validate().is_ok(),
                "peer static_folder {value:?} must be accepted"
            );
        }
    }

    /// Builds `<tmp>/project/{src/routes,static}` and returns the temp dir plus
    /// the markdown root (`src/routes`) — the SvelteKit shape, where the
    /// framework pins the markdown root two levels below the project because a
    /// route's filesystem path *is* its URL.
    ///
    /// Nested inside a `project` directory for the same reason as
    /// [`peer_layout`]: the anchor two levels up must be `project`'s own parent
    /// (the temp dir), never `$HOME` or the filesystem root by accident.
    fn two_deep_layout() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project/src/routes");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(tmp.path().join("project/static/images")).unwrap();
        (tmp, root)
    }

    /// The regression this change exists for: SvelteKit's `src/routes` holding
    /// the markdown (and `.mbr/`) with the project's `static/` two levels up.
    #[test]
    fn test_validate_static_folder_two_levels_up_passes() {
        let (_tmp, root) = two_deep_layout();

        for value in [
            "../../static",
            "../../static/images",
            "./../../static",
            "../../not-created-yet",
        ] {
            assert!(
                config_with_static(&root, value).validate().is_ok(),
                "two-level static_folder {value:?} must be accepted"
            );
        }
    }

    /// The anchor alone is not the whole rule. With two levels of ascent the
    /// root's own parent has become a legal *descendant* of the anchor, so
    /// without the ancestor check `..` would be accepted — and it serves the
    /// entire `src/` tree, including the markdown sources, as raw static files.
    #[test]
    fn test_validate_static_folder_ancestor_of_root_fails() {
        let (_tmp, root) = two_deep_layout();

        for value in ["..", "../..", "../../src", "../../src/routes/.."] {
            let err = match config_with_static(&root, value).validate() {
                Err(err) => err,
                Ok(()) => panic!("an ancestor of the markdown root must be rejected: {value:?}"),
            };
            assert!(
                format!("{err:?}").contains("contains the markdown root"),
                "error should say the value contains the root, got: {err:?}"
            );
        }
    }

    /// Two levels is the budget, and spending it is what the refusal must say —
    /// these values are neither ancestors of the root nor near `$HOME`, so the
    /// budget is the only thing refusing them.
    #[test]
    fn test_validate_static_folder_three_levels_up_fails() {
        let (_tmp, root) = two_deep_layout();

        for value in ["../../../elsewhere", "../../../../elsewhere"] {
            let err = match config_with_static(&root, value).validate() {
                Err(err) => err,
                Ok(()) => {
                    panic!("static_folder past the ascent budget must be rejected: {value:?}")
                }
            };
            assert!(
                format!("{err:?}").contains("levels above the markdown root"),
                "error should name the ascent limit, got: {err:?}"
            );
        }
    }

    /// The anchor is the one thing the whole policy is measured against, so it
    /// must always be the root or one of its ancestors, and never more than
    /// [`MAX_STATIC_FOLDER_ASCENT`] levels up. A bug that let it wander would
    /// widen every other rule at once.
    #[test]
    fn test_static_folder_anchor_never_leaves_the_root_line() {
        let (tmp, peer_root) = peer_layout();
        let (tmp2, deep_root) = two_deep_layout();

        for root in [
            peer_root.as_path(),
            deep_root.as_path(),
            tmp.path(),
            tmp2.path(),
            Path::new("/"),
            Path::new("/notes"),
        ] {
            let (anchor, _) = static_folder_anchor(root);
            assert!(
                root.starts_with(&anchor),
                "anchor {} is not an ancestor of root {}",
                anchor.display(),
                root.display()
            );
            let climbed = root.components().count() - anchor.components().count();
            assert!(
                climbed <= MAX_STATIC_FOLDER_ASCENT,
                "anchor {} climbed {climbed} levels above {}",
                anchor.display(),
                root.display()
            );
        }
    }

    /// `is_home_dir` fires only on `$HOME` itself, which was enough at one
    /// level. At two it is not: the anchor for `/home/you/notes` would be
    /// `/home`, and `../bob` would then name another account. The climb has to
    /// stop below any directory that *contains* `$HOME`.
    #[test]
    fn test_ascent_stops_at_a_directory_containing_home() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        let users = tmp.path().join("Users");
        let home = users.join("alice");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(users.join("bob")).unwrap();

        let home_str = home.to_string_lossy().into_owned();
        let _env = EnvVars::set(&[("HOME", &home_str), ("USERPROFILE", &home_str)]);

        // A root sitting directly under `Users` is the case `is_home_dir` alone
        // misses: its parent is not `$HOME`, it merely *contains* it, and one
        // level of budget is all it takes to get there.
        let beside_home = users.join("notes");
        std::fs::create_dir(&beside_home).unwrap();
        let (anchor, stop) = static_folder_anchor(&beside_home);
        assert_eq!(
            anchor, beside_home,
            "the climb must not step into a directory holding $HOME"
        );
        assert_eq!(stop, AscentStop::HomeDir);
        assert!(
            config_with_static(&beside_home, "../alice")
                .validate()
                .is_err(),
            "$HOME must not be reachable from a sibling of it"
        );

        // And the case it does catch, one level lower, still stops for the
        // same reason rather than spending the second level of budget.
        let root = home.join("notes");
        std::fs::create_dir(&root).unwrap();
        assert_eq!(static_folder_anchor(&root).1, AscentStop::HomeDir);
        assert!(
            config_with_static(&root, "../../bob").validate().is_err(),
            "another account under the same Users directory must not be reachable"
        );
    }

    /// A relative `root_dir` bottoms out at an empty parent rather than at a
    /// filesystem root, and `contains_home_dir` must not claim that empty path
    /// as `$HOME` — `starts_with("")` is true of every path. Pinned because the
    /// distinction is invisible until someone constructs a `Config` by hand.
    #[test]
    fn test_validate_static_folder_relative_root_dir() {
        let root = PathBuf::from("no-such-root-8f2c/content");
        for value in ["../../x", "../../../x"] {
            assert!(
                config_with_static(&root, value).validate().is_err(),
                "a relative root must not reach outside itself: {value:?}"
            );
        }
    }

    /// A UNC share root behaves exactly like `C:\`: it has no parent, so the
    /// climb stops there rather than reaching another share.
    #[cfg(windows)]
    #[test]
    fn test_validate_static_folder_unc_share_boundary() {
        let root = Path::new(r"\\server\share\proj\content");
        assert!(
            config_with_static(root, r"..\..\x").validate().is_err(),
            "the climb must stop at a UNC share root"
        );
    }

    /// Four different walls produce four different refusals. The messages are
    /// the only thing an operator sees when mbr refuses to start, and "outside
    /// the boundary" for all four would not tell them which knob to turn.
    #[test]
    fn test_static_folder_refusal_messages_name_the_wall() {
        let anchor = Path::new("/project");

        assert!(
            AscentStop::RootHasNoParent
                .refusal(anchor)
                .contains("no parent directory")
        );
        assert!(
            AscentStop::HomeDir
                .refusal(anchor)
                .contains("the home directory")
        );
        assert!(
            AscentStop::FilesystemRoot
                .refusal(anchor)
                .contains("the filesystem root")
        );

        let budget = AscentStop::Budget.refusal(anchor);
        assert!(budget.contains("levels above the markdown root"));
        // Each message names the anchor, which is what tells the operator where
        // an overlay *would* have been accepted.
        for stop in [
            AscentStop::HomeDir,
            AscentStop::FilesystemRoot,
            AscentStop::Budget,
        ] {
            assert!(
                stop.refusal(anchor).contains("/project"),
                "refusal should name the anchor: {}",
                stop.refusal(anchor)
            );
        }
    }

    /// A directory that does not exist yet must be judged the same way an
    /// existing one is: `canonicalize` fails, so the lexical fallback has to
    /// produce the same verdict rather than silently passing everything.
    #[test]
    fn test_validate_static_folder_nonexistent_paths_use_lexical_fallback() {
        let (_tmp, root) = peer_layout();
        for value in ["../not-created-yet", "../../not-created-yet"] {
            assert!(
                config_with_static(&root, value).validate().is_ok(),
                "a not-yet-created directory within the ascent budget is still allowed: {value:?}"
            );
        }
        assert!(
            config_with_static(&root, "../../../not-created-yet")
                .validate()
                .is_err(),
            "a nonexistent path past the ascent budget must still be rejected"
        );
    }

    // ==================== find_root_dir $HOME Skip Tests ====================

    /// Serializes tests that mutate process-global environment variables.
    /// Rust runs tests in parallel threads within one process, so `HOME` /
    /// `MBR_*` mutation would otherwise race.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        // A panicking test poisons the mutex; the guarded state is only the
        // environment, which each test sets up for itself.
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Sets environment variables for the duration of a scope and restores
    /// their previous values on drop (including on panic).
    struct EnvVars {
        saved: Vec<(String, Option<std::ffi::OsString>)>,
    }

    impl EnvVars {
        fn set(pairs: &[(&str, &str)]) -> Self {
            let saved = pairs
                .iter()
                .map(|(key, value)| {
                    let previous = std::env::var_os(key);
                    // SAFETY: all environment mutation in these tests happens
                    // under `ENV_LOCK`, and no other thread in the test binary
                    // reads the environment concurrently.
                    unsafe { std::env::set_var(key, value) };
                    ((*key).to_string(), previous)
                })
                .collect();
            Self { saved }
        }
    }

    impl Drop for EnvVars {
        fn drop(&mut self) {
            for (key, previous) in &self.saved {
                // SAFETY: as above - still holding `ENV_LOCK`.
                unsafe {
                    match previous {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
    }

    /// Regression: a `$HOME` marker match used to `break` out of the whole
    /// marker loop, so a dotfiles `~/.git` hid the `.obsidian` that marks the
    /// real vault and the root collapsed to the leaf directory.
    #[test]
    fn test_find_root_dir_home_marker_does_not_abort_search() {
        let _guard = env_lock();
        let home = tempfile::tempdir().unwrap();
        let home_str = home.path().to_string_lossy().into_owned();
        let _env = EnvVars::set(&[("HOME", &home_str), ("USERPROFILE", &home_str)]);

        // ~/.git (dotfiles repo) plus a real vault at ~/notes.
        std::fs::create_dir(home.path().join(".git")).unwrap();
        let vault = home.path().join("notes");
        std::fs::create_dir(&vault).unwrap();
        std::fs::create_dir(vault.join(".obsidian")).unwrap();
        let leaf = vault.join("projects");
        std::fs::create_dir(&leaf).unwrap();
        let note = leaf.join("plan.md");
        std::fs::write(&note, "# Plan").unwrap();

        assert_eq!(
            find_root_dir(&note),
            vault,
            "the `.obsidian` vault must win once the `$HOME` `.git` is skipped"
        );
    }

    /// The `$HOME` guard itself still holds: when every marker only matches at
    /// `$HOME`, the root falls back to the start directory.
    #[test]
    fn test_find_root_dir_only_home_marker_falls_back() {
        let _guard = env_lock();
        let home = tempfile::tempdir().unwrap();
        let home_str = home.path().to_string_lossy().into_owned();
        let _env = EnvVars::set(&[("HOME", &home_str), ("USERPROFILE", &home_str)]);

        std::fs::create_dir(home.path().join(".git")).unwrap();
        let leaf = home.path().join("notes/projects");
        std::fs::create_dir_all(&leaf).unwrap();
        let note = leaf.join("plan.md");
        std::fs::write(&note, "# Plan").unwrap();

        assert_eq!(find_root_dir(&note), leaf);
    }

    // ==================== Config::read Layering Tests ====================

    /// Creates a repo root pinned by a `.mbr/` directory so `find_root_dir`
    /// cannot wander into whatever markers exist above the temp directory.
    fn repo_with_mbr_dir(config_toml: Option<&str>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let mbr = dir.path().join(".mbr");
        std::fs::create_dir(&mbr).unwrap();
        if let Some(contents) = config_toml {
            std::fs::write(mbr.join("config.toml"), contents).unwrap();
        }
        dir
    }

    #[test]
    fn test_config_read_defaults_only() {
        let _guard = env_lock();
        let repo = repo_with_mbr_dir(None);

        let config = Config::read(repo.path()).expect("defaults must load");

        assert_eq!(config.theme, Config::default().theme);
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.root_dir, repo.path());
    }

    #[test]
    fn test_config_read_toml_overrides_defaults() {
        let _guard = env_lock();
        let repo = repo_with_mbr_dir(Some("theme = \"amber\"\nport = 5321\n"));

        let config = Config::read(repo.path()).expect("toml must load");

        assert_eq!(config.theme, "amber");
        assert_eq!(config.port, 5321);
    }

    #[test]
    fn test_config_read_env_overrides_defaults() {
        let _guard = env_lock();
        let repo = repo_with_mbr_dir(None);
        let _env = EnvVars::set(&[("MBR_THEME", "cyan"), ("MBR_PORT", "5322")]);

        let config = Config::read(repo.path()).expect("env must load");

        assert_eq!(config.theme, "cyan");
        assert_eq!(config.port, 5322);
    }

    /// The documented precedence: `MBR_*` beats `.mbr/config.toml` beats
    /// defaults. This matters for security, not just ergonomics - the toml
    /// layer ships inside the (possibly untrusted) markdown repository, so an
    /// operator must be able to override it from outside.
    #[test]
    fn test_config_read_env_beats_toml_beats_defaults() {
        let _guard = env_lock();
        let repo = repo_with_mbr_dir(Some(
            "theme = \"amber\"\nport = 5321\nindex_file = \"home.md\"\n",
        ));
        let _env = EnvVars::set(&[("MBR_THEME", "cyan")]);

        let config = Config::read(repo.path()).expect("all three layers must load");

        assert_eq!(config.theme, "cyan", "env must win over the config file");
        assert_eq!(
            config.index_file, "home.md",
            "config file must win over defaults where env is silent"
        );
        assert_eq!(
            config.port, 5321,
            "config file must win over defaults where env is silent"
        );
        assert_eq!(
            config.markdown_extensions,
            Config::default().markdown_extensions,
            "untouched keys keep the compiled-in default"
        );
    }

    // ==================== tasks_ignore_globs Tests ====================

    /// Opt-in: an unconfigured repository indexes every file's tasks.
    #[test]
    fn test_tasks_ignore_globs_defaults_to_empty() {
        assert!(Config::default().tasks_ignore_globs.is_empty());
    }

    #[test]
    fn test_config_read_tasks_ignore_globs_from_toml() {
        let _guard = env_lock();
        let repo = repo_with_mbr_dir(Some(
            "tasks_ignore_globs = [\"templates/**\", \"**/archive/**\"]\n",
        ));

        let config = Config::read(repo.path()).expect("toml must load");

        assert_eq!(config.tasks_ignore_globs, ["templates/**", "**/archive/**"]);
    }

    /// The env layer takes a JSON array, and wins over the repository's own
    /// config file like every other key.
    #[test]
    fn test_config_read_tasks_ignore_globs_env_overrides_toml() {
        let _guard = env_lock();
        let repo = repo_with_mbr_dir(Some("tasks_ignore_globs = [\"templates/**\"]\n"));
        let _env = EnvVars::set(&[("MBR_TASKS_IGNORE_GLOBS", r#"["**/archive/**"]"#)]);

        let config = Config::read(repo.path()).expect("env must load");

        assert_eq!(config.tasks_ignore_globs, ["**/archive/**"]);
    }

    /// A `.mbr/config.toml` written before this option existed must still load.
    #[test]
    fn test_config_read_without_tasks_ignore_globs_key_still_loads() {
        let _guard = env_lock();
        let repo = repo_with_mbr_dir(Some("theme = \"amber\"\nport = 5323\n"));

        let config = Config::read(repo.path()).expect("a config predating the option must load");

        assert!(config.tasks_ignore_globs.is_empty());
        assert_eq!(config.theme, "amber");
    }

    /// Fail fast: the patterns are compiled once at `TaskIndex` construction,
    /// where a bad one would be dropped and silently index the folder it was
    /// meant to exclude.
    #[test]
    fn test_validate_rejects_a_malformed_tasks_ignore_glob() {
        // `**` must be a whole path component; `**bad` is not.
        let config = Config {
            tasks_ignore_globs: vec!["templates/**".to_string(), "templates/**bad".to_string()],
            ..Default::default()
        };

        let err = config
            .validate()
            .expect_err("a malformed glob must fail validation");

        assert!(
            matches!(err, ConfigError::InvalidTasksIgnoreGlob { ref pattern, .. } if pattern == "templates/**bad"),
            "expected InvalidTasksIgnoreGlob naming the offending pattern, got: {err:?}"
        );
        assert!(err.to_string().contains("templates/**"));
    }

    #[test]
    fn test_validate_accepts_the_documented_glob_spellings() {
        let config = Config {
            tasks_ignore_globs: vec![
                "templates/**".to_string(),
                "**/templates/**".to_string(),
                "docs/*.md".to_string(),
            ],
            ..Default::default()
        };

        assert!(config.validate().is_ok());
    }

    /// The validation runs during load, not only when called directly.
    #[test]
    fn test_config_read_rejects_a_malformed_tasks_ignore_glob() {
        let _guard = env_lock();
        let repo = repo_with_mbr_dir(Some("tasks_ignore_globs = [\"templates/**bad\"]\n"));

        let err = Config::read(repo.path()).expect_err("a malformed glob must abort loading");

        assert!(
            matches!(
                err,
                crate::MbrError::Config(ConfigError::InvalidTasksIgnoreGlob { .. })
            ),
            "expected ConfigError::InvalidTasksIgnoreGlob, got: {err:?}"
        );
    }

    /// Checkboxes only, deliberately narrower than the wire default of
    /// [`IncludeFilter::All`]: the panel widens by itself when this returns
    /// nothing, so the narrow default costs a repo without checkboxes nothing.
    #[test]
    fn test_default_tasks_default_include_is_tasks() {
        assert_eq!(
            Config::default().tasks_default_include,
            IncludeFilter::Tasks
        );
    }

    #[test]
    fn test_config_read_tasks_default_include_round_trips_every_value() {
        let _guard = env_lock();
        for (written, expected) in [
            ("all", IncludeFilter::All),
            ("tasks", IncludeFilter::Tasks),
            ("markers", IncludeFilter::Markers),
        ] {
            let repo = repo_with_mbr_dir(Some(&format!("tasks_default_include = \"{written}\"\n")));

            let config = Config::read(repo.path())
                .unwrap_or_else(|e| panic!("{written:?} must load, got: {e:?}"));

            assert_eq!(config.tasks_default_include, expected, "for {written:?}");
        }
    }

    /// Typed as the real enum rather than a `String` precisely so this fails at
    /// startup, where it is visible, instead of silently selecting a default.
    #[test]
    fn test_config_read_rejects_an_unknown_tasks_default_include() {
        let _guard = env_lock();
        let repo = repo_with_mbr_dir(Some("tasks_default_include = \"todos\"\n"));

        let err = Config::read(repo.path()).expect_err("an unknown value must abort loading");

        assert!(
            matches!(err, crate::MbrError::Config(ConfigError::ParseFailed(_))),
            "expected ConfigError::ParseFailed, got: {err:?}"
        );
    }

    /// Env beats the repository's own file, for the reason the whole `MBR_*`
    /// layer exists: the operator serving a repo need not be its author.
    #[test]
    fn test_config_read_tasks_default_include_env_overrides_toml() {
        let _guard = env_lock();
        let repo = repo_with_mbr_dir(Some("tasks_default_include = \"tasks\"\n"));
        let _env = EnvVars::set(&[("MBR_TASKS_DEFAULT_INCLUDE", "markers")]);

        let config = Config::read(repo.path()).expect("env must load");

        assert_eq!(config.tasks_default_include, IncludeFilter::Markers);
    }

    /// A `.mbr/config.toml` written before this option existed must still load.
    #[test]
    fn test_config_read_without_tasks_default_include_key_still_loads() {
        let _guard = env_lock();
        let repo = repo_with_mbr_dir(Some("theme = \"amber\"\nport = 5324\n"));

        let config = Config::read(repo.path()).expect("a config predating the option must load");

        assert_eq!(config.tasks_default_include, IncludeFilter::Tasks);
        assert_eq!(config.theme, "amber");
    }

    #[test]
    fn test_config_read_malformed_toml_returns_parse_failed() {
        let _guard = env_lock();
        let repo = repo_with_mbr_dir(Some("theme = \nport = [not valid toml\n"));

        let err = Config::read(repo.path()).expect_err("malformed toml must fail");

        assert!(
            matches!(err, crate::MbrError::Config(ConfigError::ParseFailed(_))),
            "expected ConfigError::ParseFailed, got: {err:?}"
        );
    }

    #[test]
    fn test_config_read_rejects_escaping_static_folder_from_toml() {
        let _guard = env_lock();
        let repo = repo_with_mbr_dir(Some("static_folder = \"../..\"\n"));

        assert!(
            Config::read(repo.path()).is_err(),
            "a repo-supplied static_folder that escapes the root must be rejected"
        );
    }

    /// The untrusted-repo half of the absolute-path rule: whoever cloned the
    /// repo did not choose this value, so an absolute `static_folder` in the
    /// repo's own `.mbr/config.toml` is refused no matter where it points.
    #[test]
    fn test_config_read_rejects_absolute_static_folder_from_toml() {
        let _guard = env_lock();
        let absolute = if cfg!(windows) { r"C:\\etc" } else { "/etc" };
        let repo = repo_with_mbr_dir(Some(&format!("static_folder = \"{absolute}\"\n")));

        let err = Config::read(repo.path())
            .expect_err("an absolute static_folder from the repo config must be rejected");

        assert!(
            format!("{err:?}").contains("static_folder"),
            "error should name static_folder, got: {err:?}"
        );
    }

    /// The operator half of the same rule: `MBR_STATIC_FOLDER` is set by whoever
    /// runs the server, not by the repository, so an absolute value there is a
    /// deliberate choice and is honored.
    #[test]
    fn test_config_read_accepts_absolute_static_folder_from_env() {
        let _guard = env_lock();
        let repo = repo_with_mbr_dir(None);
        let assets = tempfile::tempdir().unwrap();
        let assets_str = assets.path().to_string_lossy().into_owned();
        let _env = EnvVars::set(&[("MBR_STATIC_FOLDER", &assets_str)]);

        let config = Config::read(repo.path())
            .expect("an absolute static_folder from the environment must be accepted");

        assert_eq!(config.static_folder, assets_str);
    }

    /// Env wins over the config file, and provenance follows the winner: the
    /// operator's absolute value is honored even when the repo also names one.
    #[test]
    fn test_config_read_env_absolute_static_folder_overrides_toml() {
        let _guard = env_lock();
        let repo = repo_with_mbr_dir(Some("static_folder = \"/etc\"\n"));
        let assets = tempfile::tempdir().unwrap();
        let assets_str = assets.path().to_string_lossy().into_owned();
        let _env = EnvVars::set(&[("MBR_STATIC_FOLDER", &assets_str)]);

        let config = Config::read(repo.path())
            .expect("the operator's absolute static_folder must win over the repo's");

        assert_eq!(config.static_folder, assets_str);
    }

    /// The user-reported regression, exercised through the real loader: a
    /// `content/` markdown root holding `.mbr/config.toml` with a `static/`
    /// directory alongside it must load.
    #[test]
    fn test_config_read_accepts_peer_static_folder_from_toml() {
        let _guard = env_lock();
        let project = tempfile::tempdir().unwrap();
        let content = project.path().join("content");
        std::fs::create_dir_all(content.join(".mbr")).unwrap();
        std::fs::write(
            content.join(".mbr/config.toml"),
            "static_folder = \"../static\"\n",
        )
        .unwrap();
        std::fs::create_dir(project.path().join("static")).unwrap();

        let config = Config::read(&content).expect("a peer static folder must load");

        assert_eq!(config.static_folder, "../static");
        assert_eq!(config.root_dir, content);
    }
}
