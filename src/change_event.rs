//! File change events: what the watcher broadcasts and what the in-process
//! writers (task toggles, edits, uploads, renames) announce.
//!
//! Kept apart from [`crate::watcher`] so the repository index, the task index
//! and the server can consume and produce events without depending on the
//! `notify` crate. Only the watcher itself needs that.

use serde::{Deserialize, Serialize};

/// Capacity of the broadcast channel that carries [`FileChangeEvent`]s.
/// If clients don't keep up, the oldest messages will be dropped.
///
/// Lives here rather than in the watcher because the server creates the
/// channel whether or not a watcher feeds it: in-process writers announce
/// their own edits on it, and a build without the `watcher` feature still
/// has live reload for those.
pub const BROADCAST_CAPACITY: usize = 100;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
