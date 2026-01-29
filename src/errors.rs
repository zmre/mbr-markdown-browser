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
    ConfigParse(Box<figment::Error>),

    #[error("Markdown error: {0}")]
    Markdown(#[from] MarkdownError),

    #[error("Template error: {0}")]
    Template(#[from] TemplateError),

    #[error("Repository error: {0}")]
    Repo(#[from] RepoError),

    #[cfg(feature = "gui")]
    #[error("Browser error: {0}")]
    Browser(#[from] BrowserError),

    #[error("Watcher error: {0}")]
    Watcher(#[from] WatcherError),

    #[error("Build error: {0}")]
    Build(Box<BuildError>),

    #[cfg(feature = "media-metadata")]
    #[error("Video metadata error: {0}")]
    Metadata(#[from] MetadataError),

    #[cfg(feature = "media-metadata")]
    #[error("PDF metadata error: {0}")]
    PdfMetadata(#[from] PdfMetadataError),

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
    ParseFailed(Box<figment::Error>),

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

    #[error(
        "Invalid host address: {host}. Must be a valid IPv4 address (e.g., 127.0.0.1 or 0.0.0.0)"
    )]
    InvalidHost { host: String },

    #[error("Invalid port: {port}. Port must be between 1 and 65535")]
    InvalidPort { port: u16 },

    #[error("Invalid sidebar_max_items: {value}. Must be greater than 0")]
    InvalidSidebarMaxItems { value: usize },

    #[error("Invalid build_concurrency: {value}. Must be greater than 0")]
    InvalidBuildConcurrency { value: usize },
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
#[cfg(feature = "gui")]
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

    #[error("Server failed to start for new folder")]
    ServerStartFailed,
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

/// Errors related to video metadata extraction.
#[cfg(feature = "media-metadata")]
#[derive(Debug, Error)]
pub enum MetadataError {
    #[error("FFmpeg initialization failed")]
    InitFailed,

    #[error("Failed to open video file: {}", path.display())]
    OpenFailed {
        path: PathBuf,
        #[source]
        source: ffmpeg_next::Error,
    },

    #[error("No video stream found in file: {}", path.display())]
    NoVideoStream { path: PathBuf },

    #[error("No subtitle stream found in file: {}", path.display())]
    NoSubtitleStream { path: PathBuf },

    #[error("No chapters found in file: {}", path.display())]
    NoChapters { path: PathBuf },

    #[error("Failed to decode video frame: {0}")]
    DecodeFailed(String),

    #[error("Failed to encode image: {0}")]
    EncodeFailed(String),

    #[error("Video too short for thumbnail (duration: {duration_secs:.1}s)")]
    VideoTooShort { duration_secs: f64 },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Errors related to PDF metadata and cover extraction.
#[cfg(feature = "media-metadata")]
#[derive(Debug, Error)]
pub enum PdfMetadataError {
    #[error("Failed to open PDF file: {}", path.display())]
    OpenFailed {
        path: PathBuf,
        #[source]
        source: lopdf::Error,
    },

    #[error("Failed to initialize PDF renderer")]
    RendererInitFailed,

    #[error("PDF has no pages: {}", path.display())]
    NoPages { path: PathBuf },

    #[error("Failed to render PDF page: {0}")]
    RenderFailed(String),

    #[error("Failed to encode image: {0}")]
    EncodeFailed(String),

    #[error("PDF is password-protected: {}", path.display())]
    PasswordProtected { path: PathBuf },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
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
    Config(Box<ConfigError>),
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

// Auto-box BuildError when converting to MbrError
impl From<BuildError> for MbrError {
    fn from(err: BuildError) -> Self {
        MbrError::Build(Box::new(err))
    }
}

// Auto-box ConfigError when converting to BuildError
impl From<ConfigError> for BuildError {
    fn from(err: ConfigError) -> Self {
        BuildError::Config(Box::new(err))
    }
}

// Auto-box figment::Error when converting to MbrError
impl From<figment::Error> for MbrError {
    fn from(err: figment::Error) -> Self {
        MbrError::ConfigParse(Box::new(err))
    }
}

// Auto-box figment::Error when converting to ConfigError
impl From<figment::Error> for ConfigError {
    fn from(err: figment::Error) -> Self {
        ConfigError::ParseFailed(Box::new(err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error as IoError, ErrorKind};

    // ==================== Error Display Tests ====================

    #[test]
    fn test_mbr_error_display() {
        let io_err = MbrError::Io(IoError::new(ErrorKind::NotFound, "file not found"));
        assert!(io_err.to_string().contains("file not found"));

        let config_err = MbrError::Config(ConfigError::RootDirNotFound {
            path: PathBuf::from("/test/path"),
        });
        assert!(config_err.to_string().contains("Configuration error"));
    }

    #[test]
    fn test_server_error_display() {
        let err = ServerError::BindFailed {
            addr: "127.0.0.1:8080".to_string(),
            source: IoError::new(ErrorKind::AddrInUse, "address in use"),
        };
        assert!(err.to_string().contains("127.0.0.1:8080"));
        assert!(err.to_string().contains("Failed to bind"));
    }

    #[test]
    fn test_config_error_display() {
        let err = ConfigError::RootDirNotFound {
            path: PathBuf::from("/missing/root"),
        };
        assert!(err.to_string().contains("/missing/root"));

        let err = ConfigError::InvalidHost {
            host: "invalid-host".to_string(),
        };
        assert!(err.to_string().contains("invalid-host"));
        assert!(err.to_string().contains("IPv4"));

        let err = ConfigError::InvalidPort { port: 0 };
        assert!(err.to_string().contains("0"));
        assert!(err.to_string().contains("Port must be between 1 and 65535"));

        let err = ConfigError::InvalidSidebarMaxItems { value: 0 };
        assert!(err.to_string().contains("sidebar_max_items"));
        assert!(err.to_string().contains("greater than 0"));

        let err = ConfigError::InvalidBuildConcurrency { value: 0 };
        assert!(err.to_string().contains("build_concurrency"));
        assert!(err.to_string().contains("greater than 0"));
    }

    #[test]
    fn test_markdown_error_display() {
        let err = MarkdownError::ReadFailed {
            path: PathBuf::from("/test/doc.md"),
            source: IoError::new(ErrorKind::NotFound, "not found"),
        };
        assert!(err.to_string().contains("/test/doc.md"));

        let err = MarkdownError::OembedFetchFailed {
            url: "https://example.com".to_string(),
        };
        assert!(err.to_string().contains("https://example.com"));
    }

    #[test]
    fn test_template_error_display() {
        let err = TemplateError::RenderFailed {
            template_name: "index.html".to_string(),
            source: tera::Error::msg("missing variable"),
        };
        assert!(err.to_string().contains("index.html"));
    }

    #[test]
    fn test_build_error_display() {
        let err = BuildError::UnsupportedPlatform;
        assert!(err.to_string().contains("Windows"));

        let err = BuildError::SymlinkFailed {
            target: PathBuf::from("/target"),
            link: PathBuf::from("/link"),
            source: IoError::new(ErrorKind::PermissionDenied, "permission denied"),
        };
        assert!(err.to_string().contains("/target"));
        assert!(err.to_string().contains("/link"));
    }

    // ==================== Error Conversion Tests ====================

    #[test]
    fn test_box_dyn_error_to_mbr_error() {
        let original: Box<dyn std::error::Error> = Box::new(IoError::other("test error"));
        let converted: MbrError = original.into();

        // Should convert to Io variant with message preserved
        match converted {
            MbrError::Io(io_err) => {
                assert!(io_err.to_string().contains("test error"));
            }
            _ => panic!("Expected MbrError::Io, got {:?}", converted),
        }
    }

    #[test]
    fn test_box_dyn_error_send_sync_to_mbr_error() {
        let original: Box<dyn std::error::Error + Send + Sync> =
            Box::new(IoError::other("sync error"));
        let converted: MbrError = original.into();

        match converted {
            MbrError::Io(io_err) => {
                assert!(io_err.to_string().contains("sync error"));
            }
            _ => panic!("Expected MbrError::Io, got {:?}", converted),
        }
    }

    #[test]
    fn test_build_error_to_mbr_error() {
        let build_err = BuildError::UnsupportedPlatform;
        let mbr_err: MbrError = build_err.into();

        match mbr_err {
            MbrError::Build(boxed) => {
                assert!(matches!(*boxed, BuildError::UnsupportedPlatform));
            }
            _ => panic!("Expected MbrError::Build, got {:?}", mbr_err),
        }
    }

    #[test]
    fn test_config_error_to_build_error() {
        let config_err = ConfigError::RootDirNotFound {
            path: PathBuf::from("/test"),
        };
        let build_err: BuildError = config_err.into();

        match build_err {
            BuildError::Config(boxed) => {
                assert!(matches!(*boxed, ConfigError::RootDirNotFound { .. }));
            }
            _ => panic!("Expected BuildError::Config, got {:?}", build_err),
        }
    }

    #[test]
    fn test_figment_error_to_mbr_error() {
        // Create a figment error by trying to parse invalid TOML
        let figment_err = figment::Error::from("test figment error".to_string());
        let mbr_err: MbrError = figment_err.into();

        match mbr_err {
            MbrError::ConfigParse(boxed) => {
                assert!(boxed.to_string().contains("test figment error"));
            }
            _ => panic!("Expected MbrError::ConfigParse, got {:?}", mbr_err),
        }
    }

    #[test]
    fn test_figment_error_to_config_error() {
        let figment_err = figment::Error::from("parse failed".to_string());
        let config_err: ConfigError = figment_err.into();

        match config_err {
            ConfigError::ParseFailed(boxed) => {
                assert!(boxed.to_string().contains("parse failed"));
            }
            _ => panic!("Expected ConfigError::ParseFailed, got {:?}", config_err),
        }
    }

    // ==================== Auto From Derive Tests ====================

    #[test]
    fn test_io_error_to_mbr_error() {
        let io_err = IoError::new(ErrorKind::NotFound, "file missing");
        let mbr_err: MbrError = io_err.into();

        match mbr_err {
            MbrError::Io(err) => {
                assert_eq!(err.kind(), ErrorKind::NotFound);
                assert!(err.to_string().contains("file missing"));
            }
            _ => panic!("Expected MbrError::Io, got {:?}", mbr_err),
        }
    }

    #[test]
    fn test_server_error_to_mbr_error() {
        let server_err = ServerError::TracingInit;
        let mbr_err: MbrError = server_err.into();

        match mbr_err {
            MbrError::Server(ServerError::TracingInit) => {}
            _ => panic!("Expected MbrError::Server(TracingInit), got {:?}", mbr_err),
        }
    }

    #[test]
    fn test_markdown_error_to_mbr_error() {
        let md_err = MarkdownError::YamlParseFailed;
        let mbr_err: MbrError = md_err.into();

        match mbr_err {
            MbrError::Markdown(MarkdownError::YamlParseFailed) => {}
            _ => panic!(
                "Expected MbrError::Markdown(YamlParseFailed), got {:?}",
                mbr_err
            ),
        }
    }

    #[test]
    fn test_template_error_to_mbr_error() {
        let tpl_err = TemplateError::InvalidPathEncoding;
        let mbr_err: MbrError = tpl_err.into();

        match mbr_err {
            MbrError::Template(TemplateError::InvalidPathEncoding) => {}
            _ => panic!(
                "Expected MbrError::Template(InvalidPathEncoding), got {:?}",
                mbr_err
            ),
        }
    }

    #[test]
    fn test_repo_error_to_mbr_error() {
        let repo_err = RepoError::InvalidUtf8Path {
            path: PathBuf::from("/bad/path"),
        };
        let mbr_err: MbrError = repo_err.into();

        match mbr_err {
            MbrError::Repo(RepoError::InvalidUtf8Path { path }) => {
                assert_eq!(path, PathBuf::from("/bad/path"));
            }
            _ => panic!(
                "Expected MbrError::Repo(InvalidUtf8Path), got {:?}",
                mbr_err
            ),
        }
    }

    // ==================== Error Chain Tests ====================

    #[test]
    fn test_error_chain_preserves_source() {
        use std::error::Error;

        let io_err = IoError::new(ErrorKind::PermissionDenied, "access denied");
        let config_err = ConfigError::CanonicalizeFailed {
            path: PathBuf::from("/test"),
            source: io_err,
        };

        // Check that source() returns the underlying IO error
        let source = config_err.source();
        assert!(source.is_some());
        assert!(source.unwrap().to_string().contains("access denied"));
    }

    #[test]
    fn test_nested_error_chain() {
        use std::error::Error;

        let io_err = IoError::new(ErrorKind::NotFound, "file not found");
        let md_err = MarkdownError::ReadFailed {
            path: PathBuf::from("/test.md"),
            source: io_err,
        };
        let mbr_err: MbrError = md_err.into();

        // MbrError -> MarkdownError -> IoError
        let source1 = mbr_err.source();
        assert!(source1.is_some());

        let source2 = source1.unwrap().source();
        assert!(source2.is_some());
        assert!(source2.unwrap().to_string().contains("file not found"));
    }

    // ==================== Question Mark Operator Tests ====================

    fn fallible_operation() -> std::result::Result<(), IoError> {
        Err(IoError::other("operation failed"))
    }

    fn uses_question_mark() -> Result<()> {
        fallible_operation()?; // Should auto-convert IoError to MbrError
        Ok(())
    }

    #[test]
    fn test_question_mark_conversion() {
        let result = uses_question_mark();
        assert!(result.is_err());

        match result {
            Err(MbrError::Io(io_err)) => {
                assert!(io_err.to_string().contains("operation failed"));
            }
            Err(other) => panic!("Expected MbrError::Io, got {:?}", other),
            Ok(_) => panic!("Expected error, got Ok"),
        }
    }

    // ==================== Edge Case Tests ====================

    #[test]
    fn test_empty_error_message() {
        let err: Box<dyn std::error::Error> = Box::new(IoError::other(""));
        let converted: MbrError = err.into();

        // Should not panic on empty message
        let _ = converted.to_string();
    }

    #[test]
    fn test_unicode_in_error_message() {
        let err: Box<dyn std::error::Error> = Box::new(IoError::other("文件未找到 🔍"));
        let converted: MbrError = err.into();

        match converted {
            MbrError::Io(io_err) => {
                assert!(io_err.to_string().contains("文件未找到"));
                assert!(io_err.to_string().contains("🔍"));
            }
            _ => panic!("Expected MbrError::Io"),
        }
    }

    #[test]
    fn test_long_error_message() {
        let long_msg = "x".repeat(10000);
        let err: Box<dyn std::error::Error> = Box::new(IoError::other(long_msg.clone()));
        let converted: MbrError = err.into();

        match converted {
            MbrError::Io(io_err) => {
                assert_eq!(io_err.to_string().len(), 10000);
            }
            _ => panic!("Expected MbrError::Io"),
        }
    }
}
