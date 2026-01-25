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

fn default_build_tag_pages() -> bool {
    true
}

fn default_sidebar_style() -> String {
    "panel".to_string()
}

fn default_sidebar_max_items() -> usize {
    100
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
    /// Lowercased for URL consistency.
    pub fn url_source(&self) -> String {
        self.field.to_lowercase()
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
            port: 5200,
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
            oembed_timeout_ms: 500,
            oembed_cache_size: 2 * 1024 * 1024, // 2MB default
            template_folder: None,
            sort: default_sort_config(),
            build_concurrency: None, // Auto-detect based on CPU cores
            transcode: false,        // Disabled by default
            skip_link_checks: false, // Link checking enabled by default
            link_tracking: true,     // Bidirectional link tracking enabled by default
            tag_sources: default_tag_sources(),
            build_tag_pages: true, // Tag pages enabled by default
            sidebar_style: default_sidebar_style(),
            sidebar_max_items: default_sidebar_max_items(),
        }
    }
}

impl Config {
    pub fn read(search_config_from: &PathBuf) -> Result<Self, crate::MbrError> {
        let default_config = Config::default();
        let root_dir = Self::find_root_dir(search_config_from);
        let mut config: Config = Figment::new()
            .merge(Serialized::defaults(default_config))
            .merge(Env::prefixed("MBR_"))
            .merge(Toml::file(root_dir.join(".mbr/config.toml")))
            .extract()
            .map_err(|e| ConfigError::ParseFailed(Box::new(e)))?;
        tracing::debug!("Loaded config: {:?}", &config);
        config.root_dir = root_dir;
        config.validate()?;
        Ok(config)
    }

    /// Validates the configuration values.
    ///
    /// Checks that numeric configuration options are within valid bounds:
    /// - `port`: Must be 1-65535 (port 0 means "auto-assign", which isn't useful for display)
    /// - `sidebar_max_items`: Must be > 0
    /// - `build_concurrency`: If set, must be > 0
    ///
    /// Note: `oembed_cache_size` of 0 is valid (disables caching).
    pub fn validate(&self) -> Result<(), ConfigError> {
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

        // build_concurrency of 0 would mean no parallelism (None means auto-detect)
        if matches!(self.build_concurrency, Some(0)) {
            return Err(ConfigError::InvalidBuildConcurrency { value: 0 });
        }

        Ok(())
    }

    fn find_root_dir(start_dir: &PathBuf) -> PathBuf {
        // Search for common repository markers in priority order
        // Directories
        const DIR_MARKERS: &[&str] = &[".mbr", ".git", ".zk", ".obsidian"];
        // Config files for documentation tools
        const FILE_MARKERS: &[&str] = &["book.toml", "mkdocs.yml", "docusaurus.config.js"];

        for marker in DIR_MARKERS {
            if let Some(root) = Self::search_folder_in_ancestors(start_dir, marker) {
                return root;
            }
        }

        for marker in FILE_MARKERS {
            if let Some(root) = Self::search_file_in_ancestors(start_dir, marker) {
                return root;
            }
        }

        Self::cwd_if_ancestor(start_dir).unwrap_or_else(|| start_dir.clone())
    }

    fn cwd_if_ancestor(start_path: &PathBuf) -> Option<PathBuf> {
        let cwd = std::env::current_dir().ok()?;
        let dir = if start_path.is_dir() {
            start_path
        } else {
            start_path.parent()?
        };
        dir.ancestors()
            .find(|candidate| *candidate == cwd)
            .map(|x| x.to_path_buf())
    }

    fn search_folder_in_ancestors<P: AsRef<Path>>(
        start_path: &PathBuf,
        search_folder: P, // the folder I'm looking for (usually .mbr)
    ) -> Option<PathBuf> {
        let search_folder = search_folder.as_ref();
        let dir = if start_path.is_dir() {
            start_path
        } else {
            start_path.parent()?
        };
        // ancestors() yields `dir`, then its parent, then its parent, … until root.
        dir.ancestors()
            .map(|ancestor| ancestor.join(search_folder))
            .find(|candidate| candidate.as_path().is_dir())
            .and_then(|mbr_dir| mbr_dir.parent().map(|p| p.to_path_buf()))
    }

    fn search_file_in_ancestors<P: AsRef<Path>>(
        start_path: &PathBuf,
        search_file: P, // the file I'm looking for (e.g., book.toml)
    ) -> Option<PathBuf> {
        let search_file = search_file.as_ref();
        let dir = if start_path.is_dir() {
            start_path
        } else {
            start_path.parent()?
        };
        // ancestors() yields `dir`, then its parent, then its parent, … until root.
        dir.ancestors()
            .map(|ancestor| ancestor.join(search_file))
            .find(|candidate| candidate.as_path().is_file())
            .and_then(|file_path| file_path.parent().map(|p| p.to_path_buf()))
    }
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
    fn test_validate_oembed_cache_size_zero_is_valid() {
        // Zero means disabled, which is valid
        let config = Config {
            oembed_cache_size: 0,
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }
}
