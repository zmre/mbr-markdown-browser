//! [`MemVault`]: a vault held entirely in memory.
//!
//! For tests above all — a repository built from a handful of
//! `with_file(path, contents)` calls instead of a temp directory — and, because
//! it has **no** [`Vault::local_path`], the proof that a consumer works without
//! a real filesystem: every fast path that needs one has to fall back to
//! reading through the vault here. It also models the two things a cloud-backed
//! vault adds: files that are listed but not downloaded
//! ([`MemVault::set_availability`]) and download requests
//! ([`MemVault::download_requests`]).
//!
//! Always compiled (not test-only) so the integration tests of the other
//! crates can build one; it is small, and unreferenced code is dropped at link
//! time.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::RwLock;

use super::{Availability, Entry, EntryKind, Vault, VaultError, VaultPath};

#[derive(Debug, Clone)]
struct MemFile {
    bytes: Arc<[u8]>,
    modified: SystemTime,
    created: SystemTime,
    availability: Availability,
}

#[derive(Debug, Default)]
struct MemState {
    files: BTreeMap<VaultPath, MemFile>,
    /// Every directory, including the root and every ancestor of every file.
    dirs: BTreeSet<VaultPath>,
    downloads: Vec<VaultPath>,
}

impl MemState {
    fn add_ancestors(&mut self, path: &VaultPath) {
        let mut parent = path.parent();
        while let Some(dir) = parent {
            parent = dir.parent();
            if !self.dirs.insert(dir) {
                break;
            }
        }
    }
}

/// An in-memory vault. Interior mutability, so it can be shared as
/// `Arc<dyn Vault>` and still written to.
#[derive(Debug)]
pub struct MemVault {
    root: PathBuf,
    state: RwLock<MemState>,
}

impl Default for MemVault {
    fn default() -> Self {
        Self::new()
    }
}

/// Parses a test-supplied path, panicking on a bad one — these are fixture
/// literals, so a bad one is a bug in the test.
fn fixture_path(path: &str) -> VaultPath {
    VaultPath::new(path).unwrap_or_else(|e| panic!("invalid MemVault path {path:?}: {e}"))
}

impl MemVault {
    /// The default namespace root. Deliberately a path no real repository
    /// would sit at, so a key that is accidentally opened as a file fails
    /// loudly instead of reading something.
    pub const DEFAULT_ROOT: &'static str = "/mbr-mem-vault";

    /// An empty vault (just the root directory).
    pub fn new() -> Self {
        Self::with_root(Self::DEFAULT_ROOT)
    }

    /// An empty vault whose [`Vault::root`] is `root`.
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        let state = MemState {
            dirs: BTreeSet::from([VaultPath::root()]),
            ..MemState::default()
        };
        Self {
            root: root.into(),
            state: RwLock::new(state),
        }
    }

    /// Builder form of [`Self::insert_file`].
    #[must_use]
    pub fn with_file(self, path: &str, contents: impl AsRef<[u8]>) -> Self {
        self.insert_file(path, contents);
        self
    }

    /// Builder form of [`Self::insert_dir`].
    #[must_use]
    pub fn with_dir(self, path: &str) -> Self {
        self.insert_dir(path);
        self
    }

    /// Creates or replaces a file (and its ancestor directories).
    ///
    /// # Panics
    ///
    /// When `path` is not a valid [`VaultPath`].
    pub fn insert_file(&self, path: &str, contents: impl AsRef<[u8]>) {
        let path = fixture_path(path);
        let now = SystemTime::now();
        let mut state = self.state.write();
        state.add_ancestors(&path);
        let created = state.files.get(&path).map_or(now, |f| f.created);
        state.files.insert(
            path,
            MemFile {
                bytes: Arc::from(contents.as_ref()),
                modified: now,
                created,
                availability: Availability::Local,
            },
        );
    }

    /// Creates a directory (and its ancestors).
    ///
    /// # Panics
    ///
    /// When `path` is not a valid [`VaultPath`].
    pub fn insert_dir(&self, path: &str) {
        let path = fixture_path(path);
        let mut state = self.state.write();
        state.add_ancestors(&path);
        state.dirs.insert(path);
    }

    /// Marks a file as listed-but-not-downloaded (or back).
    ///
    /// # Panics
    ///
    /// When there is no such file.
    pub fn set_availability(&self, path: &str, availability: Availability) {
        let path = fixture_path(path);
        let mut state = self.state.write();
        let file = state
            .files
            .get_mut(&path)
            .unwrap_or_else(|| panic!("no MemVault file {path}"));
        file.availability = availability;
    }

    /// Sets a file's modification time.
    ///
    /// # Panics
    ///
    /// When there is no such file.
    pub fn set_modified(&self, path: &str, modified: SystemTime) {
        let path = fixture_path(path);
        let mut state = self.state.write();
        let file = state
            .files
            .get_mut(&path)
            .unwrap_or_else(|| panic!("no MemVault file {path}"));
        file.modified = modified;
    }

    /// A file's current bytes, for assertions.
    pub fn contents(&self, path: &str) -> Option<Vec<u8>> {
        let path = VaultPath::new(path).ok()?;
        self.state.read().files.get(&path).map(|f| f.bytes.to_vec())
    }

    /// Every path passed to [`Vault::request_download`] so far, in order.
    pub fn download_requests(&self) -> Vec<VaultPath> {
        self.state.read().downloads.clone()
    }

    fn file_entry(path: &VaultPath, file: &MemFile) -> Entry {
        Entry {
            path: path.clone(),
            kind: EntryKind::File,
            size: file.bytes.len() as u64,
            modified: file.modified,
            created: Some(file.created),
            availability: file.availability,
        }
    }

    fn dir_entry(path: &VaultPath) -> Entry {
        Entry {
            path: path.clone(),
            kind: EntryKind::Dir,
            size: 0,
            modified: SystemTime::UNIX_EPOCH,
            created: None,
            availability: Availability::Local,
        }
    }

    fn readable(&self, path: &VaultPath) -> Result<Arc<[u8]>, VaultError> {
        let state = self.state.read();
        match state.files.get(path) {
            Some(file) if file.availability == Availability::NotDownloaded => {
                Err(VaultError::NotDownloaded {
                    path: path.to_string(),
                })
            }
            Some(file) => Ok(Arc::clone(&file.bytes)),
            None if state.dirs.contains(path) => Err(VaultError::Io {
                path: path.to_string(),
                source: io::Error::new(io::ErrorKind::IsADirectory, "is a directory"),
            }),
            None => Err(VaultError::not_found(path)),
        }
    }
}

impl Vault for MemVault {
    fn root(&self) -> &Path {
        &self.root
    }

    fn stat(&self, path: &VaultPath) -> Result<Option<Entry>, VaultError> {
        let state = self.state.read();
        if let Some(file) = state.files.get(path) {
            return Ok(Some(Self::file_entry(path, file)));
        }
        Ok(state.dirs.contains(path).then(|| Self::dir_entry(path)))
    }

    fn list_dir(&self, dir: &VaultPath) -> Result<Vec<Entry>, VaultError> {
        let state = self.state.read();
        if !state.dirs.contains(dir) {
            return Err(if state.files.contains_key(dir) {
                VaultError::Io {
                    path: dir.to_string(),
                    source: io::Error::new(io::ErrorKind::NotADirectory, "not a directory"),
                }
            } else {
                VaultError::not_found(dir)
            });
        }
        let is_child = |p: &VaultPath| p.parent().as_ref() == Some(dir);
        let dirs = state
            .dirs
            .iter()
            .filter(|p| !p.is_root() && is_child(p))
            .map(Self::dir_entry);
        let files = state
            .files
            .iter()
            .filter(|(p, _)| is_child(p))
            .map(|(p, f)| Self::file_entry(p, f));
        Ok(dirs.chain(files).collect())
    }

    fn canonicalize(&self, path: &VaultPath) -> Result<VaultPath, VaultError> {
        let state = self.state.read();
        if state.files.contains_key(path) || state.dirs.contains(path) {
            Ok(path.clone())
        } else {
            Err(VaultError::not_found(path))
        }
    }

    fn read(&self, path: &VaultPath) -> Result<Vec<u8>, VaultError> {
        self.readable(path).map(|bytes| bytes.to_vec())
    }

    fn read_prefix(&self, path: &VaultPath, max_len: usize) -> Result<Vec<u8>, VaultError> {
        self.readable(path)
            .map(|bytes| bytes[..bytes.len().min(max_len)].to_vec())
    }

    fn write_atomic(&self, path: &VaultPath, bytes: &[u8]) -> Result<(), VaultError> {
        let mut state = self.state.write();
        let parent = path.parent().ok_or_else(|| VaultError::Io {
            path: path.to_string(),
            source: io::Error::new(io::ErrorKind::IsADirectory, "is a directory"),
        })?;
        if !state.dirs.contains(&parent) {
            return Err(VaultError::not_found(&parent));
        }
        if state.dirs.contains(path) {
            return Err(VaultError::Io {
                path: path.to_string(),
                source: io::Error::new(io::ErrorKind::IsADirectory, "is a directory"),
            });
        }
        let now = SystemTime::now();
        let created = state.files.get(path).map_or(now, |f| f.created);
        state.files.insert(
            path.clone(),
            MemFile {
                bytes: Arc::from(bytes),
                modified: now,
                created,
                availability: Availability::Local,
            },
        );
        Ok(())
    }

    fn create_dir_all(&self, path: &VaultPath) -> Result<(), VaultError> {
        let mut state = self.state.write();
        if let Some(blocker) = path.ancestors().find(|a| state.files.contains_key(a)) {
            return Err(VaultError::Io {
                path: blocker.to_string(),
                source: io::Error::new(io::ErrorKind::NotADirectory, "not a directory"),
            });
        }
        state.add_ancestors(path);
        state.dirs.insert(path.clone());
        Ok(())
    }

    fn remove_file(&self, path: &VaultPath) -> Result<(), VaultError> {
        self.state
            .write()
            .files
            .remove(path)
            .map(|_| ())
            .ok_or_else(|| VaultError::not_found(path))
    }

    fn rename(&self, from: &VaultPath, to: &VaultPath) -> Result<(), VaultError> {
        let mut state = self.state.write();
        let parent = to.parent().ok_or_else(|| VaultError::not_found(to))?;
        if !state.dirs.contains(&parent) {
            return Err(VaultError::not_found(&parent));
        }
        let file = state
            .files
            .remove(from)
            .ok_or_else(|| VaultError::not_found(from))?;
        state.files.insert(to.clone(), file);
        Ok(())
    }

    fn request_download(&self, paths: &[VaultPath]) {
        let mut state = self.state.write();
        for path in paths {
            if let Some(file) = state.files.get_mut(path) {
                file.availability = Availability::Local;
            }
            state.downloads.push(path.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp(s: &str) -> VaultPath {
        VaultPath::new(s).unwrap()
    }

    #[test]
    fn files_imply_their_directories() {
        let vault = MemVault::new().with_file("a/b/c.md", "# C");
        assert!(vault.is_dir(&vp("a")));
        assert!(vault.is_dir(&vp("a/b")));
        assert!(vault.is_file(&vp("a/b/c.md")));
        assert!(vault.is_dir(&VaultPath::root()));
        assert_eq!(vault.stat(&vp("a/x")).unwrap(), None);
    }

    #[test]
    fn list_dir_lists_immediate_children_only() {
        let vault = MemVault::new()
            .with_file("a/b/c.md", "c")
            .with_file("a/d.md", "d")
            .with_dir("a/empty");
        let mut names: Vec<String> = vault
            .list_dir(&vp("a"))
            .unwrap()
            .into_iter()
            .map(|e| e.path.to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a/b", "a/d.md", "a/empty"]);
        let top: Vec<String> = vault
            .list_dir(&VaultPath::root())
            .unwrap()
            .into_iter()
            .map(|e| e.path.to_string())
            .collect();
        assert_eq!(top, vec!["a"]);
        assert!(vault.list_dir(&vp("missing")).unwrap_err().is_not_found());
        assert!(vault.list_dir(&vp("a/d.md")).is_err());
    }

    #[test]
    fn reads_prefixes_and_utf8() {
        let vault = MemVault::new().with_file("x.md", "hello");
        assert_eq!(vault.read(&vp("x.md")).unwrap(), b"hello");
        assert_eq!(vault.read_prefix(&vp("x.md"), 2).unwrap(), b"he");
        assert_eq!(vault.read_prefix(&vp("x.md"), 99).unwrap(), b"hello");
        assert_eq!(vault.read_to_string(&vp("x.md")).unwrap(), "hello");
        assert!(vault.read(&vp("y.md")).unwrap_err().is_not_found());
        assert_eq!(vault.local_path(&vp("x.md")), None);
    }

    #[test]
    fn not_downloaded_files_are_listed_but_unreadable_until_requested() {
        let vault = MemVault::new().with_file("cloud.md", "# Cloud");
        vault.set_availability("cloud.md", Availability::NotDownloaded);
        let entry = vault.stat(&vp("cloud.md")).unwrap().unwrap();
        assert_eq!(entry.availability, Availability::NotDownloaded);
        assert_eq!(entry.size, 7);
        assert!(matches!(
            vault.read(&vp("cloud.md")),
            Err(VaultError::NotDownloaded { .. })
        ));
        vault.request_download(&[vp("cloud.md")]);
        assert_eq!(vault.download_requests(), vec![vp("cloud.md")]);
        assert_eq!(vault.read(&vp("cloud.md")).unwrap(), b"# Cloud");
    }

    #[test]
    fn write_atomic_semantics_match_the_local_vault() {
        let vault = MemVault::new().with_file("docs/a.md", "old");
        vault.write_atomic(&vp("docs/a.md"), b"new").unwrap();
        assert_eq!(vault.contents("docs/a.md").unwrap(), b"new");
        vault.write_atomic(&vp("docs/b.md"), b"b").unwrap();
        assert!(vault.is_file(&vp("docs/b.md")));
        // Parent must exist, as for `atomic_write`.
        assert!(
            vault
                .write_atomic(&vp("nope/x.md"), b"x")
                .unwrap_err()
                .is_not_found()
        );
        // Cannot replace a directory.
        assert!(vault.write_atomic(&vp("docs"), b"x").is_err());
        vault.create_dir_all(&vp("nope/deeper")).unwrap();
        vault.write_atomic(&vp("nope/deeper/x.md"), b"x").unwrap();
        // A file in the way of a directory.
        assert!(vault.create_dir_all(&vp("docs/a.md/sub")).is_err());
    }

    #[test]
    fn rename_and_remove() {
        let vault = MemVault::new().with_file("a.md", "a").with_dir("d");
        vault.rename(&vp("a.md"), &vp("d/a.md")).unwrap();
        assert!(vault.is_file(&vp("d/a.md")));
        assert!(!vault.is_file(&vp("a.md")));
        assert!(vault.rename(&vp("a.md"), &vp("d/b.md")).is_err());
        assert!(vault.rename(&vp("d/a.md"), &vp("nope/a.md")).is_err());
        vault.remove_file(&vp("d/a.md")).unwrap();
        assert!(vault.remove_file(&vp("d/a.md")).is_err());
    }

    #[test]
    fn canonicalize_is_identity_for_existing_paths() {
        let vault = MemVault::new().with_file("a/b.md", "b");
        assert_eq!(vault.canonicalize(&vp("a/b.md")).unwrap(), vp("a/b.md"));
        assert_eq!(vault.canonicalize(&vp("a")).unwrap(), vp("a"));
        assert!(
            vault
                .canonicalize(&vp("a/c.md"))
                .unwrap_err()
                .is_not_found()
        );
    }

    #[test]
    fn keys_live_under_the_virtual_root() {
        let vault = MemVault::new();
        let key = vault.key(&vp("a/b.md"));
        assert!(key.starts_with(MemVault::DEFAULT_ROOT));
        assert_eq!(vault.vault_path(&key).unwrap(), vp("a/b.md"));
    }
}
