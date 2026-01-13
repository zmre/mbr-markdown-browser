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
    pub ip: IpArray,
    pub port: u16,
    pub static_folder: String,
    pub enable_writes: bool,
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
    /// Optional template folder that overrides the default .mbr/ and compiled defaults.
    /// Files found here take precedence; missing files fall back to compiled defaults.
    #[serde(skip)]
    pub template_folder: Option<PathBuf>,
    /// Sort configuration for file listings. Supports multi-level sorting by any field.
    /// Default: sort by title (falling back to filename), ascending, string comparison.
    #[serde(default = "default_sort_config")]
    pub sort: Vec<SortField>,
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
            ip: IpArray([127, 0, 0, 1]),
            port: 5200,
            static_folder: "static".to_string(),
            enable_writes: false,
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
            oembed_timeout_ms: 300,
            template_folder: None,
            sort: default_sort_config(),
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
        Ok(config)
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
