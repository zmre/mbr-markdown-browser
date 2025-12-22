//! mbr - Markdown Browser
//!
//! A markdown previewer, browser, and static site generator.

pub mod audio;
pub mod browser;
pub mod build;
pub mod cli;
pub mod config;
pub mod errors;
pub mod html;
pub mod link_transform;
pub mod markdown;
pub mod media;
pub mod oembed;
pub mod path_resolver;
pub mod repo;
pub mod search;
pub mod server;
pub mod templates;
pub mod vid;
pub mod watcher;

pub use build::{BuildStats, Builder};
pub use config::Config;
pub use errors::{BuildError, ConfigError, MbrError, SearchError};
pub use search::{SearchEngine, SearchQuery, SearchResponse, SearchResult, SearchScope};
