//! mbr - Markdown Browser
//!
//! A markdown previewer, browser, and static site generator.
//!
//! This is the application crate of a Cargo workspace. The work lives in the
//! member crates — `mbr-core` (rendering, config, repository scan, search,
//! indexes, assets), `mbr-server` (HTTP server and watcher), `mbr-ssg` (static
//! site generator) and `mbr-ffi` (the UniFFI surface for the Apple shells) —
//! and this crate adds what only the `mbr` binary needs: the CLI, the native
//! GUI window and its OS integrations.
//!
//! Everything is re-exported at the paths it had before the split
//! (`mbr::markdown`, `mbr::server`, `mbr::build`, …), which is what the
//! integration tests and benchmarks are written against. Subsystems are Cargo
//! features (`server`, `watcher`, `ssg`, `cli`, `gui`, `media-metadata`,
//! `ffi`; see Cargo.toml); with none of them this is the render core alone.

pub use mbr_core::*;

#[cfg(feature = "gui")]
pub mod browser;
#[cfg(feature = "cli")]
pub mod cli;
pub mod errors;
#[cfg(feature = "gui")]
pub mod external_open;
pub mod launch_url;
#[cfg(feature = "gui")]
mod macos_open;
#[cfg(feature = "gui")]
pub mod open_picker;

#[cfg(feature = "watcher")]
pub use mbr_server::watcher;
#[cfg(feature = "server")]
pub use mbr_server::{file_write, link_grep, link_rewrite, server};

#[cfg(feature = "ssg")]
pub use mbr_ssg::build;
#[cfg(feature = "ssg")]
pub use mbr_ssg::{BuildStats, Builder};

#[cfg(all(feature = "ffi", target_vendor = "apple"))]
pub use mbr_ffi::quicklook;
#[cfg(all(feature = "ffi", target_vendor = "apple"))]
pub use mbr_ffi::{
    PreviewAttachment, PreviewDocument, QuickLookConfig, QuickLookError, find_config_root,
    render_preview, render_preview_with_config,
};
