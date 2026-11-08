//! A server whose configuration is reloaded without a restart.
//!
//! [`Server`] copies its configuration into the router's state when it is
//! built, and nothing there is mutable. A [`LiveServer`] owns the listener
//! instead, and every request is handed to whichever `Server` is current —
//! one atomic load ([`ArcSwap`]). [`LiveServer::reload`] re-reads the
//! configuration the way the process first read it (a [`ConfigReader`]: the
//! file, the `MBR_*` environment and the command line's overrides) and, when
//! it changed, builds the next server with [`Server::reinit`] and swaps it in:
//! same port and origin (so the pages' `localStorage` survives), same change
//! channel (so live-reload sockets and the watcher carry on), and the same
//! repository index unless the scan settings changed. Requests already
//! running finish on the server they started with.
//!
//! With the `watcher` feature, a change to `<root>/.mbr/config.toml` on disk
//! reloads by itself and then tells open pages to reload
//! ([`ChangeEventType::Reloaded`]); the GUI's Cmd/Ctrl+R calls
//! [`LiveServer::reload`] before reloading the page.
//!
//! **A `config.toml` that no longer loads** is not applied: the server keeps
//! the last configuration that loaded, logs a WARN, and reports the problem on
//! every page's `errors.json` (`config_parse_error`) until it is fixed.
//!
//! The iOS app does not use this: it has no listener and no watcher, and
//! falls back to the defaults on a broken file (`mbr_ffi::mobile`). Both
//! share [`Config::same_settings`] and [`config_error_message`].

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwap;
use axum::Router;
use mbr_core::config::{Config, config_error_message};
use mbr_core::errors::MbrError;

use crate::ServerError;
use crate::server::{Server, ServerConfig, announce_listening, bind_with_retry, serve};
#[cfg(feature = "watcher")]
use mbr_core::change_event::{ChangeEventType, FileChangeEvent};

/// Reads the configuration again, exactly as the process read it at launch:
/// the same entry point (`Config::read`, `Config::read_for_gui`) on the same
/// path, then the same command-line overrides, so flags still beat the file.
/// Called off the async threads.
pub type ConfigReader = Arc<dyn Fn() -> Result<Config, MbrError> + Send + Sync>;

/// How long a change to `config.toml` on disk waits for the rest of an
/// editor's save (write a temp file, rename it over, touch it) before the
/// configuration is read. Shorter than the index's two-second batch window:
/// one small file, and someone is looking at the page.
#[cfg(feature = "watcher")]
const CONFIG_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(300);

/// `config.toml`'s place relative to the repository root.
const CONFIG_FILE: &str = ".mbr/config.toml";

/// What a [`LiveServer::reload`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadOutcome {
    /// The configuration (and its error, if any) is what is already served.
    /// The templates were read again.
    Unchanged,
    /// The changed configuration is being served (this also clears an
    /// earlier [`ReloadOutcome::Rejected`] problem).
    Applied,
    /// `config.toml` does not load. The last configuration that did keeps
    /// being served, and every page's `errors.json` now reports `message`.
    /// Only when the problem is new: the same problem again is
    /// [`ReloadOutcome::Unchanged`].
    Rejected { message: String },
    /// No server could be built for the configuration (a template that does
    /// not parse, say); the current one keeps serving. Logged at WARN.
    Failed { message: String },
}

impl ReloadOutcome {
    /// Whether open pages are out of date: they should reload.
    pub fn pages_changed(&self) -> bool {
        matches!(self, Self::Applied | Self::Rejected { .. })
    }
}

/// The configuration being served and the problem, if the file on disk is
/// broken.
struct Applied {
    config: Config,
    /// The message of the `config.toml` that did not load, as logged.
    problem: Option<String>,
}

/// One listener whose server is rebuilt when its configuration changes. See
/// the [module docs](self).
pub struct LiveServer {
    current: ArcSwap<Server>,
    read_config: ConfigReader,
    gui_mode: bool,
    /// Serializes reloads; holds what the current server was built from.
    applied: tokio::sync::Mutex<Applied>,
    /// `<canonical root>/.mbr/config.toml`, as the watcher reports it.
    config_path: PathBuf,
    /// Dropped with the server, which ends the config watch.
    _shutdown: tokio::sync::watch::Sender<()>,
}

impl LiveServer {
    /// Builds the first server for `config` (already read through
    /// `read_config`'s path, overrides applied) and, with the `watcher`
    /// feature, starts following `.mbr/config.toml`. Must run inside the
    /// tokio runtime, like [`Server::init`].
    pub fn init(
        config: Config,
        gui_mode: bool,
        read_config: ConfigReader,
    ) -> Result<Arc<Self>, ServerError> {
        let server = Server::init(ServerConfig::from(&config).with_gui_mode(gui_mode))?;
        let root = config
            .root_dir
            .canonicalize()
            .unwrap_or_else(|_| config.root_dir.clone());
        let (shutdown, _) = tokio::sync::watch::channel(());
        let live = Arc::new(Self {
            current: ArcSwap::from_pointee(server),
            read_config,
            gui_mode,
            applied: tokio::sync::Mutex::new(Applied {
                config,
                problem: None,
            }),
            config_path: root.join(CONFIG_FILE),
            _shutdown: shutdown,
        });
        #[cfg(feature = "watcher")]
        live.follow_config_file();
        Ok(live)
    }

    /// The server answering requests now.
    pub fn current(&self) -> Arc<Server> {
        self.current.load_full()
    }

    /// A router that hands each request to the current server.
    pub fn router(self: &Arc<Self>) -> Router {
        let live = Arc::clone(self);
        Router::new().fallback_service(tower::service_fn(move |request: axum::extract::Request| {
            // One atomic load; the router is an `Arc` inside.
            let router = live.current.load().router.clone();
            tower::ServiceExt::oneshot(router, request)
        }))
    }

    /// Binds the configured address and serves until stopped.
    pub async fn start(self: &Arc<Self>) -> Result<(), ServerError> {
        self.start_with_port_retry(None, 0).await
    }

    /// [`Self::start`], trying up to `max_retries` higher ports while the
    /// configured one is in use; `ready_tx` receives the bound port.
    pub async fn start_with_port_retry(
        self: &Arc<Self>,
        ready_tx: Option<tokio::sync::oneshot::Sender<u16>>,
        max_retries: u16,
    ) -> Result<(), ServerError> {
        let server = self.current();
        let (listener, port) = bind_with_retry(server.ip, server.port, max_retries).await?;
        let local_addr: SocketAddr = listener
            .local_addr()
            .map_err(ServerError::LocalAddrFailed)?;
        announce_listening(self.gui_mode, local_addr);
        if let Some(tx) = ready_tx
            && tx.send(port).is_err()
        {
            tracing::debug!("Port signal receiver dropped (shutdown in progress)");
        }
        serve(listener, self.router()).await
    }

    /// Reads the configuration again and serves it if it changed. See
    /// [`ReloadOutcome`] for what each case does. Concurrent reloads run one
    /// after the other. Never rescans unless the scan settings changed.
    pub async fn reload(&self) -> ReloadOutcome {
        let mut applied = self.applied.lock().await;
        let read_config = Arc::clone(&self.read_config);
        let read = match tokio::task::spawn_blocking(move || read_config()).await {
            Ok(read) => read,
            Err(e) => {
                let message = format!("configuration read task failed: {e}");
                tracing::warn!("{message}");
                return ReloadOutcome::Failed { message };
            }
        };
        match read {
            Err(error) => {
                let message = config_error_message(&error);
                tracing::warn!(
                    "{} not applied; still serving the configuration loaded before: {message}",
                    self.config_path.display()
                );
                if applied.problem.as_deref() == Some(message.as_str()) {
                    return ReloadOutcome::Unchanged;
                }
                let config = applied.config.clone();
                let problem = format!(
                    "{message} (mbr keeps using the configuration it loaded before, until this is fixed)"
                );
                match self.swap(&config, Some(problem)) {
                    Ok(()) => {
                        applied.problem = Some(message.clone());
                        ReloadOutcome::Rejected { message }
                    }
                    Err(e) => self.failed(&e),
                }
            }
            Ok(mut config) => {
                let relaunch = keep_relaunch_only_settings(&mut config, &applied.config);
                if !relaunch.is_empty() {
                    tracing::info!(
                        "{} changed {}; that takes effect when mbr is relaunched",
                        self.config_path.display(),
                        relaunch.join(", ")
                    );
                }
                if config.same_settings(&applied.config) && applied.problem.is_none() {
                    tracing::debug!("Configuration unchanged; templates re-read");
                    self.current().reload_templates();
                    return ReloadOutcome::Unchanged;
                }
                match self.swap(&config, None) {
                    Ok(()) => {
                        tracing::info!("Applied the changed configuration");
                        *applied = Applied {
                            config,
                            problem: None,
                        };
                        ReloadOutcome::Applied
                    }
                    Err(e) => self.failed(&e),
                }
            }
        }
    }

    fn failed(&self, error: &ServerError) -> ReloadOutcome {
        let message = error.to_string();
        tracing::warn!("Configuration not applied, the server could not be rebuilt: {message}");
        ReloadOutcome::Failed { message }
    }

    /// Builds the server for `config` from the current one and puts it in
    /// its place.
    fn swap(&self, config: &Config, config_problem: Option<String>) -> Result<(), ServerError> {
        let previous = self.current();
        let server_config = ServerConfig {
            config_problem,
            ..ServerConfig::from(config).with_gui_mode(self.gui_mode)
        };
        let next = Server::reinit(server_config, &previous)?;
        self.current.store(Arc::new(next));
        previous.retire();
        Ok(())
    }

    /// Reloads on every change to `config.toml` the watcher reports, after
    /// [`CONFIG_DEBOUNCE`], and tells open pages when they are out of date.
    /// Each event costs one path comparison.
    #[cfg(feature = "watcher")]
    fn follow_config_file(self: &Arc<Self>) {
        let live = Arc::downgrade(self);
        let mut shutdown = self._shutdown.subscribe();
        let mut changes = self.current().file_changes().subscribe();
        let config_path = self.config_path.clone();
        tokio::spawn(async move {
            use tokio::sync::broadcast::error::RecvError;
            loop {
                let received = tokio::select! {
                    _ = shutdown.changed() => break,
                    received = changes.recv() => received,
                };
                let relevant = match received {
                    Ok(event) => is_config_change(&event, &config_path),
                    // Events were dropped; one of them may have been it.
                    Err(RecvError::Lagged(_)) => true,
                    Err(RecvError::Closed) => break,
                };
                if !relevant {
                    continue;
                }
                let deadline = tokio::time::Instant::now() + CONFIG_DEBOUNCE;
                loop {
                    tokio::select! {
                        () = tokio::time::sleep_until(deadline) => break,
                        received = changes.recv() => {
                            if matches!(received, Err(RecvError::Closed)) {
                                break;
                            }
                        }
                    }
                }
                let Some(live) = live.upgrade() else { break };
                if live.reload().await.pages_changed() {
                    live.announce_reload();
                }
            }
        });
    }

    /// Tells every live-reload socket that pages are out of date.
    #[cfg(feature = "watcher")]
    fn announce_reload(&self) {
        // No receivers is not an error: no page is open.
        let _ = self.current().file_changes().send(FileChangeEvent {
            path: self.config_path.to_string_lossy().into_owned(),
            relative_path: CONFIG_FILE.to_owned(),
            event: ChangeEventType::Reloaded,
        });
    }
}

impl Drop for LiveServer {
    /// Stops the current server's background tasks; its channel never closes
    /// on its own (the watcher feeds it), so nothing else would.
    fn drop(&mut self) {
        self.current.load().retire();
    }
}

/// Whether `event` is a change on disk to the file at `config_path`.
#[cfg(feature = "watcher")]
fn is_config_change(event: &FileChangeEvent, config_path: &std::path::Path) -> bool {
    !matches!(event.event, ChangeEventType::Reloaded)
        && std::path::Path::new(&event.path) == config_path
}

/// Puts back, in `next`, the settings a running process cannot change — the
/// address it listens on, the repository root, the GUI's menu bar — and names
/// the ones `next` had changed, so the caller can say they need a relaunch.
fn keep_relaunch_only_settings(next: &mut Config, running: &Config) -> Vec<&'static str> {
    let mut changed = Vec::new();
    if next.host != running.host {
        changed.push("host");
        next.host = running.host;
    }
    if next.port != running.port {
        changed.push("port");
        next.port = running.port;
    }
    if next.gui_menu_bar != running.gui_menu_bar {
        changed.push("gui_menu_bar");
        next.gui_menu_bar = running.gui_menu_bar;
    }
    // Not settings of the file, but derived from where the configuration was
    // found: a root that moved (a `.mbr/` created further up) is a different
    // repository.
    if next.root_dir != running.root_dir {
        changed.push("the repository root");
        next.root_dir = running.root_dir.clone();
    }
    next.explicit_hidden_dirs = running.explicit_hidden_dirs.clone();
    changed
}

#[cfg(test)]
mod tests;
