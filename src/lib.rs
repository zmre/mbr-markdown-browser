//! mbr - Markdown Browser
//!
//! A markdown previewer, browser, and static site generator.

// Include the UniFFI scaffolding generated from mbr.udl (only when ffi feature is enabled)
// This must be in the crate root (lib.rs) for UniFFI to work properly
#[cfg(feature = "ffi")]
uniffi::include_scaffolding!("mbr");

/// Returns a reqwest `ClientBuilder` pre-configured with bundled Mozilla root
/// certificates. This avoids reliance on the system certificate store, which
/// may be absent in sandboxed or minimal Linux environments (e.g. Nix builds).
pub fn http_client_builder() -> reqwest::ClientBuilder {
    use std::sync::Arc;
    let tls_config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("safe default protocol versions")
    .with_root_certificates(Arc::new(rustls::RootCertStore::from_iter(
        webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
    )))
    .with_no_client_auth();
    reqwest::Client::builder().use_preconfigured_tls(tls_config)
}

/// Build a reqwest HTTP client with bundled Mozilla root certificates.
pub fn http_client(timeout: std::time::Duration) -> reqwest::Client {
    http_client_builder()
        .timeout(timeout)
        .build()
        .expect("failed to build HTTP client")
}

pub mod attrs;
pub mod audio;
#[cfg(feature = "gui")]
pub mod browser;
pub mod build;
pub mod cli;
pub mod config;
pub mod constants;
pub mod embedded_hljs;
pub mod embedded_katex;
pub mod embedded_pico;
pub mod errors;
pub mod html;
pub mod link_grep;
pub mod link_index;
pub mod link_transform;
pub mod markdown;
pub mod media;
pub mod oembed;
pub mod oembed_cache;
pub mod path_resolver;
#[cfg(feature = "media-metadata")]
pub mod pdf_metadata;
#[cfg(feature = "ffi")]
pub mod quicklook;
pub mod repo;
pub mod search;
pub mod server;
pub mod sorting;
pub mod tag_index;
pub mod templates;
pub mod vid;
#[cfg(feature = "media-metadata")]
pub mod video_metadata;
#[cfg(feature = "media-metadata")]
pub mod video_metadata_cache;
#[cfg(feature = "media-metadata")]
pub mod video_transcode;
#[cfg(feature = "media-metadata")]
pub mod video_transcode_cache;
pub mod watcher;
pub mod wikilink;

pub use build::{BuildStats, Builder};
pub use config::{Config, SortField, TagSource, find_root_dir};
#[cfg(feature = "media-metadata")]
pub use errors::MetadataError;
#[cfg(feature = "media-metadata")]
pub use errors::PdfMetadataError;
pub use errors::{BuildError, ConfigError, MbrError, SearchError};
pub use markdown::MarkdownRenderResult;
#[cfg(feature = "ffi")]
pub use quicklook::{
    QuickLookConfig, QuickLookError, find_config_root, render_preview, render_preview_with_config,
};
pub use search::{SearchEngine, SearchQuery, SearchResponse, SearchResult, SearchScope};
pub use sorting::sort_files;
#[cfg(feature = "media-metadata")]
pub use video_transcode::TranscodeError;
