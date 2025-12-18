//! mbr - Markdown Browser
//!
//! A markdown previewer, browser, and static site generator.

pub mod browser;
pub mod cli;
pub mod config;
pub mod errors;
pub mod html;
pub mod markdown;
pub mod oembed;
pub mod path_resolver;
pub mod repo;
pub mod server;
pub mod templates;
pub mod vid;

pub use config::Config;
pub use errors::{ConfigError, MbrError};
