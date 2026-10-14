//! File system watcher for live reload functionality.
//!
//! This module provides a file watcher that monitors the entire repository directory
//! for changes and broadcasts change events via a tokio broadcast channel.
//!
//! Uses RecommendedWatcher (FSEvents on macOS) for kernel-level efficiency —
//! no per-file stat polling, handles large directories without CPU overhead.

use mbr_core::change_event::{BROADCAST_CAPACITY, ChangeEventType, FileChangeEvent};
use mbr_core::repo::should_ignore;
use notify::{Event, EventKind, RecursiveMode, Watcher as NotifyWatcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use thiserror::Error;
use tokio::sync::broadcast;
use tracing::{debug, error, info, trace};

/// Errors related to file watching.
///
/// Defined here rather than in [`mbr_core::errors`] because its sources are
/// `notify` errors, and the shared error module must not depend on `notify`.
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

/// The most symlink mounts the watcher follows.
///
/// Each mount is one more recursive watch: one FSEvents stream path on macOS
/// (cheap), but one inotify watch per *directory* on Linux, counted against
/// `fs.inotify.max_user_watches`, and one file descriptor per file under
/// kqueue (BSD). A mount to a huge tree is the user's choice and is served
/// regardless; past this many mounts, further ones are served without live
/// reload rather than risk exhausting the host's watches for the root itself.
pub const MAX_WATCHED_MOUNTS: usize = 32;

/// One watched mount: events under `target` are reported at `location`.
#[derive(Debug, Clone)]
struct WatchedMount {
    /// The mount's canonical target, as notify reports its events.
    target: PathBuf,
    /// The index key of the link — where the scanner indexed the target's
    /// files (`<root>/static/videos`).
    location: PathBuf,
}

/// File watcher that monitors the repository for changes.
pub struct FileWatcher {
    watcher: Mutex<notify::RecommendedWatcher>,
    /// Watched symlink mounts, shared with the event callback, which reports a
    /// change in a mount's target at the link's location instead.
    mounts: Arc<RwLock<Vec<WatchedMount>>>,
    pub sender: broadcast::Sender<FileChangeEvent>,
}

/// `path` as the index knows it: an event under a watched mount's target is
/// moved to the link's location (the longest target wins, so a nested mount
/// beats an enclosing one). Paths elsewhere are returned unchanged.
fn translate_mount_path(path: PathBuf, mounts: &[WatchedMount]) -> PathBuf {
    mounts
        .iter()
        .filter(|mount| path.starts_with(&mount.target))
        .max_by_key(|mount| mount.target.components().count())
        .and_then(|mount| {
            path.strip_prefix(&mount.target)
                .ok()
                .map(|below| mount.location.join(below))
        })
        .unwrap_or(path)
}

impl FileWatcher {
    /// Creates a new file watcher for the given base directory.
    ///
    /// # Arguments
    ///
    /// * `base_dir` - The root directory to watch
    /// * `template_folder` - Optional template folder to also watch for hot reload
    /// * `ignore_dirs` - Directory names to ignore (e.g., "target", ".git")
    /// * `ignore_globs` - Glob patterns to ignore (e.g., "*.log")
    /// * `explicit_hidden_dirs` - Repo-relative hidden directories the user named
    ///   on the command line (`Config::explicit_hidden_dirs`), which must keep
    ///   emitting events despite the leading-dot rule
    ///
    /// # Returns
    ///
    /// Returns a FileWatcher instance and a receiver for subscribing to change events.
    pub fn new(
        base_dir: &Path,
        template_folder: Option<&Path>,
        ignore_dirs: &[String],
        ignore_globs: &[String],
        explicit_hidden_dirs: &[PathBuf],
    ) -> Result<(Self, broadcast::Receiver<FileChangeEvent>), WatcherError> {
        let (tx, rx) = broadcast::channel(BROADCAST_CAPACITY);
        let watcher = Self::new_with_sender(
            base_dir,
            template_folder,
            ignore_dirs,
            ignore_globs,
            explicit_hidden_dirs,
            tx,
        )?;
        Ok((watcher, rx))
    }

    /// Creates a new file watcher using an existing broadcast sender.
    ///
    /// This variant is useful when you want to create the broadcast channel ahead of time
    /// (e.g., to avoid blocking during watcher initialization).
    ///
    /// # Arguments
    ///
    /// * `base_dir` - The root directory to watch
    /// * `template_folder` - Optional template folder to also watch for hot reload
    /// * `ignore_dirs` - Directory names to ignore (e.g., "target", ".git")
    /// * `ignore_globs` - Glob patterns to ignore (e.g., "*.log")
    /// * `explicit_hidden_dirs` - Repo-relative hidden directories the user named
    ///   on the command line (`Config::explicit_hidden_dirs`)
    /// * `sender` - An existing broadcast sender to use for file change events
    pub fn new_with_sender(
        base_dir: &Path,
        template_folder: Option<&Path>,
        ignore_dirs: &[String],
        ignore_globs: &[String],
        explicit_hidden_dirs: &[PathBuf],
        sender: broadcast::Sender<FileChangeEvent>,
    ) -> Result<Self, WatcherError> {
        let tx = sender;
        // notify reports canonical paths — FSEvents and inotify both hand back
        // the resolved inode's path — so the diff base has to be canonical too
        // or every `diff_paths` below climbs *out* of the repo instead of
        // staying inside it. With the root configured as `/tmp/notes`, an event
        // for `/private/tmp/notes/x.md` diffs to `../../private/tmp/notes/x.md`,
        // which breaks two things at once. The ignore checks would see the
        // ancestors of the root again — the very components the repo-relative
        // matching in the callback exists to exclude — and `relative_path` would
        // carry the absolute path in disguise to every live-reload client,
        // leaking exactly the username, home layout, and private filenames the
        // struct docs promise it never does. Canonicalize once, here, so the
        // watched path and the diff base cannot drift apart. Falling back to the
        // path as given covers a root that does not exist yet; the `watch()`
        // call below then surfaces that as a proper `WatchFailed`.
        let base_dir = base_dir
            .canonicalize()
            .unwrap_or_else(|_| base_dir.to_path_buf());

        // Use configured ignore directories (defaults are set in Config)
        let ignore_set: HashSet<String> = ignore_dirs.iter().cloned().collect();
        // Own the ignore globs so they can move into the watcher callback.
        let ignore_globs: Vec<String> = ignore_globs.to_vec();
        // Already repo-relative, which is the base the callback compares in, so
        // this is an owning copy and nothing more. Kept in that base rather than
        // joined onto `base_dir`: the callback deliberately relativizes before
        // testing anything (see `is_ignored`), and rebasing here would make the
        // exemption the one check in it that reasons about absolute paths.
        let exempt_hidden_dirs: Vec<PathBuf> = explicit_hidden_dirs.to_vec();

        let tx_clone = tx.clone();
        let base_dir_clone = base_dir.clone();
        let mounts: Arc<RwLock<Vec<WatchedMount>>> = Arc::new(RwLock::new(Vec::new()));
        let mounts_for_callback = Arc::clone(&mounts);

        // Create RecommendedWatcher (FSEvents on macOS, inotify on Linux)
        // Kernel-level: no polling, no CPU overhead for large directories
        let mut watcher = notify::RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                // A path is ignored when it lives under a configured ignore
                // directory or when its repo-relative form matches an ignore
                // glob. Reuses `repo::should_ignore` for glob matching so the
                // watcher and the repo scanner stay consistent.
                let is_ignored = |path: &Path| -> bool {
                    // Both checks run against the repo-relative path. Matching
                    // ignore-dir names against the absolute path would also match
                    // components *above* the root, so a repo that merely happens to
                    // live under a directory named `build`/`target`/`.git` would
                    // discard every event and silently disable live reload — exactly
                    // what the Nix Linux sandbox does with its `TMPDIR=/build`.
                    let relative = pathdiff::diff_paths(path, &base_dir_clone)
                        .unwrap_or_else(|| path.to_path_buf());
                    let under_ignored_dir = relative.components().any(|comp| {
                        ignore_set.contains(comp.as_os_str().to_string_lossy().as_ref())
                    });
                    under_ignored_dir
                        || should_ignore(&relative, &[], &ignore_globs, &exempt_hidden_dirs)
                };

                match res {
                    Ok(event) => {
                        debug!("File watcher event: {:?}", event);

                        // Determine event type
                        let event_type = match event.kind {
                            EventKind::Create(_) => ChangeEventType::Created,
                            EventKind::Modify(_) => ChangeEventType::Modified,
                            EventKind::Remove(_) => ChangeEventType::Deleted,
                            _ => {
                                debug!("Ignoring event kind: {:?}", event.kind);
                                return;
                            }
                        };

                        let watched = mounts_for_callback
                            .read()
                            .unwrap_or_else(PoisonError::into_inner)
                            .clone();
                        // Process each path in the event
                        for path in event.paths {
                            let path = translate_mount_path(path, &watched);
                            // Skip ignored directories and ignore-glob matches
                            if is_ignored(&path) {
                                debug!("Ignoring change in: {}", path.to_string_lossy());
                                continue;
                            }

                            // Calculate relative path
                            let relative_path = pathdiff::diff_paths(&path, &base_dir_clone)
                                .unwrap_or_else(|| path.clone());

                            let change_event = FileChangeEvent {
                                path: path.to_string_lossy().to_string(),
                                relative_path: relative_path.to_string_lossy().to_string(),
                                event: event_type.clone(),
                            };

                            debug!("Broadcasting file change: {:?}", change_event);

                            // Broadcast the event (don't care if no receivers)
                            let _ = tx_clone.send(change_event);
                        }
                    }
                    Err(e) => {
                        // Process each path in the event
                        for path in &e.paths {
                            // Skip ignored directories and ignore-glob matches
                            if is_ignored(path) {
                                trace!("Ignoring error in: {}", path.to_string_lossy());
                            } else {
                                error!("File watcher error: {}", e);
                            }
                        }
                    }
                }
            },
            notify::Config::default(),
        )
        .map_err(WatcherError::WatcherInit)?;

        // Watch the entire directory recursively
        // FSEvents handles this efficiently at the kernel level
        // Events from ignored directories are filtered in the callback
        watcher
            .watch(base_dir.as_ref(), RecursiveMode::Recursive)
            .map_err(|e| WatcherError::WatchFailed {
                path: base_dir.clone(),
                source: e,
            })?;

        info!("File watcher started for {:?} (FSEvents/inotify)", base_dir);

        // Also watch template_folder if provided (for dev mode hot reload of templates/assets)
        if let Some(template_path) = template_folder {
            watcher
                .watch(template_path, RecursiveMode::Recursive)
                .map_err(|e| WatcherError::WatchFailed {
                    path: template_path.to_path_buf(),
                    source: e,
                })?;
            info!(
                "File watcher also watching template folder {:?}",
                template_path
            );
        }

        Ok(FileWatcher {
            watcher: Mutex::new(watcher),
            mounts,
            sender: tx,
        })
    }

    /// Also watches every directory mount in `mounts` (as
    /// [`mbr_core::repo::Repo::mounts`] lists them: the link's index key and
    /// the mount) that is not watched yet, so a change inside an external
    /// folder reloads like a change in the repository. Idempotent; call it
    /// whenever the repository may have discovered new mounts (after a scan).
    ///
    /// File mounts are not watched — a watch on a single file outside any
    /// watched directory is not portable across notify's backends — and at
    /// most [`MAX_WATCHED_MOUNTS`] mounts are. A mount that cannot be watched
    /// is still served; only live reload is missing for it.
    pub fn watch_mounts(&self, mounts: &[(PathBuf, mbr_core::vault::Mount)]) {
        let mut watcher = self.watcher.lock().unwrap_or_else(PoisonError::into_inner);
        for (location, mount) in mounts.iter().filter(|(_, mount)| mount.is_dir) {
            // The mapping goes in *before* the watch and comes out again if
            // the watch fails: an event arriving in between must already be
            // translated, or it would be broadcast with the target's absolute
            // path. The lock is never held across `watch`, which on some
            // backends waits for the thread that runs the callback.
            {
                let mut watched = self.mounts.write().unwrap_or_else(PoisonError::into_inner);
                if watched.iter().any(|w| w.target == mount.target) {
                    continue;
                }
                if watched.len() >= MAX_WATCHED_MOUNTS {
                    tracing::warn!(
                        "Not watching {} for changes: already watching {MAX_WATCHED_MOUNTS} external folders",
                        mount.target.display()
                    );
                    continue;
                }
                watched.push(WatchedMount {
                    target: mount.target.clone(),
                    location: location.clone(),
                });
            }
            match watcher.watch(&mount.target, RecursiveMode::Recursive) {
                Ok(()) => info!(
                    "File watcher also watching external folder {:?} (via {:?})",
                    mount.target, location
                ),
                Err(e) => {
                    tracing::warn!(
                        "Not watching external folder {} for changes: {e}",
                        mount.target.display()
                    );
                    self.mounts
                        .write()
                        .unwrap_or_else(PoisonError::into_inner)
                        .retain(|w| w.target != mount.target);
                }
            }
        }
    }

    /// Subscribes to file change events.
    ///
    /// Returns a new receiver that will receive all future change events.
    pub fn subscribe(&self) -> broadcast::Receiver<FileChangeEvent> {
        self.sender.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;

    // RecommendedWatcher delivers events faster than PollWatcher, but allow headroom
    const WATCH_TIMEOUT_SECS: u64 = 5;

    /// Drain events from the receiver until one matches the predicate.
    ///
    /// Filesystem watchers can emit spurious events (directory metadata, temp files)
    /// so tests must not assume the *first* event is the one they care about.
    ///
    /// On failure the events that *did* arrive come back with the error: a bare
    /// "no event" is indistinguishable from "the wrong event", and the two have
    /// different causes.
    async fn recv_matching(
        rx: &mut broadcast::Receiver<FileChangeEvent>,
        predicate: impl Fn(&FileChangeEvent) -> bool,
    ) -> Result<FileChangeEvent, Vec<FileChangeEvent>> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(WATCH_TIMEOUT_SECS);
        let mut seen = Vec::new();
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(event)) if predicate(&event) => return Ok(event),
                Ok(Ok(event)) => seen.push(event),
                // A burst that overruns the channel drops the oldest events; the
                // one being waited on may still be behind it.
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => return Err(seen),
                Err(_) => return Err(seen),
            }
        }
    }

    /// Blocks until the watcher is demonstrably delivering events for `dir`.
    ///
    /// FSEvents resolves `kFSEventStreamEventIdSinceNow` in `fseventsd`, not in
    /// `FSEventStreamStart`, so a write issued the instant `watch()` returns can
    /// land ahead of the stream's start point. The create is then never reported
    /// and the write arrives alone as `Modified`, which is why asserting the kind
    /// of the first event for a freshly written file fails under load. One
    /// throwaway file establishes the boundary: once its event has been observed,
    /// every later change has a higher event id.
    async fn wait_until_live(dir: &Path, rx: &mut broadcast::Receiver<FileChangeEvent>) {
        let probe = dir.join("watcher-readiness-probe");
        fs::write(&probe, "probe").unwrap();
        recv_matching(rx, |e| e.path.ends_with("watcher-readiness-probe"))
            .await
            .expect("watcher delivered no event for its readiness probe");
        fs::remove_file(&probe).unwrap();
    }
    #[tokio::test]
    async fn test_watcher_creates_and_receives_events() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path();

        let (_watcher, mut rx) = FileWatcher::new(base_path, None, &[], &[], &[]).unwrap();

        wait_until_live(base_path, &mut rx).await;

        // Create a test file
        let test_file = base_path.join("test.md");
        fs::write(&test_file, "# Test").unwrap();

        // Wait for an event matching our file (skip spurious events)
        let change = recv_matching(&mut rx, |e| e.relative_path.contains("test.md"))
            .await
            .expect("should receive file change event for test.md");
        assert_eq!(change.event, ChangeEventType::Created);
    }

    #[tokio::test]
    async fn test_watcher_ignores_configured_directories() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path();

        // Create watcher with target in ignore list
        let ignore_dirs = vec!["target".to_string()];
        let (_watcher, mut rx) = FileWatcher::new(base_path, None, &ignore_dirs, &[], &[]).unwrap();

        // Create a file in the base directory - this should be visible
        let visible_file = base_path.join("visible.md");
        fs::write(&visible_file, "visible content").unwrap();

        // Wait for an event matching our file (skip spurious events)
        recv_matching(&mut rx, |e| e.relative_path.contains("visible.md"))
            .await
            .expect("should receive event for visible.md");

        // Now create an ignored directory and file
        let target_dir = base_path.join("target");
        fs::create_dir(&target_dir).unwrap();

        // Create file in ignored directory
        let ignored_file = target_dir.join("ignored.txt");
        fs::write(&ignored_file, "ignored content").unwrap();

        // Wait and check that we didn't receive the ignored file
        let mut saw_ignored_file = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);

        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
                Ok(Ok(change)) => {
                    if change.relative_path.contains("ignored.txt") {
                        saw_ignored_file = true;
                    }
                }
                Ok(Err(_)) => break,
                Err(_) => continue,
            }
        }

        assert!(
            !saw_ignored_file,
            "Should NOT see ignored.txt from target/ directory"
        );
    }

    #[tokio::test]
    async fn test_watcher_ignores_only_dirs_inside_the_repo() {
        // Regression: ignore-dir names describe directories *inside* the repo, so
        // they must be matched against the repo-relative path. Matching the
        // absolute path meant an ancestor named `build` killed every event for the
        // whole repo — which is what the Nix sandbox (`TMPDIR=/build`) hits.
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path().join("build").join("notes");
        fs::create_dir_all(&base_path).unwrap();
        // notify reports canonical paths, so the base dir has to be canonical too
        // or `diff_paths` yields a `../..`-prefixed path (macOS temp dirs live
        // under the /var -> /private/var symlink). `TestRepo` canonicalizes for
        // the same reason.
        let base_path = base_path.canonicalize().unwrap();

        let ignore_dirs = vec!["build".to_string()];
        let (_watcher, mut rx) =
            FileWatcher::new(&base_path, None, &ignore_dirs, &[], &[]).unwrap();

        let test_file = base_path.join("test.md");
        fs::write(&test_file, "# Test").unwrap();

        recv_matching(&mut rx, |e| e.relative_path.contains("test.md"))
            .await
            .expect("should receive event for test.md even though an ancestor of the root is named 'build'");
    }

    // Symlink creation on Windows needs Developer Mode or elevation, so this is
    // unix-only like the symlink walk helper in repo.rs.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_watcher_relative_paths_stay_inside_a_symlinked_root() {
        // Regression: notify reports canonical paths, so a root configured
        // through a symlink (macOS `/tmp` -> `/private/tmp`, or any symlinked
        // checkout) made `diff_paths` climb out of the repo and produce
        // `../../private/tmp/...`. `relative_path` is broadcast to every
        // live-reload client, so such a value hands out the absolute path in
        // disguise — the username, home layout, and private note filenames the
        // struct docs say must never cross the wire.
        let temp_dir = TempDir::new().unwrap();
        let real_root = temp_dir.path().join("real");
        let real_notes = real_root.join("notes");
        fs::create_dir_all(&real_notes).unwrap();
        let link_root = temp_dir.path().join("link");
        std::os::unix::fs::symlink(&real_root, &link_root).unwrap();

        // Watch through the symlink: the path an operator configured, not the
        // canonical one notify will report events for.
        let link_notes = link_root.join("notes");
        let (_watcher, mut rx) = FileWatcher::new(&link_notes, None, &[], &[], &[]).unwrap();

        fs::write(link_notes.join("test.md"), "# Test").unwrap();

        // Match on the suffix so a climbing path still satisfies the predicate —
        // the assertions below, not the filter, are what fail before the fix.
        let change = recv_matching(&mut rx, |e| e.relative_path.ends_with("test.md"))
            .await
            .expect("should receive event for test.md through a symlinked root");
        assert!(
            !change.relative_path.contains(".."),
            "relative_path escaped the repo root and leaks absolute path segments: {}",
            change.relative_path
        );
        assert_eq!(change.relative_path, "test.md");

        // And the absolute path is an index key of the vault over the same
        // root: both canonicalize it, so the server's `Repo::invalidate_file`
        // finds the entry the scan made instead of keying a second one.
        use mbr_core::vault::Vault;
        let vault = mbr_core::vault::LocalVault::new(&link_notes);
        assert_eq!(
            vault
                .vault_path(Path::new(&change.path))
                .expect("event path is inside the vault")
                .as_str(),
            "test.md"
        );
    }

    #[tokio::test]
    async fn test_watcher_ignores_glob_patterns() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path();

        // Ignore any *.log file via ignore_globs (matched against repo-relative path)
        let ignore_globs = vec!["*.log".to_string()];
        let (_watcher, mut rx) =
            FileWatcher::new(base_path, None, &[], &ignore_globs, &[]).unwrap();

        // A normal markdown file must still fire an event...
        let note_file = base_path.join("note.md");
        fs::write(&note_file, "# Note").unwrap();
        // ...while a file matching the ignore glob must not.
        let log_file = base_path.join("debug.log");
        fs::write(&log_file, "log line").unwrap();

        // The normal path invokes the reload callback (broadcasts an event).
        recv_matching(&mut rx, |e| e.relative_path.contains("note.md"))
            .await
            .expect("should receive event for note.md");

        // The ignored glob path must never invoke the reload callback.
        let mut saw_log = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
                Ok(Ok(change)) => {
                    if change.relative_path.contains("debug.log") {
                        saw_log = true;
                    }
                }
                Ok(Err(_)) => break,
                Err(_) => continue,
            }
        }

        assert!(
            !saw_log,
            "Should NOT see debug.log (matches *.log ignore glob)"
        );
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path();

        let (watcher, mut rx1) = FileWatcher::new(base_path, None, &[], &[], &[]).unwrap();
        let mut rx2 = watcher.subscribe();

        // Create a test file
        let test_file = base_path.join("multi.md");
        fs::write(&test_file, "# Multi").unwrap();

        // Both receivers should get the event
        let event1 =
            tokio::time::timeout(Duration::from_secs(WATCH_TIMEOUT_SECS), rx1.recv()).await;
        let event2 =
            tokio::time::timeout(Duration::from_secs(WATCH_TIMEOUT_SECS), rx2.recv()).await;

        assert!(event1.is_ok());
        assert!(event2.is_ok());

        let change1 = event1.unwrap().unwrap();
        let change2 = event2.unwrap().unwrap();

        assert_eq!(change1, change2);
    }

    #[tokio::test]
    async fn test_watcher_watches_template_folder() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path();

        // Create a separate template folder
        let template_dir = TempDir::new().unwrap();
        let template_path = template_dir.path();

        let (_watcher, mut rx) =
            FileWatcher::new(base_path, Some(template_path), &[], &[], &[]).unwrap();

        wait_until_live(template_path, &mut rx).await;

        // Create a file in the template folder (not base dir)
        let template_file = template_path.join("custom.css");
        fs::write(&template_file, "/* custom css */").unwrap();

        // Wait for an event matching our file (skip spurious events)
        let change = recv_matching(&mut rx, |e| e.path.contains("custom.css"))
            .await
            .expect("should receive file change event for custom.css from template folder");
        assert_eq!(change.event, ChangeEventType::Created);
    }
}
