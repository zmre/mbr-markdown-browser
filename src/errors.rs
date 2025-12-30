//! Centralized error types for the mbr markdown browser.
//!
//! This module provides typed errors using thiserror to replace
//! `Box<dyn std::error::Error>` throughout the codebase.

use std::path::PathBuf;
use thiserror::Error;

/// Top-level error type for the mbr application.
#[derive(Debug, Error)]
pub enum MbrError {
    #[error("Server error: {0}")]
    Server(#[from] ServerError),

    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("Configuration parsing error: {0}")]
    ConfigParse(#[from] figment::Error),

    #[error("Markdown error: {0}")]
    Markdown(#[from] MarkdownError),

    #[error("Template error: {0}")]
    Template(#[from] TemplateError),

    #[error("Repository error: {0}")]
    Repo(#[from] RepoError),

    #[error("Browser error: {0}")]
    Browser(#[from] BrowserError),

    #[error("Watcher error: {0}")]
    Watcher(#[from] WatcherError),

    #[error("Build error: {0}")]
    Build(#[from] BuildError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),
}

/// Errors related to the HTTP server.
#[derive(Debug, Error)]
pub enum ServerError {
    #[error("Failed to bind to {addr}")]
    BindFailed {
        addr: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Server failed to start")]
    StartFailed(#[source] std::io::Error),

    #[error("Failed to get local address")]
    LocalAddrFailed(#[source] std::io::Error),

    #[error("Template initialization failed: {0}")]
    TemplateInit(#[from] TemplateError),

    #[error("Tracing initialization failed")]
    TracingInit,
}

/// Errors related to configuration loading and parsing.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to get current directory")]
    CurrentDirFailed(#[source] std::io::Error),

    #[error("Failed to find root directory from path: {path}")]
    RootDirNotFound { path: PathBuf },

    #[error("Failed to get parent directory of: {path}")]
    NoParentDir { path: PathBuf },

    #[error("Configuration parsing failed")]
    ParseFailed(#[from] figment::Error),

    #[error("Failed to canonicalize path: {path}")]
    CanonicalizeFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to calculate relative path from {from} to {to}")]
    RelativePathFailed { from: PathBuf, to: PathBuf },

    #[error("Template folder is not a directory: {}", path.display())]
    TemplateFolderNotDirectory { path: PathBuf },
}

/// Errors related to markdown parsing and rendering.
#[derive(Debug, Error)]
pub enum MarkdownError {
    #[error("Failed to read markdown file: {path}")]
    ReadFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to parse YAML frontmatter")]
    YamlParseFailed,

    #[error("Failed to fetch oembed data for URL: {url}")]
    OembedFetchFailed { url: String },

    #[error("HTTP request failed: {0}")]
    HttpFailed(#[from] reqwest::Error),
}

/// Errors related to template rendering.
#[derive(Debug, Error)]
pub enum TemplateError {
    #[error("Failed to initialize templates from: {path}")]
    InitFailed {
        path: PathBuf,
        #[source]
        source: tera::Error,
    },

    #[error("Failed to render template: {template_name}")]
    RenderFailed {
        template_name: String,
        #[source]
        source: tera::Error,
    },

    #[error("Template error: {0}")]
    Tera(#[from] tera::Error),

    #[error("Invalid path encoding")]
    InvalidPathEncoding,
}

/// Errors related to repository scanning.
#[derive(Debug, Error)]
pub enum RepoError {
    #[error("Failed to scan folder: {path}")]
    ScanFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to canonicalize path: {path}")]
    CanonicalizeFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to serialize repository to JSON")]
    JsonSerializeFailed(#[from] serde_json::Error),

    #[error("Failed to read file metadata: {path}")]
    MetadataFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to get file times: {0}")]
    SystemTimeFailed(#[from] std::time::SystemTimeError),

    #[error("Invalid UTF-8 in path: {path}")]
    InvalidUtf8Path { path: PathBuf },
}

/// Errors related to the browser/GUI window.
#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("Failed to create window")]
    WindowCreationFailed(#[source] tao::error::OsError),

    #[error("Failed to create webview")]
    WebViewCreationFailed(#[source] wry::Error),

    #[error("Failed to load icon")]
    IconLoadFailed(String),

    #[error("Failed to create icon from RGBA data")]
    IconCreationFailed(#[source] tao::window::BadIcon),
}

/// Errors related to file watching.
#[derive(Debug, Error)]
pub enum WatcherError {
    #[error("Failed to initialize file watcher")]
    WatcherInit(#[source] notify::Error),

    #[error("Failed to watch path: {path}")]
    WatchFailed {
        path: PathBuf,
        #[source]
        source: notify::Error,
    },

    #[error("Failed to send file change event")]
    BroadcastFailed,
}

/// Errors related to search functionality.
#[derive(Debug, Error)]
pub enum SearchError {
    #[error("Invalid search pattern: {pattern}")]
    PatternInvalid { pattern: String, reason: String },

    #[error("Search failed: {0}")]
    SearchFailed(String),

    #[error("File read error during search: {}", path.display())]
    FileReadError {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Errors related to static site building.
#[derive(Debug, Error)]
pub enum BuildError {
    #[error("Static site generation is not supported on Windows")]
    UnsupportedPlatform,

    #[error("Failed to create output directory: {}", path.display())]
    CreateDirFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to render markdown: {}", path.display())]
    RenderFailed {
        path: PathBuf,
        #[source]
        source: Box<MbrError>,
    },

    #[error("Failed to write output file: {}", path.display())]
    WriteFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to create symlink: {} -> {}", link.display(), target.display())]
    SymlinkFailed {
        target: PathBuf,
        link: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to copy file: {} -> {}", from.display(), to.display())]
    CopyFailed {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Repository scan failed")]
    RepoScan(#[from] RepoError),

    #[error("Template error")]
    Template(#[from] TemplateError),

    #[error("Markdown error")]
    Markdown(#[from] MarkdownError),

    #[error("Configuration error")]
    Config(#[from] ConfigError),
}

// Convenience type alias for Results using MbrError
pub type Result<T> = std::result::Result<T, MbrError>;

// Conversion from Box<dyn std::error::Error> for gradual migration
impl From<Box<dyn std::error::Error>> for MbrError {
    fn from(err: Box<dyn std::error::Error>) -> Self {
        // This is a fallback for gradual migration
        // In practice, we should use specific error types
        MbrError::Io(std::io::Error::other(err.to_string()))
    }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for MbrError {
    fn from(err: Box<dyn std::error::Error + Send + Sync>) -> Self {
        MbrError::Io(std::io::Error::other(err.to_string()))
    }
}
