//! mbr-server — mbr's HTTP server.
//!
//! The axum router and its handlers (`server`), the live-reload change
//! channel, the edit/move/rename endpoints and the modules only they use:
//! atomic file writes (`file_write`), inbound-link grepping (`link_grep`) and
//! link rewriting on move (`link_rewrite`). With the `watcher` feature, the
//! filesystem watcher (`watcher`, notify) that feeds live reload and index
//! invalidation from disk; without it the change channel still carries the
//! server's own writes. `live` puts a server behind one listener so a
//! changed `.mbr/config.toml` is applied without a restart.
//!
//! Built on `mbr-core`; nothing here is reachable from the core.

pub mod errors;
pub mod file_write;
pub mod link_grep;
pub mod link_rewrite;
pub mod live;
pub mod server;
mod vault_body;
#[cfg(feature = "watcher")]
pub mod watcher;

pub use errors::ServerError;
pub use live::{ConfigReader, LiveServer, ReloadOutcome};
pub use server::{EmbeddedStorage, Server, ServerConfig};
