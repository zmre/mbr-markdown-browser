//! The iOS app's core: an open vault, served in-process to the app's web view.
//!
//! The app renders in a WebKit view whose `mbr://localhost/…` requests are
//! handled by a Swift `URLSchemeHandler`. That handler passes each request to
//! [`MbrApp::handle`], which runs it through mbr's own axum router — the
//! desktop server minus its listener — so every page, asset, JSON endpoint and
//! light write (task toggles, flashcard reviews) works unchanged. Review notes
//! live in the page's `localStorage` and need nothing here.
//!
//! An [`MbrApp`] is one open vault:
//!
//! - **Storage**: a [`CoordinatedVault`] over the picked folder — reads in
//!   Rust on `std::fs`, writes and download requests through the app's
//!   [`FileCoordinator`], not-downloaded files reported as such — or a
//!   [`SingleFileVault`] for one file opened in place ([`MbrApp::open_file`]).
//! - **Threads**: its own small tokio runtime plus the process's rayon pool,
//!   every thread failing fast on dataless files ([`iopolicy`]).
//! - **Index**: scanned on open, or restored from the snapshot in the app's
//!   `Caches/` ([`index_cache`]; the mobile-only exception to mbr's no-cache
//!   rule) and then brought up to date by a stat-diff, all in the background:
//!   the router answers from the moment [`MbrApp::open`] returns, and
//!   [`MbrApp::wait_until_ready`] reports when indexing is done.
//! - **Freshness**: no watcher. [`MbrApp::refresh`] re-lists and applies what
//!   changed; the app calls it on foreground and pull-to-refresh, and reloads
//!   the page (with `evaluateJavaScript("location.reload()")`, since a
//!   WebSocket cannot cross a scheme handler) when the summary says the shown
//!   page or the navigation changed.
//! - **`.mbr/`**: the same refresh notices edits to the vault's `.mbr/` folder
//!   (and a `template_folder`) from a stat-only fingerprint ([`mbr_dir`]):
//!   templates reload in place, a changed `config.toml` rebuilds the server
//!   inside this `MbrApp` (same vault, cache, identity and coordinator; the
//!   index carried over unless the scan configuration changed), and asset
//!   overrides need nothing but the page reload.
//!
//! Everything compiles on every platform with the `mobile` feature; the
//! UniFFI exports and the dataless-file handling are Apple-only.

mod coordinated_vault;
mod coordinator;
mod index_cache;
mod iopolicy;
mod mbr_dir;
mod runtime;
mod single_file;
mod transport;

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use axum::Router;
use mbr_core::config::{Config, IpArray};
use mbr_core::repo::{RepoDiff, build_markdown_url_path, is_markdown_extension};
use mbr_core::vault::Vault;
use mbr_server::{EmbeddedStorage, Server, ServerConfig};

pub use coordinated_vault::CoordinatedVault;
pub use coordinator::{CoordinatorError, FileCoordinator};
pub use iopolicy::{materializes_dataless_files, set_materialize_dataless_files};
pub use single_file::SingleFileVault;
pub use transport::{MOBILE_HOST, MbrResponse, MobileHeader};

use index_cache::IndexCache;
use mbr_dir::MbrStamp;
use runtime::BackgroundRuntime;

/// Log filter for the in-process server (stdout ends up in the device log).
const LOG_FILTER: &str = "mbr=info,tower_http=warn";

/// Errors crossing the FFI boundary.
#[derive(Debug, thiserror::Error)]
#[cfg_attr(target_vendor = "apple", derive(uniffi::Error))]
pub enum MobileError {
    /// The vault could not be opened (bad path, config, templates, runtime).
    #[error("could not open: {message}")]
    Open { message: String },
    /// The request could not be turned into an HTTP request.
    #[error("invalid request: {message}")]
    InvalidRequest { message: String },
    /// A background task or a response body failed.
    #[error("{message}")]
    Internal { message: String },
}

fn open_error(message: impl std::fmt::Display) -> MobileError {
    MobileError::Open {
        message: message.to_string(),
    }
}

/// What a [`MbrApp::refresh`] (or the stat-diff after a restore) changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(target_vendor = "apple", derive(uniffi::Record))]
pub struct RefreshSummary {
    /// Files that appeared.
    pub added: u32,
    /// Files whose size or modification time changed, or that finished
    /// downloading.
    pub changed: u32,
    /// Files that are gone.
    pub removed: u32,
    /// Files the provider evicted (still listed, content no longer on the
    /// device). Their pages and metadata are unchanged.
    pub evicted: u32,
    /// URL paths (`/notes/idea/`) of the notes added, changed or removed: if
    /// the page on screen is one of them, reload it; if any were added or
    /// removed, the navigation changed too.
    pub changed_pages: Vec<String>,
    pub duration_ms: u64,
    /// `.mbr/config.toml` changed and the vault now runs with it (or with the
    /// defaults, when it no longer parses: see `config_error`). Reload the
    /// page.
    pub config_reloaded: bool,
    /// A template in `.mbr/` (or the template folder) changed and the
    /// templates were reloaded. Reload the page.
    pub templates_reloaded: bool,
    /// Another file in `.mbr/` changed (`theme.css`, `user.css`, a
    /// component). Assets are read per request, so reloading the page shows
    /// it.
    pub assets_changed: bool,
    /// The configuration problem after this refresh, as
    /// [`MbrApp::config_error`] reports it: set when a `config.toml` broke,
    /// cleared when it was fixed.
    pub config_error: Option<String>,
    /// Files in `.mbr/` (or the template folder) still not downloaded, whose
    /// download this refresh asked for again. When non-zero, refresh again in
    /// a few seconds: their arrival shows up as `config_reloaded`,
    /// `templates_reloaded` or `assets_changed`.
    pub mbr_downloads_requested: u32,
}

impl RefreshSummary {
    /// Whether anything a reader could see changed.
    pub fn is_empty(&self) -> bool {
        self.added == 0 && self.changed == 0 && self.removed == 0
    }

    /// Whether anything in `.mbr/` changed what pages look like.
    pub fn mbr_changed(&self) -> bool {
        self.config_reloaded || self.templates_reloaded || self.assets_changed
    }
}

/// How the initial indexing went. See [`MbrApp::wait_until_ready`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(target_vendor = "apple", derive(uniffi::Record))]
pub struct OpenReport {
    /// The index came from the saved snapshot rather than a scan.
    pub from_cache: bool,
    /// Files restored from the snapshot (0 when scanned).
    pub restored_files: u64,
    /// Notes plus other files indexed once ready.
    pub indexed_files: u64,
    /// From `open` to navigation and `site.json` being answerable.
    pub scan_ms: u64,
    /// From `open` to everything below being done, snapshot saved included.
    pub ready_ms: u64,
    /// What the stat-diff after a restore found (empty after a scan).
    pub refresh: RefreshSummary,
    /// Why the vault's `.mbr/config.toml` was not used, when it could not be
    /// loaded: the vault opened with the default configuration instead (see
    /// [`MbrApp::config_error`]).
    pub config_error: Option<String>,
    /// Files in `.mbr/` (or the template folder) the provider had not
    /// downloaded at open, whose download was requested then. Nothing else
    /// asks for them — the scan skips hidden folders — so until they arrive
    /// the vault runs without them (default templates, configuration or
    /// styles). When non-zero, [`MbrApp::refresh`] a few seconds later picks
    /// them up.
    pub mbr_downloads_requested: u32,
}

/// An open vault. See the [module docs](self).
#[cfg_attr(target_vendor = "apple", derive(uniffi::Object))]
pub struct MbrApp {
    /// Declared first so it is dropped last: the server's tasks run on it.
    runtime: BackgroundRuntime,
    /// The server for the current configuration. Replaced, whole, when a
    /// refresh finds `.mbr/config.toml` changed; requests already running
    /// finish on the one they started with.
    served: Arc<RwLock<Arc<Served>>>,
    /// The storage every server of this vault reads, kept for a rebuild.
    vault: Arc<dyn Vault>,
    root: PathBuf,
    /// Where `/` redirects (a single-file vault's page).
    home: Option<String>,
    cache: Option<IndexCache>,
    ready: tokio::sync::watch::Receiver<Option<OpenReport>>,
    /// Serializes refreshes, snapshot saves and the initial indexing.
    index_lock: Arc<tokio::sync::Mutex<()>>,
}

/// One configuration's server and what was derived with it.
struct Served {
    router: Router,
    server: Arc<Server>,
    pages: PageUrls,
    /// The configuration it runs with (mobile overrides applied), to tell a
    /// `config.toml` that was only touched from one that changed.
    config: Config,
    config_error: Option<String>,
    /// `.mbr/` as of the last look. Moves forward on every refresh that
    /// finds a change, so each change is acted on once.
    stamp: std::sync::Mutex<MbrStamp>,
}

impl Served {
    /// Builds the server for `loaded` over `vault`. Must run inside the
    /// vault's runtime: `Server::init` spawns its scan and index tasks.
    fn build(
        root: &Path,
        loaded: LoadedConfig,
        vault: Arc<dyn Vault>,
        snapshot: Option<mbr_core::repo::RepoSnapshot>,
        stamp: MbrStamp,
    ) -> Result<Self, MobileError> {
        let LoadedConfig {
            config,
            error: config_error,
        } = loaded;
        let server_config = ServerConfig {
            base_dir: root.to_path_buf(),
            embedded: Some(EmbeddedStorage { vault, snapshot }),
            config_problem: config_error.clone(),
            ..ServerConfig::from(&config).with_log_filter(Some(LOG_FILTER))
        };
        let server = Arc::new(Server::init(server_config).map_err(open_error)?);
        let pages = PageUrls {
            root: server.repo().vault().root().to_path_buf(),
            index_file: config.index_file.clone(),
            markdown_extensions: config.markdown_extensions.clone(),
        };
        Ok(Self {
            router: server.router.clone(),
            server,
            pages,
            config,
            config_error,
            stamp: std::sync::Mutex::new(stamp),
        })
    }
}

/// The server in `slot` now.
fn current(slot: &RwLock<Arc<Served>>) -> Arc<Served> {
    Arc::clone(
        &slot
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}

/// The configuration a vault opens with, and why the repository's own was
/// not used, when it was not.
struct LoadedConfig {
    config: Config,
    error: Option<String>,
}

impl LoadedConfig {
    /// The repository's configuration, read at `root` (never above it).
    ///
    /// Unlike the desktop, which refuses to start on a broken
    /// `.mbr/config.toml`, the app falls back to the defaults: a phone has no
    /// editor to fix the file with, and refusing would lock the reader out of
    /// their notes. The problem is logged at WARN, returned on
    /// [`OpenReport::config_error`] and [`MbrApp::config_error`], and shown on
    /// every page's error panel (`errors.json`'s `config_parse_error`).
    fn read(root: &Path) -> Self {
        match Config::read_at_root(root) {
            Ok(config) => Self {
                config,
                error: None,
            },
            Err(e) => {
                let message = config_error_message(&e);
                tracing::warn!(
                    "{} not used, opening with the default configuration: {message}",
                    root.join(".mbr/config.toml").display()
                );
                Self {
                    config: Config::default(),
                    error: Some(message),
                }
            }
        }
    }
}

/// What went wrong with the configuration, in one line a reader can act on.
///
/// A parse failure's useful part — the file, key and line figment reports —
/// is not in `ConfigError::ParseFailed`'s `Display` and not exposed as its
/// `source`, so it is unwrapped here rather than changing the desktop's
/// error text.
fn config_error_message(error: &mbr_core::MbrError) -> String {
    match error {
        mbr_core::MbrError::Config(mbr_core::errors::ConfigError::ParseFailed(figment)) => {
            format!("{error}: {figment}")
        }
        other => error_chain(other),
    }
}

/// An error and its sources, one line.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !message.contains(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        source = cause.source();
    }
    message
}

#[cfg_attr(target_vendor = "apple", uniffi::export)]
impl MbrApp {
    /// Opens the folder at `root_path` — the path of the security-scoped URL
    /// the folder picker returned, accessed for as long as this lives.
    ///
    /// - `cache_dir`: the app's `Caches/` directory (or a subfolder of it);
    ///   the index snapshot is kept there. `None` keeps nothing.
    /// - `vault_id`: a stable identity for the folder (from its bookmark), so
    ///   a snapshot survives the folder's path changing. Defaults to the
    ///   canonical path.
    /// - `coordinator`: performs writes and downloads; `None` writes with
    ///   plain `std::fs` (only right for the app's own container).
    ///
    /// The picked folder is the root: no upward search for a `.git`, since
    /// its ancestors are outside the grant. Returns as soon as the router can
    /// answer; indexing continues in the background.
    #[cfg_attr(target_vendor = "apple", uniffi::constructor)]
    pub fn open(
        root_path: String,
        cache_dir: Option<String>,
        vault_id: Option<String>,
        coordinator: Option<Arc<dyn FileCoordinator>>,
    ) -> Result<Arc<Self>, MobileError> {
        let started = Instant::now();
        let root = canonical_dir(Path::new(&root_path))?;
        let mut loaded = LoadedConfig::read(&root);
        apply_mobile_overrides(&mut loaded.config, &root);
        let vault: Arc<dyn Vault> = Arc::new(CoordinatedVault::new(&root, coordinator));
        let identity = vault_id.unwrap_or_else(|| root.to_string_lossy().into_owned());
        let cache = cache_dir.map(|dir| IndexCache::new(Path::new(&dir), &identity));
        let snapshot = cache.as_ref().and_then(IndexCache::load);
        Self::start(root, loaded, vault, cache, snapshot, None, started)
    }

    /// Opens one file in place — what iOS grants for a file opened from
    /// Files or the share sheet, and all a provider that cannot grant folders
    /// offers. Only that file is visible ([`SingleFileVault`]); `/` redirects
    /// to its page ([`MbrApp::home_url`]). No index snapshot: one file is
    /// cheap to read. Granting the folder later means opening it with
    /// [`MbrApp::open`] and dropping this.
    #[cfg_attr(target_vendor = "apple", uniffi::constructor)]
    pub fn open_file(
        file_path: String,
        coordinator: Option<Arc<dyn FileCoordinator>>,
    ) -> Result<Arc<Self>, MobileError> {
        let started = Instant::now();
        let given = Path::new(&file_path);
        let file = given.canonicalize().unwrap_or_else(|_| given.to_path_buf());
        if !file.is_file() {
            return Err(open_error(format!("not a file: {}", file.display())));
        }
        let (Some(parent), Some(name)) = (file.parent(), file.file_name().and_then(|n| n.to_str()))
        else {
            return Err(open_error(format!("no folder or name: {}", file.display())));
        };
        let parent = parent.to_path_buf();
        // The folder's own config is beside the file, outside the grant.
        let mut config = Config::default();
        apply_mobile_overrides(&mut config, &parent);
        let extension = file.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !is_markdown_extension(extension, &config.markdown_extensions) {
            return Err(open_error(format!("not a markdown file: {name}")));
        }
        let vault = SingleFileVault::new(CoordinatedVault::new(&parent, coordinator), name)
            .map_err(open_error)?;
        let page =
            build_markdown_url_path(&vault.key(vault.file()), vault.root(), &config.index_file);
        let home = (page != "/").then_some(page);
        let loaded = LoadedConfig {
            config,
            error: None,
        };
        Self::start(parent, loaded, Arc::new(vault), None, None, home, started)
    }

    /// The vault's root folder (canonical).
    pub fn root_path(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }

    /// Whether this is a single file opened in place.
    pub fn is_single_file(&self) -> bool {
        self.home.is_some()
    }

    /// The URL path to load first: `/`, or a single file's page.
    pub fn home_url(&self) -> String {
        self.home.clone().unwrap_or_else(|| "/".to_owned())
    }

    /// Why the vault's `.mbr/config.toml` was not used (it opened with the
    /// default configuration), or `None` when it loaded or there is none.
    pub fn config_error(&self) -> Option<String> {
        current(&self.served).config_error.clone()
    }

    /// The index snapshot's file, when the vault keeps one.
    pub fn index_cache_path(&self) -> Option<String> {
        self.cache
            .as_ref()
            .map(|c| c.path().to_string_lossy().into_owned())
    }

    /// Handles one request from the web view.
    ///
    /// `url` is the full `mbr://localhost/path?query` URL; `body` is empty
    /// for requests without one. The status and headers are final when this
    /// returns; read the body with [`MbrResponse::next_chunk`].
    pub async fn handle(
        &self,
        method: String,
        url: String,
        headers: Vec<MobileHeader>,
        body: Vec<u8>,
    ) -> Result<Arc<MbrResponse>, MobileError> {
        let request = transport::build_request(&method, &url, &headers, body)?;
        let router = current(&self.served).router.clone();
        let home = self.home.clone();
        let runtime = self.runtime.handle().clone();
        // Spawned rather than awaited in place: the caller's executor is
        // UniFFI's (driven from Swift), not tokio's, and the router needs
        // tokio. A `JoinHandle` can be awaited from any executor.
        self.on_runtime(async move {
            let response = transport::dispatch(router, request, home.as_deref()).await;
            Arc::new(MbrResponse::from_response(response, &runtime))
        })
        .await
    }

    /// Waits for the initial indexing — scan or restore, the stat-diff after
    /// a restore, the snapshot save — and reports how it went. Requests are
    /// answered meanwhile; this is for timing and for knowing when search
    /// covers everything.
    pub async fn wait_until_ready(&self) -> Result<OpenReport, MobileError> {
        let mut ready = self.ready.clone();
        self.on_runtime(async move {
            ready
                .wait_for(Option::is_some)
                .await
                .map(|report| report.clone())
                .ok()
                .flatten()
                .ok_or_else(|| MobileError::Internal {
                    message: "indexing stopped before it finished".to_owned(),
                })
        })
        .await?
    }

    /// Re-lists the vault and applies what changed since the last look —
    /// on foreground, pull-to-refresh, or after the app wrote a file through
    /// another route — and does the same for `.mbr/` (see the
    /// [module docs](self)). Waits for the initial indexing first. Cheap: no
    /// file is read unless it changed. Saves the snapshot when something
    /// changed.
    pub async fn refresh(&self) -> Result<RefreshSummary, MobileError> {
        let lock = Arc::clone(&self.index_lock);
        let cache = self.cache.clone();
        let slot = Arc::clone(&self.served);
        let vault = Arc::clone(&self.vault);
        let root = self.root.clone();
        // A file opened in place sees nothing but itself: its folder's
        // `.mbr/` is outside the grant.
        let watch_mbr = self.home.is_none();
        self.on_runtime(async move {
            let _guard = lock.lock().await;
            let started = Instant::now();
            let served = current(&slot);
            let diff = served.server.refresh().await;
            let mut summary = served.pages.summarize(&diff, started);
            let mut save = !diff.is_empty() || !diff.evicted.is_empty();
            if watch_mbr {
                save |= refresh_mbr(&slot, &served, &vault, &root, &mut summary).await;
            }
            let served = current(&slot);
            summary.config_error = served.config_error.clone();
            if save {
                save_snapshot(&served.server, cache.as_ref()).await;
            }
            summary.duration_ms = millis(started);
            summary
        })
        .await
    }

    /// Writes the index snapshot now (for instance as the app goes to the
    /// background). A no-op without a cache directory.
    pub async fn flush_index(&self) -> Result<(), MobileError> {
        let server = Arc::clone(&current(&self.served).server);
        let lock = Arc::clone(&self.index_lock);
        let cache = self.cache.clone();
        self.on_runtime(async move {
            let _guard = lock.lock().await;
            save_snapshot(&server, cache.as_ref()).await;
        })
        .await
    }
}

impl MbrApp {
    fn start(
        root: PathBuf,
        loaded: LoadedConfig,
        vault: Arc<dyn Vault>,
        cache: Option<IndexCache>,
        snapshot: Option<mbr_core::repo::RepoSnapshot>,
        home: Option<String>,
        started: Instant,
    ) -> Result<Arc<Self>, MobileError> {
        let runtime = runtime::build().map_err(|e| open_error(format!("tokio runtime: {e}")))?;
        // What `.mbr/` looks like now, so a refresh can tell what changed. A
        // single file opened in place has no `.mbr/` in reach.
        let stamp = if home.is_none() {
            MbrStamp::take(vault.as_ref(), loaded.config.template_folder.as_deref())
        } else {
            MbrStamp::default()
        };
        let mbr_downloads_requested = stamp.request_downloads(vault.as_ref());
        // `Server::init` spawns its scan and index tasks, so it runs inside
        // the runtime's context.
        let served = {
            let _guard = runtime.enter();
            Served::build(&root, loaded, Arc::clone(&vault), snapshot, stamp)?
        };
        let server = Arc::clone(&served.server);
        let pages = served.pages.clone();
        let at_open = AtOpen {
            started,
            config_error: served.config_error.clone(),
            mbr_downloads_requested,
        };
        let (ready_tx, ready) = tokio::sync::watch::channel(None);
        let index_lock = Arc::new(tokio::sync::Mutex::new(()));
        // Taken here, not in the task, so a `refresh` that arrives before the
        // task first runs still waits for the initial indexing.
        let guard = Arc::clone(&index_lock)
            .try_lock_owned()
            .map_err(|e| open_error(format!("index lock: {e}")))?;
        runtime.spawn(initial_indexing(
            server,
            cache.clone(),
            pages,
            at_open,
            guard,
            ready_tx,
        ));
        Ok(Arc::new(Self {
            served: Arc::new(RwLock::new(Arc::new(served))),
            vault,
            runtime: BackgroundRuntime::new(runtime),
            root,
            home,
            cache,
            ready,
            index_lock,
        }))
    }

    /// Runs `future` on this vault's runtime and waits for it from whatever
    /// executor the caller is on.
    async fn on_runtime<T: Send + 'static>(
        &self,
        future: impl std::future::Future<Output = T> + Send + 'static,
    ) -> Result<T, MobileError> {
        self.runtime
            .spawn(future)
            .await
            .map_err(|e| MobileError::Internal {
                message: format!("background task: {e}"),
            })
    }
}

/// What the open itself found, for its [`OpenReport`].
struct AtOpen {
    started: Instant,
    config_error: Option<String>,
    mbr_downloads_requested: u32,
}

/// Scan or restore, the stat-diff after a restore, then the snapshot save.
async fn initial_indexing(
    server: Arc<Server>,
    cache: Option<IndexCache>,
    pages: PageUrls,
    at_open: AtOpen,
    guard: tokio::sync::OwnedMutexGuard<()>,
    ready: tokio::sync::watch::Sender<Option<OpenReport>>,
) {
    let AtOpen {
        started,
        config_error,
        mbr_downloads_requested,
    } = at_open;
    let restored = server.restored_files();
    let repo = Arc::clone(server.repo());
    repo.wait_for_scan().await;
    let scan_ms = millis(started);
    // Static files and their basic metadata come after the scan; the diff
    // and the snapshot need them.
    repo.wait_for_media().await;
    let (refresh, unchanged) = match restored {
        Some(_) => {
            let diff_started = Instant::now();
            let diff = server.refresh().await;
            let unchanged = diff.is_empty() && diff.evicted.is_empty();
            (pages.summarize(&diff, diff_started), unchanged)
        }
        None => (RefreshSummary::default(), false),
    };
    let refresh = RefreshSummary {
        config_error: config_error.clone(),
        ..refresh
    };
    // A restored index that nothing changed is the snapshot already on disk.
    if !unchanged {
        save_snapshot(&server, cache.as_ref()).await;
    }
    let report = OpenReport {
        from_cache: restored.is_some(),
        restored_files: restored.unwrap_or(0) as u64,
        indexed_files: (repo.markdown_files.len() + repo.other_files.len()) as u64,
        scan_ms,
        ready_ms: millis(started),
        refresh,
        config_error,
        mbr_downloads_requested,
    };
    tracing::info!("Vault ready: {report:?}");
    drop(guard);
    let _ = ready.send(Some(report));
}

/// The `.mbr/` half of [`MbrApp::refresh`]: stamps `.mbr/` (stats only) and
/// acts on what changed since the last look. Fills the summary's `.mbr/`
/// fields; `true` when the snapshot should be saved (a rebuild).
///
/// Which change takes which path:
///
/// - **`config.toml`**: every field. The server copies its configuration into
///   the router's state at `Server::init` and nothing there is mutable, so
///   there is no field that can be applied in place: theme, sort,
///   `index_file`, sidebar, tag sources, tasks, ignore globs alike. The vault
///   is reopened inside this `MbrApp` ([`rebuild`]), carrying the index over.
///   A `config.toml` that was only touched (same configuration once parsed)
///   changes nothing.
/// - **Templates**: reloaded in place, with the desktop watcher's own reload
///   (`Server::reload_templates`). Skipped when a rebuild already read them.
/// - **Other files** (`theme.css`, `user.css`, `components/*.js`, …):
///   nothing to do server-side. `/.mbr/*` assets are read per request with
///   `no-cache`; the summary tells the app to reload the page.
async fn refresh_mbr(
    slot: &RwLock<Arc<Served>>,
    served: &Arc<Served>,
    vault: &Arc<dyn Vault>,
    root: &Path,
    summary: &mut RefreshSummary,
) -> bool {
    let stamp_vault = Arc::clone(vault);
    let template_folder = served.config.template_folder.clone();
    let now = tokio::task::spawn_blocking(move || {
        MbrStamp::take(stamp_vault.as_ref(), template_folder.as_deref())
    })
    .await
    .unwrap_or_default();
    // Asked for again on every refresh until they are here: a provider may
    // drop a request (offline, app suspended), and asking is idempotent.
    summary.mbr_downloads_requested = now.request_downloads(vault.as_ref());
    let changes = {
        let mut seen = served
            .stamp
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let changes = now.changes_since(&seen);
        *seen = now.clone();
        changes
    };
    if !changes.any() {
        return false;
    }
    tracing::info!("{} changed: {changes:?}", root.join(".mbr").display());
    let mut rebuilt = false;
    if changes.config {
        match rebuild(root, vault, served, now).await {
            Ok(Some(next)) => {
                *slot
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(next);
                summary.config_reloaded = true;
                rebuilt = true;
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("Could not reopen with the changed configuration: {e}"),
        }
    }
    if changes.templates && !rebuilt {
        summary.templates_reloaded = served.server.reload_templates();
    }
    summary.assets_changed = changes.assets;
    rebuilt
}

/// The server for `.mbr/config.toml` as it is now, when that differs from
/// what `served` runs with; `None` when the file changed but its
/// configuration did not.
///
/// Same vault, so the same coordinator and grant; the caller keeps the same
/// cache and identity. The index is handed over as a snapshot, which the new
/// server restores without a scan unless the scan configuration (ignore
/// globs, markdown extensions, static folder, …) changed, in which case its
/// fingerprint no longer matches and it rescans, as it should. Read with the
/// open's fallback: a `config.toml` that no longer parses gives the defaults
/// plus its error, and fixing it clears the error.
async fn rebuild(
    root: &Path,
    vault: &Arc<dyn Vault>,
    served: &Arc<Served>,
    stamp: MbrStamp,
) -> Result<Option<Served>, MobileError> {
    let started = Instant::now();
    let read_root = root.to_path_buf();
    let loaded = tokio::task::spawn_blocking(move || {
        let mut loaded = LoadedConfig::read(&read_root);
        apply_mobile_overrides(&mut loaded.config, &read_root);
        loaded
    })
    .await
    .map_err(|e| MobileError::Internal {
        message: format!("config read: {e}"),
    })?;
    if same_config(&loaded.config, &served.config) && loaded.error == served.config_error {
        tracing::info!("config.toml changed on disk, but not its configuration");
        return Ok(None);
    }
    let repo = Arc::clone(served.server.repo());
    let snapshot = tokio::task::spawn_blocking(move || repo.snapshot())
        .await
        .map_err(|e| MobileError::Internal {
            message: format!("index snapshot: {e}"),
        })?;
    let next = Served::build(root, loaded, Arc::clone(vault), Some(snapshot), stamp)?;
    next.server.repo().wait_for_media().await;
    tracing::info!(
        "Reopened with the changed configuration in {}ms ({})",
        millis(started),
        match next.server.restored_files() {
            Some(files) => format!("index carried over, {files} files"),
            None => "scan configuration changed, rescanned".to_owned(),
        }
    );
    Ok(Some(next))
}

/// Whether two configurations are the same, field for field.
fn same_config(a: &Config, b: &Config) -> bool {
    match (serde_json::to_value(a), serde_json::to_value(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Writes the repository's snapshot to `cache`, off the async threads.
/// Failures are logged: a missing snapshot only costs the next open a scan.
async fn save_snapshot(server: &Arc<Server>, cache: Option<&IndexCache>) {
    let Some(cache) = cache.cloned() else {
        return;
    };
    let repo = Arc::clone(server.repo());
    let saved = tokio::task::spawn_blocking(move || {
        let snapshot = repo.snapshot();
        cache.save(&snapshot).map(|()| (snapshot.len(), cache))
    })
    .await;
    match saved {
        Ok(Ok((files, cache))) => {
            tracing::info!("Saved index of {files} files to {}", cache.path().display());
        }
        Ok(Err(e)) => tracing::warn!("Could not save the index: {e}"),
        Err(e) => tracing::warn!("Index save task failed: {e}"),
    }
}

/// What turns an index key into the URL of its page.
#[derive(Debug, Clone)]
struct PageUrls {
    /// The vault's root, which every key is under.
    root: PathBuf,
    index_file: String,
    markdown_extensions: Vec<String>,
}

impl PageUrls {
    /// A diff as the app sees it.
    fn summarize(&self, diff: &RepoDiff, started: Instant) -> RefreshSummary {
        let mut changed_pages: Vec<String> = diff
            .created
            .iter()
            .chain(&diff.modified)
            .chain(&diff.deleted)
            .filter(|key| {
                let extension = key.extension().and_then(|e| e.to_str()).unwrap_or("");
                is_markdown_extension(extension, &self.markdown_extensions)
            })
            .map(|key| build_markdown_url_path(key, &self.root, &self.index_file))
            .collect();
        changed_pages.sort();
        changed_pages.dedup();
        let count = |v: &[PathBuf]| u32::try_from(v.len()).unwrap_or(u32::MAX);
        RefreshSummary {
            added: count(&diff.created),
            changed: count(&diff.modified),
            removed: count(&diff.deleted),
            evicted: count(&diff.evicted),
            changed_pages,
            duration_ms: millis(started),
            ..RefreshSummary::default()
        }
    }
}

fn millis(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// `path`, canonical, provided it is a directory.
///
/// Canonical, as `main` does before `Config::read`: the scanner compares
/// canonical paths, and a root reached through a symlink (`/var` is one on
/// iOS, `/tmp` on macOS) otherwise yields `../../../private/...` source paths
/// for every page.
fn canonical_dir(path: &Path) -> Result<PathBuf, MobileError> {
    let root = path
        .canonicalize()
        .map_err(|e| open_error(format!("{}: {e}", path.display())))?;
    if !root.is_dir() {
        return Err(open_error(format!("not a folder: {}", root.display())));
    }
    Ok(root)
}

/// What the app needs that differs from `mbr -s`:
///
/// - **no network fetches** for bare URLs (the phone may be offline);
/// - **editing on, without a token**: the only client is the app's own web
///   view, through a scheme no other process can reach, so the repository's
///   `edit_token_hash` (a defence for network exposure) does not apply, and
///   the `Host` the page is loaded under (`localhost`) passes the edit gate;
/// - **the picked folder is the root**, with no hidden-folder exemptions.
fn apply_mobile_overrides(config: &mut Config, root: &Path) {
    config.root_dir = root.to_path_buf();
    config.explicit_hidden_dirs = Vec::new();
    config.oembed_timeout_ms = 0;
    config.edit_enabled = true;
    config.edit_token_hash = None;
    config.edit_require_token_on_loopback = false;
    config.host = IpArray([127, 0, 0, 1]);
}

#[cfg(test)]
mod tests;
