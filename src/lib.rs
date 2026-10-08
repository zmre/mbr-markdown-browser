//! mbr - Markdown Browser
//!
//! A markdown previewer, browser, and static site generator.
//!
//! Subsystems are Cargo features (`server`, `watcher`, `ssg`, `cli`, `gui`,
//! `media-metadata`, `ffi`; see Cargo.toml). With none of them this is the
//! render core alone, which is what the QuickLook staticlib links.

// UniFFI's generated UDL scaffolding (`OUT_DIR/mbr.uniffi.rs`) holds its
// interface metadata in one large `const` byte array
// (`UNIFFI_META_CONST_UDL_MBR`), which trips `clippy::large_const_arrays`.
// The generated code is not ours to change, and an `#[allow]` on the
// `include_scaffolding!` call is discarded as an unused attribute. Wrapping
// the include in a module would scope the allow but move `UniFfiTag` off the
// crate root, where UniFFI's proc-macro exports look for it. So the allow is
// crate-wide, and only when `ffi` is on.
#![cfg_attr(feature = "ffi", allow(clippy::large_const_arrays))]

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

pub mod assets;
pub mod attrs;
pub mod audio;
#[cfg(feature = "gui")]
pub mod browser;
#[cfg(feature = "ssg")]
pub mod build;
pub mod cache;
pub mod change_event;
pub mod chat;
#[cfg(feature = "cli")]
pub mod cli;
pub mod config;
pub mod constants;
pub mod contact;
pub mod edit_auth;
pub mod embedded_hljs;
pub mod embedded_katex;
pub mod embedded_pico;
pub mod errors;
#[cfg(feature = "gui")]
pub mod external_open;
pub mod file_write;
pub mod flashcards;
pub mod html;
pub mod launch_url;
pub mod link_grep;
pub mod link_index;
pub mod link_rewrite;
pub mod link_transform;
#[cfg(feature = "gui")]
mod macos_open;
pub mod markdown;
pub mod media;
pub mod oembed;
pub mod oembed_cache;
#[cfg(feature = "gui")]
pub mod open_picker;
pub mod page_context;
pub mod page_errors;
pub mod path_resolver;
#[cfg(feature = "media-metadata")]
pub mod pdf_metadata;
#[cfg(feature = "ffi")]
pub mod quicklook;
pub mod readability;
pub mod relationships;
pub mod repo;
pub mod search;
#[cfg(feature = "server")]
pub mod server;
pub mod sorting;
pub mod tag_index;
pub mod task_index;
pub mod task_query;
pub mod tasks;
pub mod templates;
#[cfg(test)]
mod test_support;
pub mod url_helpers;
pub mod url_path;
pub mod vid;
#[cfg(feature = "media-metadata")]
pub mod video_metadata;
#[cfg(feature = "media-metadata")]
pub mod video_metadata_cache;
#[cfg(feature = "media-metadata")]
pub mod video_remux;
#[cfg(feature = "media-metadata")]
pub mod video_transcode;
#[cfg(feature = "media-metadata")]
pub mod video_transcode_cache;
#[cfg(feature = "watcher")]
pub mod watcher;
pub mod wikilink;
pub mod wikilink_index;

#[cfg(feature = "ssg")]
pub use build::{BuildStats, Builder};
pub use config::{Config, RelationType, SortField, TagSource, find_root_dir};
#[cfg(feature = "media-metadata")]
pub use errors::MetadataError;
#[cfg(feature = "media-metadata")]
pub use errors::PdfMetadataError;
pub use errors::{BuildError, ConfigError, MbrError, SearchError, TaskIndexError};
pub use markdown::{MarkdownRenderResult, ParsedDocument};
pub use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Tag, TagEnd,
};
#[cfg(feature = "ffi")]
pub use quicklook::{
    PreviewAttachment, PreviewDocument, QuickLookConfig, QuickLookError, find_config_root,
    render_preview, render_preview_with_config,
};
pub use search::{SearchEngine, SearchQuery, SearchResponse, SearchResult, SearchScope};
pub use sorting::sort_files;
pub use task_index::{FileTasks, TaskIndex};
pub use task_query::{
    DueBucket, DueFilter, IncludeFilter, TaskGroup, TaskMode, TaskQuery, TaskQueryResponse,
    due_bucket, parse_task_query, run_query,
};
pub use tasks::{
    Annotations, MarkerRule, Task, TaskKind, TaskPriority, TaskStatus, parse_marker_line,
    parse_task_line, scan_source_tasks, scan_source_tasks_with_markers, set_marker,
    strip_annotations,
};
#[cfg(feature = "media-metadata")]
pub use video_transcode::TranscodeError;
