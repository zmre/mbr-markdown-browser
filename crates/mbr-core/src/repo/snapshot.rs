//! Change tracking, snapshots and the stat-diff of a [`Repo`] against its
//! vault.
//!
//! Three pieces, all opt-in and all storage-agnostic (no file I/O here):
//!
//! - **Stamps.** [`Repo::with_change_tracking`] makes the scan record a
//!   [`FileStamp`] (size, modification time, availability) for every file it
//!   indexes. Off by default: the desktop has a watcher and never pays for the
//!   table.
//! - **Diff.** [`Repo::diff_vault`] re-lists the vault, applying the scan's own
//!   ignore rules, and compares each file's stamp with the recorded one. It
//!   reads no file contents — a listing already carries every stamp — so it is
//!   the cheap "what changed while we were not looking" pass a platform without
//!   change events (iOS) runs on foreground. The result is plain
//!   [`FileChangeEvent`]s, applied through the same invalidation path the
//!   watcher's events take.
//! - **Snapshot.** [`Repo::snapshot`] captures what the scan computed that is
//!   expensive to recompute — above all the frontmatter, which costs a read of
//!   every note — and [`Repo::restore_snapshot`] puts it back without touching
//!   storage. A restored repository answers `site.json`, navigation and
//!   metadata search at once; a diff then brings it up to date.
//!
//! Where a snapshot is *kept* is the caller's business. mbr never writes one on
//! the desktop (no cache files, by policy); the iOS app keeps it in its own
//! sandboxed `Caches/` directory (`mbr_ffi::mobile`).

use std::collections::HashSet as StdHashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    MarkdownInfo, OtherFileInfo, Repo, StaticFileMetadata, build_markdown_url_path,
    is_markdown_extension, should_ignore_compiled,
};
use crate::change_event::{ChangeEventType, FileChangeEvent};
use crate::markdown::SimpleMetadata;
use crate::relationships::RawRelationship;
use crate::vault::{Availability, Entry, Vault, VaultPath};

/// The on-disk shape of a [`RepoSnapshot`]. Bump on any change to the
/// serialized types; a snapshot of another format is ignored, never migrated.
pub const SNAPSHOT_FORMAT: u32 = 1;

/// What a listing says about a file, compared across scans to decide whether
/// it changed. Nanosecond modification times, so two edits within one second
/// are still told apart (the index's own `modified` is whole seconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileStamp {
    pub size: u64,
    /// Nanoseconds since the Unix epoch (0 before it or when unknown).
    pub modified_ns: u64,
    /// The bytes were not on the device when this was recorded.
    pub not_downloaded: bool,
}

impl FileStamp {
    pub fn of(entry: &Entry) -> Self {
        let modified_ns = entry
            .modified
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
        Self {
            size: entry.size,
            modified_ns,
            not_downloaded: entry.availability == Availability::NotDownloaded,
        }
    }
}

/// What [`Repo::diff_vault`] found, as index keys.
///
/// `evicted` files were downloaded when last seen and are not any more, with
/// the same size and time: their content did not change, so the index keeps
/// what it knew (their frontmatter included) and only the stamp moves. That is
/// the distinction Logseq famously missed — an evicted file is not a deleted
/// one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoDiff {
    pub created: Vec<PathBuf>,
    pub modified: Vec<PathBuf>,
    pub deleted: Vec<PathBuf>,
    pub evicted: Vec<PathBuf>,
    /// The stamps the listing produced for every created, modified and
    /// evicted file; [`Repo::record_diff`] stores them.
    stamps: Vec<(PathBuf, FileStamp)>,
}

impl RepoDiff {
    /// Nothing to invalidate (evictions alone change no content).
    pub fn is_empty(&self) -> bool {
        self.created.is_empty() && self.modified.is_empty() && self.deleted.is_empty()
    }

    /// The created, modified and deleted files as the change events the
    /// watcher would have sent, relative paths against `root`.
    pub fn events(&self, root: &Path) -> Vec<FileChangeEvent> {
        let event = |key: &PathBuf, kind: ChangeEventType| FileChangeEvent {
            path: key.to_string_lossy().into_owned(),
            relative_path: pathdiff::diff_paths(key, root)
                .map(|p| crate::url_path::path_to_url(&p))
                .unwrap_or_default(),
            event: kind,
        };
        self.created
            .iter()
            .map(|k| event(k, ChangeEventType::Created))
            .chain(
                self.modified
                    .iter()
                    .map(|k| event(k, ChangeEventType::Modified)),
            )
            .chain(
                self.deleted
                    .iter()
                    .map(|k| event(k, ChangeEventType::Deleted)),
            )
            .collect()
    }
}

/// Why a snapshot was not restored. Never fatal: the caller scans instead.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SnapshotError {
    #[error("unreadable snapshot: {0}")]
    Corrupt(String),
    #[error("snapshot format {found}, this build reads {SNAPSHOT_FORMAT}")]
    WrongFormat { found: u32 },
    #[error("snapshot was taken by another mbr version or scan configuration")]
    WrongFingerprint,
}

/// One note as the scan left it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnapshotNote {
    /// Vault path (`/`-separated, root-relative).
    path: String,
    stamp: FileStamp,
    created: u64,
    modified: u64,
    frontmatter: Option<SimpleMetadata>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    relationships: Vec<RawRelationship>,
}

/// One non-markdown file: just enough to list it and serve its basic metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnapshotAsset {
    path: String,
    stamp: FileStamp,
    created: u64,
    modified: u64,
}

/// A serializable picture of a scanned repository. See the module docs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoSnapshot {
    format: u32,
    fingerprint: String,
    markdown: Vec<SnapshotNote>,
    other: Vec<SnapshotAsset>,
}

impl RepoSnapshot {
    /// JSON, because frontmatter is arbitrary `serde_json::Value`s, which only
    /// a self-describing format can read back.
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Parses a snapshot, refusing other formats. The fingerprint is checked
    /// against the repository by [`Repo::restore_snapshot`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SnapshotError> {
        #[derive(Deserialize)]
        struct Header {
            format: u32,
        }
        let header: Header =
            serde_json::from_slice(bytes).map_err(|e| SnapshotError::Corrupt(e.to_string()))?;
        if header.format != SNAPSHOT_FORMAT {
            return Err(SnapshotError::WrongFormat {
                found: header.format,
            });
        }
        serde_json::from_slice(bytes).map_err(|e| SnapshotError::Corrupt(e.to_string()))
    }

    /// Notes plus other files.
    pub fn len(&self) -> usize {
        self.markdown.len() + self.other.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Repo {
    /// Records a [`FileStamp`] for every file the scan indexes, which is what
    /// [`Self::diff_vault`] and [`Self::snapshot`] compare against. Call
    /// before scanning.
    #[must_use]
    pub fn with_change_tracking(mut self) -> Self {
        self.stamps = Some(std::sync::Arc::new(papaya::HashMap::new()));
        self
    }

    /// Whether [`Self::with_change_tracking`] was called.
    pub fn tracks_changes(&self) -> bool {
        self.stamps.is_some()
    }

    /// Records `entry`'s stamp under `key`, when tracking.
    pub(super) fn record_stamp(&self, key: &Path, entry: &Entry) {
        if let Some(stamps) = &self.stamps {
            stamps.pin().insert(key.to_path_buf(), FileStamp::of(entry));
        }
    }

    /// Identifies everything a snapshot's contents depend on besides the files
    /// themselves: this build (the frontmatter simplifier changes between
    /// versions) and the scan configuration (which files are notes, which are
    /// ignored, how URLs are formed).
    pub fn snapshot_fingerprint(&self) -> String {
        let inputs = serde_json::json!({
            "format": SNAPSHOT_FORMAT,
            "mbr": env!("CARGO_PKG_VERSION"),
            "markdown_extensions": self.markdown_extensions,
            "ignore_dirs": self.ignore_dirs,
            "ignore_globs": self.ignore_globs,
            "index_file": self.index_file,
            "static_folder": self.static_folder,
            "exempt_hidden_dirs": self
                .exempt_hidden_vault_paths
                .iter()
                .map(VaultPath::as_str)
                .collect::<Vec<_>>(),
        });
        let digest = Sha256::digest(inputs.to_string().as_bytes());
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The repository's current contents as a [`RepoSnapshot`]: every indexed
    /// file of the root vault that has a stamp (so only with change tracking).
    pub fn snapshot(&self) -> RepoSnapshot {
        let empty = papaya::HashMap::new();
        let stamps = self.stamps.as_deref().unwrap_or(&empty).pin();
        let vault = self.vault.as_ref();
        let in_vault = |key: &Path| {
            let path = vault.vault_path(key).ok()?;
            let stamp = *stamps.get(key)?;
            Some((path.as_str().to_owned(), stamp))
        };
        let mut markdown: Vec<SnapshotNote> = self
            .markdown_files
            .pin()
            .iter()
            .filter_map(|(key, info)| {
                let (path, stamp) = in_vault(key)?;
                Some(SnapshotNote {
                    path,
                    stamp,
                    created: info.created,
                    modified: info.modified,
                    frontmatter: info.frontmatter.clone(),
                    relationships: info.relationships.clone(),
                })
            })
            .collect();
        markdown.sort_unstable_by(|a, b| a.path.cmp(&b.path));
        let mut other: Vec<SnapshotAsset> = self
            .other_files
            .pin()
            .iter()
            .filter_map(|(key, info)| {
                let (path, stamp) = in_vault(key)?;
                Some(SnapshotAsset {
                    path,
                    stamp,
                    created: info.metadata.created.unwrap_or_default(),
                    modified: info.metadata.modified.unwrap_or_default(),
                })
            })
            .collect();
        other.sort_unstable_by(|a, b| a.path.cmp(&b.path));
        RepoSnapshot {
            format: SNAPSHOT_FORMAT,
            fingerprint: self.snapshot_fingerprint(),
            markdown,
            other,
        }
    }

    /// Fills an empty repository from `snapshot` without reading storage, and
    /// rebuilds the tag index from the restored frontmatter. Returns the
    /// number of files restored.
    ///
    /// The caller then does what it does after a scan — build the
    /// relationship and wikilink indexes, mark the scan complete — and runs a
    /// [`Self::diff_vault`] to catch what changed since the snapshot. Entries
    /// that no longer parse as vault paths are dropped (the diff re-adds
    /// them if they exist). Stamps are restored with them, so with change
    /// tracking the diff compares against the snapshot's view.
    pub fn restore_snapshot(&self, snapshot: RepoSnapshot) -> Result<usize, SnapshotError> {
        if snapshot.format != SNAPSHOT_FORMAT {
            return Err(SnapshotError::WrongFormat {
                found: snapshot.format,
            });
        }
        if snapshot.fingerprint != self.snapshot_fingerprint() {
            return Err(SnapshotError::WrongFingerprint);
        }
        let vault = self.vault.as_ref();
        let stamps = self.stamps.as_ref().map(|s| s.pin());
        let key_of = |path: &str| VaultPath::new(path).ok().map(|p| vault.key(&p));
        let markdown = self.markdown_files.pin();
        let mut restored = 0;
        for note in snapshot.markdown {
            let Some(key) = key_of(&note.path) else {
                continue;
            };
            let info = MarkdownInfo {
                raw_path: self.relative_to_root(&key),
                url_path: build_markdown_url_path(&key, &self.canonical_root, &self.index_file),
                created: note.created,
                modified: note.modified,
                frontmatter: note.frontmatter,
                relationships: note.relationships,
            };
            if let Some(stamps) = &stamps {
                stamps.insert(key.clone(), note.stamp);
            }
            markdown.insert(key, info);
            restored += 1;
        }
        drop(markdown);
        let other = self.other_files.pin();
        for asset in snapshot.other {
            let Some(key) = key_of(&asset.path) else {
                continue;
            };
            let info = OtherFileInfo {
                raw_path: key.clone(),
                url_path: self.static_url_for_key(&key),
                metadata: StaticFileMetadata::empty(&key).with_details(Some((
                    asset.stamp.size,
                    asset.created,
                    asset.modified,
                ))),
                extracted_text: None,
            };
            if let Some(stamps) = &stamps {
                stamps.insert(key.clone(), asset.stamp);
            }
            other.insert(key, info);
            restored += 1;
        }
        drop(other);
        self.rebuild_tag_index();
        Ok(restored)
    }

    /// Re-lists the vault (and an external static overlay) with the scan's
    /// ignore rules and compares every file with its recorded stamp. Reads no
    /// file contents. Blocking.
    ///
    /// Without change tracking every listed file is "created" — the diff of a
    /// repository that recorded nothing.
    pub fn diff_vault(&self) -> RepoDiff {
        let empty = papaya::HashMap::new();
        let stamps = self.stamps.as_deref().unwrap_or(&empty).pin();
        let mut diff = RepoDiff::default();
        let mut seen: StdHashSet<PathBuf> = StdHashSet::new();
        for (vault, overlay) in std::iter::once((&self.vault, false))
            .chain(self.static_vault.as_ref().map(|v| (v, true)))
        {
            for entry in self.listed_files(vault.as_ref()) {
                let key = vault.key(&entry.path);
                // Markdown in an external overlay has no URL; the scan skips it.
                if overlay
                    && is_markdown_extension(
                        entry.path.extension().unwrap_or(""),
                        &self.markdown_extensions,
                    )
                {
                    continue;
                }
                let stamp = FileStamp::of(&entry);
                match stamps.get(&key) {
                    None => diff.created.push(key.clone()),
                    Some(old) if old.size != stamp.size || old.modified_ns != stamp.modified_ns => {
                        diff.modified.push(key.clone());
                    }
                    // Downloaded since: the index may lack its frontmatter.
                    Some(old) if old.not_downloaded && !stamp.not_downloaded => {
                        diff.modified.push(key.clone());
                    }
                    Some(old) if !old.not_downloaded && stamp.not_downloaded => {
                        diff.evicted.push(key.clone());
                    }
                    Some(_) => {
                        seen.insert(key);
                        continue;
                    }
                }
                diff.stamps.push((key.clone(), stamp));
                seen.insert(key);
            }
        }
        diff.deleted = stamps
            .iter()
            .map(|(key, _)| key)
            .filter(|key| !seen.contains(*key))
            .cloned()
            .collect();
        diff.created.sort_unstable();
        diff.modified.sort_unstable();
        diff.deleted.sort_unstable();
        diff.evicted.sort_unstable();
        diff
    }

    /// Stores the stamps of a diff once its events have been applied, so the
    /// same changes are not reported again. A no-op without change tracking.
    pub fn record_diff(&self, diff: &RepoDiff) {
        let Some(stamps) = &self.stamps else {
            return;
        };
        let stamps = stamps.pin();
        for (key, stamp) in &diff.stamps {
            stamps.insert(key.clone(), *stamp);
        }
        for key in &diff.deleted {
            stamps.remove(key);
        }
    }

    /// Every file of `vault` the scan would index, by the scan's own rules:
    /// hidden entries, `ignore_dirs` and `ignore_globs` are skipped, judged on
    /// the same keys the scan builds.
    fn listed_files(&self, vault: &dyn Vault) -> Vec<Entry> {
        let ignored = |entry: &Entry| {
            should_ignore_compiled(
                &vault.key(&entry.path),
                entry.is_dir(),
                &self.ignore_dirs,
                &self.compiled_ignore_globs,
                &self.exempt_hidden_dirs,
            )
        };
        crate::vault::walk_files(vault, |dir| !ignored(dir))
            .into_iter()
            .filter(|entry| !ignored(entry))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::MemVault;
    use std::sync::Arc;
    use std::time::Duration;

    fn repo(vault: Arc<MemVault>) -> Repo {
        Repo::init_with_vault(
            vault,
            "static",
            &["md".to_string()],
            &["node_modules".to_string()],
            &[],
            "index.md",
            &[crate::config::TagSource {
                field: "tags".to_string(),
                label: None,
                label_plural: None,
            }],
            &[],
        )
        .with_change_tracking()
    }

    fn vault() -> Arc<MemVault> {
        Arc::new(
            MemVault::new()
                .with_file("a.md", "---\ntitle: Alpha\ntags: [x]\n---\n# A\n")
                .with_file("docs/b.md", "---\ntitle: Beta\n---\nSee [[Alpha]].\n")
                .with_file("docs/index.md", "# Docs\n")
                .with_file("static/img.png", "png")
                .with_file("node_modules/skip.md", "# no")
                .with_file(".hidden.md", "# no"),
        )
    }

    fn scanned(vault: Arc<MemVault>) -> Repo {
        let repo = repo(vault);
        repo.scan_all().unwrap();
        repo.scan_static_folder().unwrap();
        repo.populate_basic_metadata();
        repo
    }

    /// Root-relative and `/`-separated on every platform, as the vault paths
    /// the assertions spell them.
    fn keys(diff: &[PathBuf], root: &Path) -> Vec<String> {
        diff.iter()
            .map(|k| crate::url_path::path_to_url(k.strip_prefix(root).unwrap()))
            .collect()
    }

    #[test]
    fn a_fresh_scan_diffs_clean() {
        let vault = vault();
        let repo = scanned(Arc::clone(&vault));
        let diff = repo.diff_vault();
        assert_eq!(diff, RepoDiff::default(), "{diff:?}");
        assert!(diff.is_empty());
    }

    #[test]
    fn restore_reproduces_site_json_byte_for_byte() {
        let vault = vault();
        let original = scanned(Arc::clone(&vault));
        let bytes = original.snapshot().to_bytes();
        let restored = repo(Arc::clone(&vault));
        let count = restored
            .restore_snapshot(RepoSnapshot::from_bytes(&bytes).unwrap())
            .unwrap();
        assert_eq!(count, 4, "3 notes + 1 asset");
        assert_eq!(restored.to_json().unwrap(), original.to_json().unwrap());
        assert_eq!(
            restored.tag_index.get_pages("tags", "x").len(),
            original.tag_index.get_pages("tags", "x").len()
        );
        assert!(restored.diff_vault().is_empty());
    }

    #[test]
    fn diff_reports_created_modified_deleted_and_records_them() {
        let vault = vault();
        let repo = scanned(Arc::clone(&vault));
        let root = vault.root().to_path_buf();
        vault.insert_file("new.md", "# New");
        vault.insert_file("a.md", "---\ntitle: Alpha 2\n---\n");
        vault.set_modified(
            "a.md",
            SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000),
        );
        vault
            .remove_file(&VaultPath::new("docs/b.md").unwrap())
            .unwrap();
        let diff = repo.diff_vault();
        assert_eq!(keys(&diff.created, &root), ["new.md"]);
        assert_eq!(keys(&diff.modified, &root), ["a.md"]);
        assert_eq!(keys(&diff.deleted, &root), ["docs/b.md"]);
        let events = diff.events(&root);
        assert_eq!(events.len(), 3);
        assert!(
            events
                .iter()
                .any(|e| e.relative_path == "docs/b.md" && e.event == ChangeEventType::Deleted)
        );
        for event in &events {
            repo.invalidate_file(Path::new(&event.path), &event.event);
        }
        repo.record_diff(&diff);
        assert!(
            repo.diff_vault().is_empty(),
            "applied changes are not re-reported"
        );
        assert!(repo.markdown_files.pin().contains_key(&root.join("new.md")));
        assert!(
            !repo
                .markdown_files
                .pin()
                .contains_key(&root.join("docs/b.md"))
        );
    }

    #[test]
    fn eviction_keeps_frontmatter_and_download_triggers_a_reread() {
        let vault = vault();
        let repo = scanned(Arc::clone(&vault));
        let root = vault.root().to_path_buf();
        vault.set_availability("a.md", Availability::NotDownloaded);
        let diff = repo.diff_vault();
        assert!(diff.is_empty(), "evictions change no content");
        assert_eq!(keys(&diff.evicted, &root), ["a.md"]);
        repo.record_diff(&diff);
        // Still titled from the frontmatter read while it was local.
        let snapshot_json = String::from_utf8(repo.snapshot().to_bytes()).unwrap();
        assert!(snapshot_json.contains("Alpha"));
        assert!(snapshot_json.contains("\"not_downloaded\":true"));

        vault.set_availability("a.md", Availability::Local);
        let diff = repo.diff_vault();
        assert_eq!(keys(&diff.modified, &root), ["a.md"], "downloaded again");
    }

    #[test]
    fn a_not_downloaded_note_scans_with_its_stamp_and_rereads_once_local() {
        let vault = vault();
        vault.set_availability("docs/b.md", Availability::NotDownloaded);
        let repo = scanned(Arc::clone(&vault));
        let root = vault.root().to_path_buf();
        let info = repo
            .markdown_files
            .pin()
            .get(&root.join("docs/b.md"))
            .cloned()
            .expect("listed although not downloaded");
        assert!(info.frontmatter.is_none());
        assert!(repo.diff_vault().is_empty());
        vault.set_availability("docs/b.md", Availability::Local);
        let diff = repo.diff_vault();
        assert_eq!(keys(&diff.modified, &root), ["docs/b.md"]);
        for event in diff.events(&root) {
            repo.invalidate_file(Path::new(&event.path), &event.event);
        }
        let info = repo
            .markdown_files
            .pin()
            .get(&root.join("docs/b.md"))
            .cloned()
            .unwrap();
        assert!(info.frontmatter.is_some());
    }

    #[test]
    fn snapshots_of_another_format_version_or_config_are_refused() {
        let vault = vault();
        let original = scanned(Arc::clone(&vault));
        let mut snapshot = original.snapshot();
        assert!(matches!(
            RepoSnapshot::from_bytes(b"not json"),
            Err(SnapshotError::Corrupt(_))
        ));
        assert!(matches!(
            RepoSnapshot::from_bytes(br#"{"format":999}"#),
            Err(SnapshotError::WrongFormat { found: 999 })
        ));
        assert!(matches!(
            RepoSnapshot::from_bytes(br#"{"format":1,"markdown":7}"#),
            Err(SnapshotError::Corrupt(_))
        ));
        // A different scan configuration.
        let other_config = Repo::init_with_vault(
            Arc::clone(&vault) as Arc<dyn Vault>,
            "static",
            &["md".to_string(), "markdown".to_string()],
            &[],
            &[],
            "index.md",
            &[],
            &[],
        );
        assert_eq!(
            other_config.restore_snapshot(snapshot.clone()),
            Err(SnapshotError::WrongFingerprint)
        );
        snapshot.fingerprint = "tampered".into();
        assert_eq!(
            repo(vault).restore_snapshot(snapshot),
            Err(SnapshotError::WrongFingerprint)
        );
    }

    #[test]
    fn applied_changes_restamp_without_a_diff() {
        // The server's own write reaches the index as a Modified event; the
        // next diff must not report it again.
        let vault = vault();
        let repo = scanned(Arc::clone(&vault));
        let root = vault.root().to_path_buf();
        vault.insert_file("a.md", "---\ntitle: Written by mbr\n---\n");
        vault.set_modified(
            "a.md",
            SystemTime::UNIX_EPOCH + Duration::from_secs(2_100_000_000),
        );
        repo.invalidate_file(&root.join("a.md"), &ChangeEventType::Modified);
        assert!(repo.diff_vault().is_empty());
        // And a delete drops the stamp, so it is not "deleted" again either.
        vault.remove_file(&VaultPath::new("a.md").unwrap()).unwrap();
        repo.invalidate_file(&root.join("a.md"), &ChangeEventType::Deleted);
        assert!(repo.diff_vault().is_empty());
        // Ignored or hidden paths never gain one.
        vault.insert_file(".hidden2.md", "x");
        repo.invalidate_file(&root.join(".hidden2.md"), &ChangeEventType::Created);
        assert!(repo.diff_vault().is_empty());
    }

    #[test]
    fn untracked_repositories_record_nothing() {
        let vault = vault();
        let untracked = Repo::init_with_vault(
            vault as Arc<dyn Vault>,
            "static",
            &["md".to_string()],
            &[],
            &[],
            "index.md",
            &[],
            &[],
        );
        untracked.scan_all().unwrap();
        assert!(!untracked.tracks_changes());
        assert!(untracked.snapshot().is_empty());
        assert!(!untracked.diff_vault().created.is_empty());
    }
}
