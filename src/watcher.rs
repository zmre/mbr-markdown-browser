//! File system watcher for live reload functionality.
//!
//! This module provides a file watcher that monitors the entire repository directory
//! for changes and broadcasts change events via a tokio broadcast channel.
//!
//! Uses PollWatcher for reliability on macOS (kqueue has issues with NonRecursive mode).

use crate::errors::WatcherError;
use notify::{Config, Event, EventKind, PollWatcher, RecursiveMode, Watcher as NotifyWatcher};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{debug, error, info, trace};

/// Capacity of the broadcast channel for file change events.
/// If clients don't keep up, the oldest messages will be dropped.
const BROADCAST_CAPACITY: usize = 100;

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
    _watcher: PollWatcher,
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
        _ignore_globs: &[String],
        sender: broadcast::Sender<FileChangeEvent>,
    ) -> Result<Self, WatcherError> {
        let tx = sender;
        let base_dir = base_dir.to_path_buf();

        // Use configured ignore directories (defaults are set in Config)
        let ignore_set: HashSet<String> = ignore_dirs.iter().cloned().collect();

        let tx_clone = tx.clone();
        let base_dir_clone = base_dir.clone();

        // Configure poll watcher with 1 second interval for responsive live reload
        let poll_config = Config::default().with_poll_interval(Duration::from_secs(1));

        // Create PollWatcher - more reliable on macOS than kqueue-based watcher
        let mut watcher = PollWatcher::new(
            move |res: Result<Event, notify::Error>| {
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
                            // Skip if path contains any ignored directory
                            let path_str = path.to_string_lossy();
                            let should_ignore = ignore_set.iter().any(|ignored| {
                                path.components().any(|comp| {
                                    comp.as_os_str().to_string_lossy() == ignored.as_str()
                                })
                            });

                            if should_ignore {
                                debug!("Ignoring change in: {}", path_str);
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
                            // Skip if path contains any ignored directory
                            let path_str = path.to_string_lossy();
                            let should_ignore = ignore_set.iter().any(|ignored| {
                                path.components().any(|comp| {
                                    comp.as_os_str().to_string_lossy() == ignored.as_str()
                                })
                            });

                            if should_ignore {
                                trace!("Ignoring error in: {}", path_str);
                            } else {
                                error!("File watcher error: {}", e);
                            }
                        }
                    }
                }
            },
            poll_config,
        )
        .map_err(WatcherError::WatcherInit)?;

        // Watch the entire directory recursively
        // PollWatcher handles this reliably unlike kqueue on macOS
        // Events from ignored directories are filtered in the callback
        watcher
            .watch(base_dir.as_ref(), RecursiveMode::Recursive)
            .map_err(|e| WatcherError::WatchFailed {
                path: base_dir.clone(),
                source: e,
            })?;

        info!("File watcher started for {:?} (polling every 1s)", base_dir);

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
    use tempfile::TempDir;

    // PollWatcher uses 1 second intervals, so tests need longer timeouts
    const POLL_TIMEOUT_SECS: u64 = 3;

    #[tokio::test]
    async fn test_watcher_creates_and_receives_events() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path();

        let (_watcher, mut rx) = FileWatcher::new(base_path, None, &[], &[]).unwrap();

        // Create a test file
        let test_file = base_path.join("test.md");
        fs::write(&test_file, "# Test").unwrap();

        // Wait for the event (PollWatcher checks every 1 second)
        let event = tokio::time::timeout(Duration::from_secs(POLL_TIMEOUT_SECS), rx.recv()).await;

        assert!(event.is_ok(), "Should receive file change event");
        let change = event.unwrap().unwrap();
        assert_eq!(change.event, ChangeEventType::Created);
        assert!(change.relative_path.contains("test.md"));
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

        // Wait for the event (PollWatcher checks every 1 second)
        let event = tokio::time::timeout(Duration::from_secs(POLL_TIMEOUT_SECS), rx.recv()).await;
        assert!(event.is_ok(), "Should receive event for visible.md");
        let change = event.unwrap().unwrap();
        assert!(
            change.relative_path.contains("visible.md"),
            "Event should be for visible.md, got: {}",
            change.relative_path
        );

        // Now create an ignored directory and file
        let target_dir = base_path.join("target");
        fs::create_dir(&target_dir).unwrap();

        // Create file in ignored directory
        let ignored_file = target_dir.join("ignored.txt");
        fs::write(&ignored_file, "ignored content").unwrap();

        // Wait for one polling cycle and check that we didn't receive the ignored file
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
    async fn test_multiple_subscribers() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path();

        let (watcher, mut rx1) = FileWatcher::new(base_path, None, &[], &[]).unwrap();
        let mut rx2 = watcher.subscribe();

        // Create a test file
        let test_file = base_path.join("multi.md");
        fs::write(&test_file, "# Multi").unwrap();

        // Both receivers should get the event (PollWatcher checks every 1 second)
        let event1 = tokio::time::timeout(Duration::from_secs(POLL_TIMEOUT_SECS), rx1.recv()).await;
        let event2 = tokio::time::timeout(Duration::from_secs(POLL_TIMEOUT_SECS), rx2.recv()).await;

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

        // Should receive the event from template folder
        let event = tokio::time::timeout(Duration::from_secs(POLL_TIMEOUT_SECS), rx.recv()).await;

        assert!(
            event.is_ok(),
            "Should receive file change event from template folder"
        );
        let change = event.unwrap().unwrap();
        assert_eq!(change.event, ChangeEventType::Created);
        assert!(
            change.path.contains("custom.css"),
            "Event should be for custom.css, got: {}",
            change.path
        );
    }
}
