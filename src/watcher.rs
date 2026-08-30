//! File system watcher for live reload functionality.
//!
//! This module provides a file watcher that monitors the entire repository directory
//! for changes and broadcasts change events via a tokio broadcast channel.
//!
//! Uses RecommendedWatcher (FSEvents on macOS) for kernel-level efficiency —
//! no per-file stat polling, handles large directories without CPU overhead.

use crate::errors::WatcherError;
use crate::repo::should_ignore;
use notify::{Event, EventKind, RecursiveMode, Watcher as NotifyWatcher};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tokio::sync::broadcast;
use tracing::{debug, error, info, trace};

/// Capacity of the broadcast channel for file change events.
/// If clients don't keep up, the oldest messages will be dropped.
pub(crate) const BROADCAST_CAPACITY: usize = 100;

/// Represents a file system change event.
///
/// Only `relative_path` and `event` cross the wire: this struct is broadcast to
/// every live-reload WebSocket client, and an absolute path would leak the OS
/// username, the home directory layout, and private note filenames.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileChangeEvent {
    /// The absolute path to the changed file.
    ///
    /// **Server-side only** (`#[serde(skip)]`): consumed in-process by the
    /// template hot-reload and repo-invalidation tasks. It is never serialized
    /// to WebSocket clients — see the struct docs.
    #[serde(skip)]
    pub path: String,
    /// The path relative to the repository root.
    pub relative_path: String,
    /// The type of change event.
    pub event: ChangeEventType,
}

/// Type of file system change event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ChangeEventType {
    Modified,
    Created,
    Deleted,
}

/// File watcher that monitors the repository for changes.
pub struct FileWatcher {
    _watcher: notify::RecommendedWatcher,
    pub sender: broadcast::Sender<FileChangeEvent>,
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

                        // Process each path in the event
                        for path in event.paths {
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
            _watcher: watcher,
            sender: tx,
        })
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

    /// Drain events from the receiver until one matches the predicate, or timeout.
    ///
    /// Filesystem watchers can emit spurious events (directory metadata, temp files)
    /// so tests must not assume the *first* event is the one they care about.
    async fn recv_matching(
        rx: &mut broadcast::Receiver<FileChangeEvent>,
        predicate: impl Fn(&FileChangeEvent) -> bool,
    ) -> Option<FileChangeEvent> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(WATCH_TIMEOUT_SECS);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(event)) if predicate(&event) => return Some(event),
                Ok(Ok(_)) => continue, // spurious event, keep draining
                Ok(Err(_)) => return None,
                Err(_) => return None, // timed out
            }
        }
        None
    }

    #[test]
    fn test_serialized_event_omits_absolute_path() {
        // The live-reload WebSocket broadcasts this struct to any client that
        // completes a handshake, so the absolute path must never be on the
        // wire: it leaks the OS username, home layout, and private filenames.
        let event = FileChangeEvent {
            path: "/Users/someone/private notes/secret.md".to_string(),
            relative_path: "private notes/secret.md".to_string(),
            event: ChangeEventType::Modified,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(
            !json.contains("/Users/someone"),
            "absolute path leaked into the broadcast payload: {json}"
        );
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(
            parsed.get("path").is_none(),
            "`path` must not be serialized: {json}"
        );
        assert_eq!(parsed["relative_path"], "private notes/secret.md");
        assert_eq!(parsed["event"], "modified");
    }

    #[tokio::test]
    async fn test_watcher_creates_and_receives_events() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path();

        let (_watcher, mut rx) = FileWatcher::new(base_path, None, &[], &[], &[]).unwrap();

        // Create a test file
        let test_file = base_path.join("test.md");
        fs::write(&test_file, "# Test").unwrap();

        // Wait for an event matching our file (skip spurious events)
        let change = recv_matching(&mut rx, |e| e.relative_path.contains("test.md")).await;
        assert!(
            change.is_some(),
            "Should receive file change event for test.md"
        );
        assert_eq!(change.unwrap().event, ChangeEventType::Created);
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
        let change = recv_matching(&mut rx, |e| e.relative_path.contains("visible.md")).await;
        assert!(change.is_some(), "Should receive event for visible.md");

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

        let change = recv_matching(&mut rx, |e| e.relative_path.contains("test.md")).await;
        assert!(
            change.is_some(),
            "Should receive event for test.md even though an ancestor of the root is named 'build'"
        );
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
        let change = recv_matching(&mut rx, |e| e.relative_path.contains("note.md")).await;
        assert!(change.is_some(), "Should receive event for note.md");

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

        // Create a file in the template folder (not base dir)
        let template_file = template_path.join("custom.css");
        fs::write(&template_file, "/* custom css */").unwrap();

        // Wait for an event matching our file (skip spurious events)
        let change = recv_matching(&mut rx, |e| e.path.contains("custom.css")).await;
        assert!(
            change.is_some(),
            "Should receive file change event for custom.css from template folder"
        );
        assert_eq!(change.unwrap().event, ChangeEventType::Created);
    }
}
