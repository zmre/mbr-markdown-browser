//! [`LocalVault`]: a vault that is a directory on the local filesystem.
//!
//! This is the desktop's storage, and it is meant to behave exactly as the
//! `std::fs` code it replaced: symlinks are followed on every read and listing,
//! containment is decided by canonicalizing ([`Vault::canonicalize`]), and
//! writes are temp-file-plus-rename ([`atomic_write`]).

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use super::{Availability, Entry, EntryKind, Vault, VaultError, VaultPath};

/// A repository directory on the local filesystem.
#[derive(Debug, Clone)]
pub struct LocalVault {
    /// Canonicalized at construction — see [`LocalVault::new`].
    root: PathBuf,
}

impl LocalVault {
    /// A vault rooted at `root`, **canonicalized** when it exists.
    ///
    /// Canonical because every path this vault produces is relativized against
    /// it: `canonicalize` on a child returns a canonical path, and on macOS a
    /// temp dir (`/var/…`) or on Windows any path (`\\?\C:\…`) differs from its
    /// canonical spelling, which would make every child look like it is outside
    /// the root. Falls back to the path as given when it cannot be
    /// canonicalized (it does not exist yet); every operation then fails with
    /// the real error rather than here.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            root: root.canonicalize().unwrap_or(root),
        }
    }

    /// The native path for `path`.
    pub fn native(&self, path: &VaultPath) -> PathBuf {
        path.to_native(&self.root)
    }

    fn entry(path: VaultPath, metadata: &fs::Metadata) -> Entry {
        Entry {
            path,
            kind: if metadata.is_dir() {
                EntryKind::Dir
            } else {
                EntryKind::File
            },
            size: metadata.len(),
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            created: metadata.created().ok(),
            availability: Availability::Local,
        }
    }
}

impl Vault for LocalVault {
    fn root(&self) -> &Path {
        &self.root
    }

    fn stat(&self, path: &VaultPath) -> Result<Option<Entry>, VaultError> {
        match fs::metadata(self.native(path)) {
            Ok(metadata) => Ok(Some(Self::entry(path.clone(), &metadata))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(VaultError::from_io(path, e)),
        }
    }

    fn list_dir(&self, dir: &VaultPath) -> Result<Vec<Entry>, VaultError> {
        let native = self.native(dir);
        let entries = fs::read_dir(&native).map_err(|e| VaultError::from_io(dir, e))?;
        Ok(entries
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    tracing::debug!("Skipping non-UTF-8 name in {}", native.display());
                    return None;
                };
                let path = match dir.child(name) {
                    Ok(path) => path,
                    Err(e) => {
                        tracing::debug!("Skipping {:?} in {}: {e}", name, native.display());
                        return None;
                    }
                };
                // `fs::metadata` follows symlinks, as `WalkDir::follow_links`
                // did. A broken link fails here and is skipped, as `WalkDir`'s
                // error entries were.
                let metadata = fs::metadata(entry.path()).ok()?;
                Some(Self::entry(path, &metadata))
            })
            .collect())
    }

    fn canonicalize(&self, path: &VaultPath) -> Result<VaultPath, VaultError> {
        let canonical = self
            .native(path)
            .canonicalize()
            .map_err(|e| VaultError::from_io(path, e))?;
        VaultPath::from_absolute(&self.root, &canonical).map_err(|_| VaultError::OutsideRoot {
            path: canonical.display().to_string(),
        })
    }

    fn read(&self, path: &VaultPath) -> Result<Vec<u8>, VaultError> {
        fs::read(self.native(path)).map_err(|e| VaultError::from_io(path, e))
    }

    fn read_prefix(&self, path: &VaultPath, max_len: usize) -> Result<Vec<u8>, VaultError> {
        read_prefix_native(&self.native(path), max_len).map_err(|e| VaultError::from_io(path, e))
    }

    fn write_atomic(&self, path: &VaultPath, bytes: &[u8]) -> Result<(), VaultError> {
        atomic_write(&self.native(path), bytes).map_err(|e| VaultError::from_io(path, e))
    }

    fn create_dir_all(&self, path: &VaultPath) -> Result<(), VaultError> {
        fs::create_dir_all(self.native(path)).map_err(|e| VaultError::from_io(path, e))
    }

    fn remove_file(&self, path: &VaultPath) -> Result<(), VaultError> {
        fs::remove_file(self.native(path)).map_err(|e| VaultError::from_io(path, e))
    }

    fn rename(&self, from: &VaultPath, to: &VaultPath) -> Result<(), VaultError> {
        fs::rename(self.native(from), self.native(to)).map_err(|e| VaultError::from_io(from, e))
    }

    fn local_path(&self, path: &VaultPath) -> Option<PathBuf> {
        Some(self.native(path))
    }
}

/// At most the first `max_len` bytes of the file at `path`.
///
/// Sized from the open handle's metadata, so a short file costs one exact read
/// and no second path lookup — the frontmatter scan does this for every
/// markdown file in the repository.
pub fn read_prefix_native(path: &Path, max_len: usize) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let len = file.metadata().map(|m| m.len() as usize).unwrap_or(0);
    let mut buffer = vec![0u8; len.min(max_len)];
    file.read_exact(&mut buffer)?;
    Ok(buffer)
}

/// Creates a fresh temp file beside a write target, for a write-then-rename.
///
/// The name is hidden (leading dot, so the scanner and watcher skip it),
/// derived from the target's, and unique per process and call — `create_new`
/// guarantees no two writers ever share one, which a fixed `.{name}.mbr-tmp`
/// did not: concurrent writers truncated each other's temp file and the loser's
/// rename failed with `ENOENT`.
pub fn create_unique_temp_file(dir: &Path, file_name: &str) -> io::Result<(PathBuf, File)> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    // A leftover from a crashed process can only collide on a reused pid;
    // move past it rather than fail.
    const ATTEMPTS: u32 = 16;
    let pid = std::process::id();
    let mut last_error = None;
    for _ in 0..ATTEMPTS {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!(".{file_name}.{pid}-{n}.mbr-tmp"));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last_error = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::other("no free temp file name")))
}

/// Atomically writes `bytes` to `path` (temp file in the same dir + rename).
///
/// The temp file has a name of its own (see [`create_unique_temp_file`]), and
/// when `path` already exists it takes over that file's permissions — a rename
/// replaces the inode, and a note that was `0600` must not come back `0644`.
/// The temp file is removed on any failure. Blocking.
///
/// Does not lock: callers that read before writing hold a per-file lock across
/// both (`mbr_server::file_write::FileWriteLocks`).
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file.md");
    let (tmp, mut file) = create_unique_temp_file(parent, file_name)?;
    let written = file
        .write_all(bytes)
        .and_then(|()| match fs::metadata(path) {
            Ok(existing) => file.set_permissions(existing.permissions()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        })
        .and_then(|()| {
            drop(file);
            fs::rename(&tmp, path)
        });
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp(s: &str) -> VaultPath {
        VaultPath::new(s).unwrap()
    }

    fn fixture() -> (tempfile::TempDir, LocalVault) {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("docs/sub")).unwrap();
        fs::write(dir.path().join("docs/a.md"), "# A\n").unwrap();
        fs::write(dir.path().join("docs/sub/b.md"), "# B\n").unwrap();
        fs::write(dir.path().join("top.txt"), "top").unwrap();
        let vault = LocalVault::new(dir.path());
        (dir, vault)
    }

    #[test]
    fn root_is_canonical() {
        let (dir, vault) = fixture();
        assert_eq!(vault.root(), dir.path().canonicalize().unwrap());
    }

    #[test]
    fn stat_reports_kind_size_and_absence() {
        let (_dir, vault) = fixture();
        let a = vault.stat(&vp("docs/a.md")).unwrap().unwrap();
        assert!(a.is_file());
        assert_eq!(a.size, 4);
        assert_eq!(a.availability, Availability::Local);
        assert!(vault.stat(&vp("docs")).unwrap().unwrap().is_dir());
        assert!(vault.stat(&VaultPath::root()).unwrap().unwrap().is_dir());
        assert_eq!(vault.stat(&vp("missing.md")).unwrap(), None);
        assert!(vault.is_file(&vp("top.txt")));
        assert!(vault.is_dir(&vp("docs/sub")));
        assert!(!vault.is_dir(&vp("top.txt")));
    }

    #[test]
    fn list_dir_returns_children_with_their_kinds() {
        let (_dir, vault) = fixture();
        let mut names: Vec<(String, EntryKind)> = vault
            .list_dir(&vp("docs"))
            .unwrap()
            .into_iter()
            .map(|e| (e.path.to_string(), e.kind))
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                ("docs/a.md".to_string(), EntryKind::File),
                ("docs/sub".to_string(), EntryKind::Dir),
            ]
        );
        assert!(vault.list_dir(&vp("missing")).unwrap_err().is_not_found());
    }

    #[test]
    fn reads_whole_files_and_prefixes() {
        let (_dir, vault) = fixture();
        assert_eq!(vault.read(&vp("docs/a.md")).unwrap(), b"# A\n");
        assert_eq!(vault.read_to_string(&vp("docs/a.md")).unwrap(), "# A\n");
        assert_eq!(vault.read_prefix(&vp("docs/a.md"), 2).unwrap(), b"# ");
        assert_eq!(vault.read_prefix(&vp("docs/a.md"), 100).unwrap(), b"# A\n");
        assert!(vault.read(&vp("missing")).unwrap_err().is_not_found());
    }

    #[test]
    fn invalid_utf8_is_invalid_data() {
        let (dir, vault) = fixture();
        fs::write(dir.path().join("bin.md"), [0xff, 0xfe, 0x00]).unwrap();
        let err = vault.read_to_string(&vp("bin.md")).unwrap_err();
        assert_eq!(io::Error::from(err).kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn local_path_and_keys_are_the_native_paths() {
        let (_dir, vault) = fixture();
        let native = vault.root().join("docs").join("a.md");
        assert_eq!(vault.local_path(&vp("docs/a.md")), Some(native.clone()));
        assert_eq!(vault.key(&vp("docs/a.md")), native);
        assert_eq!(vault.vault_path(&native).unwrap(), vp("docs/a.md"));
        let outside = std::env::temp_dir().join("elsewhere.md");
        assert!(matches!(
            vault.vault_path(&outside),
            Err(VaultError::OutsideRoot { .. })
        ));
    }

    #[test]
    fn write_atomic_replaces_creates_and_leaves_no_temp_files() {
        let (dir, vault) = fixture();
        vault.write_atomic(&vp("docs/a.md"), b"# A2\n").unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("docs/a.md")).unwrap(),
            "# A2\n"
        );
        vault.write_atomic(&vp("docs/new.md"), b"new").unwrap();
        assert_eq!(fs::read(dir.path().join("docs/new.md")).unwrap(), b"new");
        let leftovers: Vec<_> = fs::read_dir(dir.path().join("docs"))
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.ends_with(".mbr-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn write_atomic_needs_an_existing_parent() {
        let (dir, vault) = fixture();
        assert!(vault.write_atomic(&vp("nope/x.md"), b"x").is_err());
        assert!(!dir.path().join("nope").exists());
        vault.create_dir_all(&vp("nope/deeper")).unwrap();
        vault.write_atomic(&vp("nope/deeper/x.md"), b"x").unwrap();
        assert!(vault.is_file(&vp("nope/deeper/x.md")));
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_keeps_the_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, vault) = fixture();
        let path = dir.path().join("docs/a.md");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        vault.write_atomic(&vp("docs/a.md"), b"secret").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn rename_and_remove() {
        let (_dir, vault) = fixture();
        vault.rename(&vp("top.txt"), &vp("docs/top.txt")).unwrap();
        assert!(!vault.is_file(&vp("top.txt")));
        assert!(vault.is_file(&vp("docs/top.txt")));
        vault.remove_file(&vp("docs/top.txt")).unwrap();
        assert_eq!(vault.stat(&vp("docs/top.txt")).unwrap(), None);
    }

    #[test]
    fn canonicalize_existing_and_missing() {
        let (_dir, vault) = fixture();
        assert_eq!(
            vault.canonicalize(&vp("docs/./a.md")).unwrap(),
            vp("docs/a.md")
        );
        assert_eq!(
            vault.canonicalize(&VaultPath::root()).unwrap(),
            VaultPath::root()
        );
        assert!(
            vault
                .canonicalize(&vp("missing"))
                .unwrap_err()
                .is_not_found()
        );
    }

    /// Symlink containment is decided by canonicalizing, exactly where the
    /// resolver's `safe_join` decided it.
    #[cfg(unix)]
    mod symlinks {
        use super::*;
        use crate::vault::resolve_under;
        use std::os::unix::fs::symlink;

        fn with_links() -> (tempfile::TempDir, tempfile::TempDir, LocalVault) {
            let (dir, vault) = fixture();
            let outside = tempfile::tempdir().unwrap();
            fs::write(outside.path().join("secret.md"), "# Secret").unwrap();
            fs::create_dir(outside.path().join("dir")).unwrap();
            fs::write(outside.path().join("dir/inner.md"), "# Inner").unwrap();
            // File and directory links out of the vault.
            symlink(outside.path().join("secret.md"), dir.path().join("leak.md")).unwrap();
            symlink(outside.path().join("dir"), dir.path().join("leakdir")).unwrap();
            // Links that stay inside.
            symlink(dir.path().join("docs/a.md"), dir.path().join("alias.md")).unwrap();
            symlink(dir.path().join("docs/sub"), dir.path().join("subalias")).unwrap();
            // A dangling link.
            symlink(
                dir.path().join("nowhere.md"),
                dir.path().join("dangling.md"),
            )
            .unwrap();
            (dir, outside, vault)
        }

        #[test]
        fn canonicalize_refuses_links_out_of_the_vault() {
            let (_dir, _outside, vault) = with_links();
            assert!(matches!(
                vault.canonicalize(&vp("leak.md")),
                Err(VaultError::OutsideRoot { .. })
            ));
            assert!(matches!(
                vault.canonicalize(&vp("leakdir")),
                Err(VaultError::OutsideRoot { .. })
            ));
            assert!(matches!(
                vault.canonicalize(&vp("leakdir/inner.md")),
                Err(VaultError::OutsideRoot { .. })
            ));
        }

        #[test]
        fn canonicalize_resolves_links_that_stay_inside() {
            let (_dir, _outside, vault) = with_links();
            assert_eq!(
                vault.canonicalize(&vp("alias.md")).unwrap(),
                vp("docs/a.md")
            );
            assert_eq!(
                vault.canonicalize(&vp("subalias/b.md")).unwrap(),
                vp("docs/sub/b.md")
            );
        }

        /// `resolve_under` must not fall through to the "missing file" branch
        /// for an *existing* link out of the vault — that branch validates only
        /// the parent and would hand back the unresolved link.
        #[test]
        fn resolve_under_refuses_escaping_links_and_follows_internal_ones() {
            let (_dir, _outside, vault) = with_links();
            let root = VaultPath::root();
            assert_eq!(resolve_under(&vault, &root, &vp("leak.md")), None);
            assert_eq!(resolve_under(&vault, &root, &vp("leakdir/inner.md")), None);
            // A probe *through* an escaping directory link is refused too: the
            // parent canonicalizes outside.
            assert_eq!(resolve_under(&vault, &root, &vp("leakdir/new.md")), None);
            assert_eq!(
                resolve_under(&vault, &root, &vp("alias.md")),
                Some(vp("docs/a.md"))
            );
            // A link inside the vault but outside a sub-base is refused for
            // that base (the static-folder case).
            symlink(
                vault.root().join("docs/a.md"),
                vault.root().join("docs/sub/up.md"),
            )
            .unwrap();
            assert_eq!(resolve_under(&vault, &vp("docs/sub"), &vp("up.md")), None);
            assert_eq!(
                resolve_under(&vault, &vp("docs/sub"), &vp("b.md")),
                Some(vp("docs/sub/b.md"))
            );
        }

        /// A dangling link is "does not exist": the probe answer is its own
        /// (in-vault) path, as `safe_join` answered.
        #[test]
        fn dangling_links_resolve_like_missing_files() {
            let (_dir, _outside, vault) = with_links();
            assert_eq!(
                resolve_under(&vault, &VaultPath::root(), &vp("dangling.md")),
                Some(vp("dangling.md"))
            );
            assert_eq!(vault.stat(&vp("dangling.md")).unwrap(), None);
        }

        /// Listing follows links (the scanner always did) and skips broken
        /// ones; deciding whether to *descend* is the caller's job, through
        /// `canonicalize`.
        #[test]
        fn list_dir_follows_links_and_skips_dangling_ones() {
            let (_dir, _outside, vault) = with_links();
            let entries = vault.list_dir(&VaultPath::root()).unwrap();
            let kind = |name: &str| entries.iter().find(|e| e.name() == name).map(|e| e.kind);
            assert_eq!(kind("leak.md"), Some(EntryKind::File));
            assert_eq!(kind("leakdir"), Some(EntryKind::Dir));
            assert_eq!(kind("subalias"), Some(EntryKind::Dir));
            assert_eq!(kind("dangling.md"), None);
        }

        /// A symlinked *root* is canonicalized at construction, so children
        /// relativize against the same spelling `canonicalize` produces.
        #[test]
        fn a_symlinked_root_still_contains_its_children() {
            let (dir, _vault) = fixture();
            let holder = tempfile::tempdir().unwrap();
            let link = holder.path().join("vault-link");
            symlink(dir.path(), &link).unwrap();
            let vault = LocalVault::new(&link);
            assert_eq!(
                vault.canonicalize(&vp("docs/a.md")).unwrap(),
                vp("docs/a.md")
            );
        }
    }
}
