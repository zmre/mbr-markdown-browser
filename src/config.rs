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

/// Default markers that flag a block as incomplete.
///
/// A block whose first text matches `^(MARKER)\b` (uppercase, word boundary)
/// gets wrapped in `<span class="mbr-incomplete">…</span>`.
pub fn default_incomplete_markers() -> Vec<String> {
    vec![
        "TK".to_string(),
        "TODO".to_string(),
        "FIXME".to_string(),
        "XXX".to_string(),
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
    /// Text to prepend to all page titles (e.g., "My Site: ").
    /// Default: empty string (no prefix).
    #[serde(default)]
    pub title_prefix: String,
    /// Text to append to all page titles (e.g., " | My Site").
    /// Default: empty string (no suffix).
    #[serde(default)]
    pub title_suffix: String,
    /// Markers that flag a block as incomplete. A paragraph, heading, list
    /// item, or table cell whose first text matches `^(MARKER)\b` gets
    /// wrapped in `<span class="mbr-incomplete">…</span>`.
    /// Default: `["TK", "TODO", "FIXME", "XXX"]`.
    #[serde(default = "default_incomplete_markers")]
    pub incomplete_markers: Vec<String>,
    /// Enable incomplete-block marking. `None` = mode default (on for
    /// server/GUI, off for static build). CLI flags
    /// `--mark-incomplete` / `--no-mark-incomplete` force a value.
    #[serde(default)]
    pub mark_incomplete: Option<bool>,
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
            title_prefix: String::new(),
            title_suffix: String::new(),
            incomplete_markers: default_incomplete_markers(),
            mark_incomplete: None,
            edit_enabled: false,
            edit_token_hash: None,
            edit_require_token_on_loopback: false,
            upload_max_bytes: DEFAULT_UPLOAD_MAX_BYTES,
        }
    }
}

/// Returns true if the given path is the user's home directory.
///
/// Compares canonical forms when both sides resolve, because callers do not
/// agree on path shape: [`find_root_dir`] walks raw ancestors, while
/// [`Config::validate_static_folder`] compares an already-canonicalized root.
/// Without this, a `$HOME` reached through a symlink (`/home/me` ->
/// `/mnt/users/me`) would slip past both guards.
fn is_home_dir(path: &Path) -> bool {
    // Windows does not set `HOME`; the equivalent is `USERPROFILE`. Without
    // this the guard below never fires there, so a stray `C:\Users\you\.git`
    // or `.obsidian` would silently make the entire home directory the repo
    // root and trigger a scan of everything the user owns.
    let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let Some(home) = std::env::var_os(home_var).map(PathBuf::from) else {
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

    /// Emits one INFO line when the static overlay lands outside the markdown
    /// root, so an external static root is never silently in effect.
    fn log_external_static_folder(&self) {
        if let Ok(StaticOverlay::External(dir)) =
            resolve_static_overlay(&self.root_dir, &self.static_folder)
        {
            tracing::info!(
                "static_folder {:?} resolves outside the markdown root ({}): serving and indexing assets from {}",
                self.static_folder,
                self.root_dir.display(),
                dir.display()
            );
        }
    }

    /// Validates the configuration values.
    ///
    /// Checks that numeric configuration options are within valid bounds:
    /// - `port`: Must be 1-65535 (port 0 means "auto-assign", which isn't useful for display)
    /// - `sidebar_max_items`: Must be > 0
    /// - `graph_depth`: Must be between 1 and 5
    /// - `build_concurrency`: If set, must be > 0
    /// - `static_folder`: Must stay inside `root_dir` or be a peer of it (see
    ///   [`Config::validate_static_folder`])
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

    /// Rejects a `static_folder` that reaches further than a *peer* of the
    /// markdown root. See [`resolve_static_overlay`] for the policy itself.
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
/// - **Outside the root**: accepted only as a strict descendant of the root's
///   *parent*, which is what makes the common `repo/content` + `repo/static`
///   layout work. The parent itself is refused — that would expose every sibling
///   of the root, which is not a peer relationship — and so is any parent that is
///   `$HOME` or the filesystem root, which is what keeps `~/notes` +
///   `static_folder = "../.ssh"` from resolving into the home directory.
///
/// The resolved directory is canonicalized when it exists, so a `static -> /etc`
/// symlink is judged by where it actually lands rather than by how innocent it
/// looks lexically. When it does not exist yet the resolution falls back to
/// lexical normalization, so an escape whose target has not been created is
/// caught too.
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

    let Some(parent) = root.parent() else {
        return Err(invalid_static_folder(
            static_folder,
            "resolves outside the markdown root, which has no parent directory",
        ));
    };
    // Reuses `find_root_dir`'s notion of "home" rather than inventing a second
    // one: `~/notes` with `../.ssh` must not become servable.
    if is_home_dir(parent) {
        return Err(invalid_static_folder(
            static_folder,
            "resolves outside the markdown root and into the home directory",
        ));
    }
    if parent.parent().is_none() {
        return Err(invalid_static_folder(
            static_folder,
            "resolves outside the markdown root and into the filesystem root",
        ));
    }
    if dir == parent {
        return Err(invalid_static_folder(
            static_folder,
            "resolves to the parent of the markdown root, which would expose every \
             sibling directory; name a specific sibling instead",
        ));
    }
    if !dir.starts_with(parent) {
        return Err(invalid_static_folder(
            static_folder,
            "reaches past the parent of the markdown root; only a peer of the root is allowed",
        ));
    }

    Ok(StaticOverlay::External(dir))
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

    /// Retargeted from "any `..` is an escape" to the boundary that actually
    /// matters. `repo/content` + `repo/static` is a real layout, so climbing one
    /// level is now legal; the threat this still covers is a hostile
    /// `.mbr/config.toml` reaching *past* that one level — or grabbing the
    /// parent wholesale, which would expose every sibling of the root.
    #[test]
    fn test_validate_static_folder_parent_escape_fails() {
        let (_tmp, root) = peer_layout();
        for value in ["..", "../..", "../../assets", "assets/../../.."] {
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
    }

    /// A markdown root sitting directly under `/` has no peer boundary that
    /// isn't the filesystem root itself, so nothing outside it is reachable.
    #[test]
    fn test_validate_static_folder_filesystem_root_boundary_fails() {
        let root = if cfg!(windows) { r"C:\notes" } else { "/notes" };
        assert!(
            config_with_static(Path::new(root), "../etc")
                .validate()
                .is_err(),
            "a root whose parent is the filesystem root must not reach outside itself"
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

    /// A peer that does not exist yet must be judged the same way an existing
    /// one is: `canonicalize` fails, so the lexical fallback has to produce the
    /// same verdict rather than silently passing everything.
    #[test]
    fn test_validate_static_folder_nonexistent_paths_use_lexical_fallback() {
        let (_tmp, root) = peer_layout();
        assert!(
            config_with_static(&root, "../not-created-yet")
                .validate()
                .is_ok(),
            "a peer that does not exist yet is still a peer"
        );
        assert!(
            config_with_static(&root, "../../not-created-yet")
                .validate()
                .is_err(),
            "a nonexistent path past the boundary must still be rejected"
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
