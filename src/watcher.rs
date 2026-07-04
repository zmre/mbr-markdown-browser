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
use std::path::Path;
use tokio::sync::broadcast;
use tracing::{debug, error, info, trace};

/// Capacity of the broadcast channel for file change events.
/// If clients don't keep up, the oldest messages will be dropped.
pub(crate) const BROADCAST_CAPACITY: usize = 100;

/// Represents a file system change event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileChangeEvent {
    /// The absolute path to the changed file.
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
    ///
    /// # Returns
    ///
    /// Returns a FileWatcher instance and a receiver for subscribing to change events.
    pub fn new(
        base_dir: &Path,
        template_folder: Option<&Path>,
        ignore_dirs: &[String],
        ignore_globs: &[String],
    ) -> Result<(Self, broadcast::Receiver<FileChangeEvent>), WatcherError> {
        let (tx, rx) = broadcast::channel(BROADCAST_CAPACITY);
        let watcher =
            Self::new_with_sender(base_dir, template_folder, ignore_dirs, ignore_globs, tx)?;
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
    /// * `sender` - An existing broadcast sender to use for file change events
    pub fn new_with_sender(
        base_dir: &Path,
        template_folder: Option<&Path>,
        ignore_dirs: &[String],
        ignore_globs: &[String],
        sender: broadcast::Sender<FileChangeEvent>,
    ) -> Result<Self, WatcherError> {
        let tx = sender;
        let base_dir = base_dir.to_path_buf();

        // Use configured ignore directories (defaults are set in Config)
        let ignore_set: HashSet<String> = ignore_dirs.iter().cloned().collect();
        // Own the ignore globs so they can move into the watcher callback.
        let ignore_globs: Vec<String> = ignore_globs.to_vec();

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
                    let under_ignored_dir = path.components().any(|comp| {
                        ignore_set.contains(comp.as_os_str().to_string_lossy().as_ref())
                    });
                    under_ignored_dir || {
                        let relative = pathdiff::diff_paths(path, &base_dir_clone)
                            .unwrap_or_else(|| path.to_path_buf());
                        should_ignore(&relative, &[], &ignore_globs)
                    }
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

    #[tokio::test]
    async fn test_watcher_creates_and_receives_events() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path();

        let (_watcher, mut rx) = FileWatcher::new(base_path, None, &[], &[]).unwrap();

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
        let (_watcher, mut rx) = FileWatcher::new(base_path, None, &ignore_dirs, &[]).unwrap();

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
    async fn test_watcher_ignores_glob_patterns() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path();

        // Ignore any *.log file via ignore_globs (matched against repo-relative path)
        let ignore_globs = vec!["*.log".to_string()];
        let (_watcher, mut rx) = FileWatcher::new(base_path, None, &[], &ignore_globs).unwrap();

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

        let (watcher, mut rx1) = FileWatcher::new(base_path, None, &[], &[]).unwrap();
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
            FileWatcher::new(base_path, Some(template_path), &[], &[]).unwrap();

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
