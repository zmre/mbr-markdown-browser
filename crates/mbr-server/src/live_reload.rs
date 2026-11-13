//! What the live-reload WebSocket (`/.mbr/ws/changes`) tells one page about
//! one change.
//!
//! The change channel carries every [`FileChangeEvent`] in the repository,
//! and one save produces several of them: FSEvents reports an in-place write
//! as `created` + three `modified`, an atomic save (vim, mbr's own writes) as
//! a rename plus its echoes, and the echo of a rename can arrive seconds after
//! the save. `<mbr-live-reload>` used to reload for any of them that named
//! its page, so an echo landing after the reload — on the *new* page — reloaded
//! it a second time (most visibly on a slide deck, which rebuilds itself on
//! every load).
//!
//! So each frame carries two facts only the server knows:
//!
//! - **`content_hash`** — for the page's own note only (the client names it
//!   in `?source=`, its `markdown_source`), the fingerprint of what is on disk
//!   *now*, computed exactly as the page render computes
//!   `markdown_source_hash` ([`markdown::read_source_fingerprinted`]). The
//!   page reloads only when it differs from the version it shows, so however
//!   many events one save produces, and however late they arrive, only the
//!   first one finds a difference.
//! - **`template`** — the change is under `.mbr/` or the configured template
//!   folder, i.e. it changes every page (templates, `theme.css`, components).
//!   The page used to guess this from the extension (`.html`/`.css`/`.js`
//!   anywhere in the repository), which reloaded every open page for an HTML
//!   file saved next to the notes.
//!
//! The hash is read only for an event naming the client's own source, so a
//! burst of thousands of events (a `git checkout`) costs nothing extra.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mbr_core::change_event::{ChangeEventType, FileChangeEvent};
use mbr_core::markdown;
use mbr_core::vault::Vault;
use serde::{Deserialize, Serialize};

/// The query a live-reload client opens the socket with.
#[derive(Debug, Default, Deserialize)]
pub struct LiveReloadParams {
    /// The page's own note, repo-relative and `/`-separated — the
    /// `markdown_source` the page render put in `window.frontmatter`. Absent
    /// on pages that are not a note (listings, tags, errors).
    #[serde(default)]
    pub source: Option<String>,
}

/// One frame on the wire: the event, plus what the server knows about it.
#[derive(Debug, Serialize)]
struct LiveReloadFrame<'a> {
    #[serde(flatten)]
    event: &'a FileChangeEvent,
    /// Fingerprint of the client's own note as it is on disk now. Only for an
    /// event naming that note, and only when it could be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    content_hash: Option<String>,
    /// The change is in the template folder, so every page renders
    /// differently. Omitted when false.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    template: bool,
}

/// `path` as the client spells repo-relative paths: `/`-separated, no
/// leading `/` or `./`.
fn normalized(path: &str) -> String {
    let path = path.replace('\\', "/");
    let path = path.trim_start_matches("./");
    path.trim_start_matches('/').to_owned()
}

/// Whether `event` changed the contents of the note at `source` (the client's
/// own page). A deletion has no contents to fingerprint; the client reloads
/// for it regardless.
fn is_source_change(event: &FileChangeEvent, source: Option<&str>) -> bool {
    matches!(
        event.event,
        ChangeEventType::Modified | ChangeEventType::Created
    ) && source.is_some_and(|source| {
        !source.is_empty() && normalized(source) == normalized(&event.relative_path)
    })
}

/// Whether a change at `event_path` (absolute, as the watcher reports it)
/// changes every page: anything under a `.mbr` directory, or under the
/// configured template folder (`template_folder`, canonical, as the watcher
/// reports it). Matched by path component, so a Windows `\.mbr\` matches too.
fn affects_every_page(event_path: &str, template_folder: Option<&Path>) -> bool {
    let path = Path::new(event_path);
    path.components().any(|c| c.as_os_str() == ".mbr")
        || template_folder.is_some_and(|folder| path.starts_with(folder))
}

/// What one live-reload connection needs to describe changes to its page.
pub struct LiveReloadScope {
    source: Option<String>,
    vault: Arc<dyn Vault>,
    template_folder: Option<PathBuf>,
}

impl LiveReloadScope {
    /// `template_folder` is canonicalized here, once per connection: the
    /// watcher reports canonical paths and the configured folder is stored as
    /// written.
    pub fn new(
        params: LiveReloadParams,
        vault: Arc<dyn Vault>,
        template_folder: Option<&Path>,
    ) -> Self {
        Self {
            source: params.source.filter(|source| !source.is_empty()),
            vault,
            template_folder: template_folder
                .map(|folder| folder.canonicalize().unwrap_or_else(|_| folder.to_owned())),
        }
    }

    /// The JSON frame for `event`. Reads the file only when `event` names this
    /// connection's own note; a read that fails leaves `content_hash` out, and
    /// the page then reloads as it always did.
    pub async fn frame(&self, event: &FileChangeEvent) -> serde_json::Result<String> {
        let content_hash = if is_source_change(event, self.source.as_deref()) {
            markdown::read_source_fingerprinted(Arc::clone(&self.vault), PathBuf::from(&event.path))
                .await
                .ok()
                .map(|source| source.hash)
        } else {
            None
        };
        serde_json::to_string(&LiveReloadFrame {
            event,
            content_hash,
            template: affects_every_page(&event.path, self.template_folder.as_deref()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mbr_core::vault::LocalVault;
    use tempfile::TempDir;

    fn event(root: &Path, relative: &str, kind: ChangeEventType) -> FileChangeEvent {
        FileChangeEvent {
            path: root.join(relative).to_string_lossy().into_owned(),
            relative_path: relative.to_owned(),
            event: kind,
        }
    }

    /// A canonical repository root holding `files`, and a scope for a page
    /// showing `source`.
    fn repo(files: &[(&str, &str)], source: Option<&str>) -> (TempDir, PathBuf, LiveReloadScope) {
        let dir = TempDir::new().unwrap();
        let root = dir.path().canonicalize().unwrap();
        for (path, contents) in files {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, contents).unwrap();
        }
        let scope = LiveReloadScope::new(
            LiveReloadParams {
                source: source.map(str::to_owned),
            },
            Arc::new(LocalVault::new(&root)),
            None,
        );
        (dir, root, scope)
    }

    async fn frame_json(scope: &LiveReloadScope, event: &FileChangeEvent) -> serde_json::Value {
        serde_json::from_str(&scope.frame(event).await.unwrap()).unwrap()
    }

    #[test]
    fn source_change_matches_the_page_note_only() {
        let root = Path::new("/r");
        let modified = event(root, "docs/guide.md", ChangeEventType::Modified);
        assert!(is_source_change(&modified, Some("docs/guide.md")));
        assert!(is_source_change(&modified, Some("/docs/guide.md")));
        assert!(!is_source_change(&modified, Some("docs/other.md")));
        assert!(!is_source_change(&modified, Some("")));
        assert!(!is_source_change(&modified, None));
        let windows = event(root, "docs\\guide.md", ChangeEventType::Created);
        assert!(is_source_change(&windows, Some("docs/guide.md")));
        for kind in [ChangeEventType::Deleted, ChangeEventType::Reloaded] {
            assert!(!is_source_change(
                &event(root, "docs/guide.md", kind),
                Some("docs/guide.md")
            ));
        }
    }

    #[test]
    fn template_changes_are_the_mbr_folder_and_the_template_folder() {
        assert!(affects_every_page("/r/.mbr/theme.css", None));
        assert!(affects_every_page("/r/.mbr/components/x.js", None));
        assert!(!affects_every_page("/r/site/page.html", None));
        assert!(!affects_every_page("/r/notes/a.md", None));
        // Not a substring match: a folder merely named like it is a note folder.
        assert!(!affects_every_page("/r/x.mbr/index.html", None));
        let folder = Path::new("/elsewhere/tpl");
        assert!(affects_every_page(
            "/elsewhere/tpl/index.html",
            Some(folder)
        ));
        assert!(!affects_every_page(
            "/elsewhere/tplx/index.html",
            Some(folder)
        ));
    }

    #[tokio::test]
    async fn own_note_frame_carries_the_page_render_fingerprint() {
        let text = "# Guide\n\nhello\n";
        let (_dir, root, scope) = repo(&[("docs/guide.md", text)], Some("docs/guide.md"));
        let json = frame_json(
            &scope,
            &event(&root, "docs/guide.md", ChangeEventType::Modified),
        )
        .await;
        // The same function the page render calls, so the two cannot disagree.
        let rendered = markdown::read_source_fingerprinted(
            Arc::new(LocalVault::new(&root)),
            root.join("docs/guide.md"),
        )
        .await
        .unwrap()
        .hash;
        assert_eq!(json["content_hash"], rendered);
        assert_eq!(
            json["content_hash"],
            mbr_core::edit_auth::content_hash(text.as_bytes())
        );
        assert_eq!(json["relative_path"], "docs/guide.md");
        assert_eq!(json["event"], "modified");
        assert!(json.get("template").is_none());
    }

    #[tokio::test]
    async fn one_save_reported_many_times_fingerprints_the_same_each_time() {
        // FSEvents reports one in-place write as created + modified x3, and a
        // late echo of it later still. Every one must carry the same hash, or
        // the page cannot tell an echo from a new save.
        let (_dir, root, scope) = repo(&[("a.md", "one\n")], Some("a.md"));
        let mut hashes = Vec::new();
        for kind in [
            ChangeEventType::Created,
            ChangeEventType::Modified,
            ChangeEventType::Modified,
            ChangeEventType::Modified,
        ] {
            hashes.push(
                frame_json(&scope, &event(&root, "a.md", kind)).await["content_hash"].clone(),
            );
        }
        assert!(hashes.iter().all(|h| h == &hashes[0] && h.is_string()));

        std::fs::write(root.join("a.md"), "two\n").unwrap();
        let after = frame_json(&scope, &event(&root, "a.md", ChangeEventType::Modified)).await;
        assert_ne!(
            after["content_hash"], hashes[0],
            "a real edit must change it"
        );
    }

    #[tokio::test]
    async fn other_files_are_never_read_or_fingerprinted() {
        let (_dir, root, scope) = repo(&[("a.md", "a\n"), ("b.md", "b\n")], Some("a.md"));
        let json = frame_json(&scope, &event(&root, "b.md", ChangeEventType::Modified)).await;
        assert!(json.get("content_hash").is_none());

        let (_dir, root, no_source) = repo(&[("a.md", "a\n")], None);
        let json = frame_json(&no_source, &event(&root, "a.md", ChangeEventType::Modified)).await;
        assert!(json.get("content_hash").is_none());
    }

    #[tokio::test]
    async fn unreadable_own_note_has_no_fingerprint() {
        // Deleted between the event and the read: the client reloads, as before.
        let (_dir, root, scope) = repo(&[], Some("gone.md"));
        let json = frame_json(&scope, &event(&root, "gone.md", ChangeEventType::Modified)).await;
        assert!(json.get("content_hash").is_none());
        let deleted = frame_json(&scope, &event(&root, "gone.md", ChangeEventType::Deleted)).await;
        assert_eq!(deleted["event"], "deleted");
        assert!(deleted.get("content_hash").is_none());
    }

    #[tokio::test]
    async fn template_frames_are_flagged() {
        let (_dir, root, scope) = repo(&[(".mbr/theme.css", "body{}")], Some("a.md"));
        let json = frame_json(
            &scope,
            &event(&root, ".mbr/theme.css", ChangeEventType::Modified),
        )
        .await;
        assert_eq!(json["template"], true);
    }

    #[tokio::test]
    async fn frame_never_carries_the_absolute_path() {
        // The flattened event keeps `FileChangeEvent`'s `#[serde(skip)]`.
        let (_dir, root, scope) = repo(&[("a.md", "a\n")], Some("a.md"));
        let text = scope
            .frame(&event(&root, "a.md", ChangeEventType::Modified))
            .await
            .unwrap();
        assert!(!text.contains(&*root.to_string_lossy()), "{text}");
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(json.get("path").is_none(), "{text}");
    }
}
