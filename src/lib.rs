//! mbr - Markdown Browser
//!
//! A markdown previewer, browser, and static site generator.

// Include the UniFFI scaffolding generated from mbr.udl (only when ffi feature is enabled)
// This must be in the crate root (lib.rs) for UniFFI to work properly
#[cfg(feature = "ffi")]
uniffi::include_scaffolding!("mbr");

pub mod audio;
#[cfg(feature = "gui")]
pub mod browser;
pub mod build;
pub mod cli;
pub mod config;
pub mod embedded_hljs;
pub mod embedded_pico;
pub mod errors;
pub mod html;
pub mod link_transform;
pub mod markdown;
pub mod media;
pub mod oembed;
pub mod oembed_cache;
pub mod path_resolver;
#[cfg(feature = "ffi")]
pub mod quicklook;
pub mod repo;
pub mod search;
pub mod server;
pub mod sorting;
pub mod templates;
pub mod vid;
#[cfg(feature = "media-metadata")]
pub mod video_metadata;
#[cfg(feature = "media-metadata")]
pub mod video_metadata_cache;
pub mod watcher;

pub use build::{BuildStats, Builder};
pub use config::{Config, SortField};
#[cfg(feature = "media-metadata")]
pub use errors::MetadataError;
pub use errors::{BuildError, ConfigError, MbrError, SearchError};
#[cfg(feature = "ffi")]
pub use quicklook::{QuickLookConfig, QuickLookError, render_preview, render_preview_with_config};
pub use search::{SearchEngine, SearchQuery, SearchResponse, SearchResult, SearchScope};
pub use sorting::sort_files;
