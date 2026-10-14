//! [`SingleFileVault`]: one file opened in place, with nothing around it.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use mbr_core::vault::{Availability, Entry, EntryKind, Vault, VaultError, VaultPath};

use super::coordinated_vault::CoordinatedVault;

/// A vault rooted at a file's folder that contains exactly that file.
///
/// iOS grants access to a file opened in place (from Files, the share sheet,
/// or a provider that cannot grant folders: Google Drive, OneDrive, Sync.com)
/// and to **nothing beside it**, so the vault does not pretend otherwise: the
/// root lists the one file, and every other path does not exist. The folder
/// itself is never touched — not listed, not stat'ed — since asking would only
/// be refused. Relative images and links to neighbours therefore come out as
/// missing (a 404), the same as a broken link on the desktop.
///
/// The file itself behaves as in a [`CoordinatedVault`]: read with `std::fs`,
/// written (task toggles, flashcard reviews) through the coordinator, reported
/// not downloaded when it is dataless. Writing anything else is refused.
///
/// "Grant access to the folder" is not an upgrade of this vault: the app
/// opens the parent as an ordinary folder vault, which replaces this one.
#[derive(Debug)]
pub struct SingleFileVault {
    inner: CoordinatedVault,
    file: VaultPath,
}

impl SingleFileVault {
    /// `inner` is rooted at the file's folder; `file_name` is the file.
    pub fn new(inner: CoordinatedVault, file_name: &str) -> Result<Self, VaultError> {
        let file = VaultPath::root().child(file_name)?;
        Ok(Self { inner, file })
    }

    /// The one file, as a vault path.
    pub fn file(&self) -> &VaultPath {
        &self.file
    }

    fn only_the_file(&self, path: &VaultPath) -> Result<(), VaultError> {
        if *path == self.file {
            Ok(())
        } else {
            Err(VaultError::not_found(path))
        }
    }

    fn root_entry() -> Entry {
        Entry {
            path: VaultPath::root(),
            kind: EntryKind::Dir,
            size: 0,
            modified: SystemTime::UNIX_EPOCH,
            created: None,
            availability: Availability::Local,
        }
    }
}

impl Vault for SingleFileVault {
    fn root(&self) -> &Path {
        self.inner.root()
    }

    fn stat(&self, path: &VaultPath) -> Result<Option<Entry>, VaultError> {
        if path.is_root() {
            Ok(Some(Self::root_entry()))
        } else if *path == self.file {
            self.inner.stat(path)
        } else {
            Ok(None)
        }
    }

    fn list_dir(&self, dir: &VaultPath) -> Result<Vec<Entry>, VaultError> {
        if !dir.is_root() {
            return Err(VaultError::not_found(dir));
        }
        Ok(self.inner.stat(&self.file)?.into_iter().collect())
    }

    fn canonicalize(&self, path: &VaultPath) -> Result<VaultPath, VaultError> {
        // No `realpath`: resolving would walk the folder. The file is taken as
        // named, which is the name the grant was for.
        if path.is_root() || *path == self.file {
            Ok(path.clone())
        } else {
            Err(VaultError::not_found(path))
        }
    }

    fn read(&self, path: &VaultPath) -> Result<Vec<u8>, VaultError> {
        self.only_the_file(path)?;
        self.inner.read(path)
    }

    fn read_prefix(&self, path: &VaultPath, max_len: usize) -> Result<Vec<u8>, VaultError> {
        self.only_the_file(path)?;
        self.inner.read_prefix(path, max_len)
    }

    fn read_range(&self, path: &VaultPath, offset: u64, len: usize) -> Result<Vec<u8>, VaultError> {
        self.only_the_file(path)?;
        self.inner.read_range(path, offset, len)
    }

    fn availability(&self, path: &VaultPath) -> Availability {
        if *path == self.file {
            self.inner.availability(path)
        } else if path.is_root() {
            Availability::Local
        } else {
            Availability::Unknown
        }
    }

    fn write_atomic(&self, path: &VaultPath, bytes: &[u8]) -> Result<(), VaultError> {
        if *path != self.file {
            return Err(refused(path));
        }
        self.inner.write_atomic(path, bytes)
    }

    fn create_dir_all(&self, path: &VaultPath) -> Result<(), VaultError> {
        Err(refused(path))
    }

    fn remove_file(&self, path: &VaultPath) -> Result<(), VaultError> {
        Err(refused(path))
    }

    fn rename(&self, from: &VaultPath, _to: &VaultPath) -> Result<(), VaultError> {
        Err(refused(from))
    }

    fn request_download(&self, paths: &[VaultPath]) {
        if paths.contains(&self.file) {
            self.inner
                .request_download(std::slice::from_ref(&self.file));
        }
    }

    fn local_path(&self, path: &VaultPath) -> Option<PathBuf> {
        (*path == self.file)
            .then(|| self.inner.local_path(path))
            .flatten()
    }
}

/// Only the opened file may be written; there is no grant for anything else.
fn refused(path: &VaultPath) -> VaultError {
    VaultError::PermissionDenied {
        path: path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp(s: &str) -> VaultPath {
        VaultPath::new(s).unwrap()
    }

    fn fixture() -> (tempfile::TempDir, SingleFileVault) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.md"), "# Note\n\n![x](pic.png)\n").unwrap();
        std::fs::write(dir.path().join("pic.png"), "png").unwrap();
        std::fs::write(dir.path().join("sibling.md"), "# Sibling").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let vault =
            SingleFileVault::new(CoordinatedVault::new(dir.path(), None), "note.md").unwrap();
        (dir, vault)
    }

    #[test]
    fn only_the_file_exists() {
        let (_dir, vault) = fixture();
        let listed: Vec<String> = vault
            .list_dir(&VaultPath::root())
            .unwrap()
            .into_iter()
            .map(|e| e.path.to_string())
            .collect();
        assert_eq!(listed, vec!["note.md"]);
        assert!(vault.is_dir(&VaultPath::root()));
        assert!(vault.is_file(&vp("note.md")));
        for other in ["pic.png", "sibling.md", "sub", "sub/x.md"] {
            assert_eq!(vault.stat(&vp(other)).unwrap(), None, "{other}");
            assert!(
                vault.read(&vp(other)).unwrap_err().is_not_found(),
                "{other}"
            );
            assert!(vault.canonicalize(&vp(other)).is_err(), "{other}");
            assert_eq!(vault.local_path(&vp(other)), None, "{other}");
        }
        assert!(vault.list_dir(&vp("sub")).is_err());
        assert!(
            vault
                .read_to_string(&vp("note.md"))
                .unwrap()
                .contains("# Note")
        );
        assert!(vault.local_path(&vp("note.md")).is_some());
    }

    #[test]
    fn only_the_file_may_be_written() {
        let (dir, vault) = fixture();
        vault.write_atomic(&vp("note.md"), b"# Changed").unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("note.md")).unwrap(),
            b"# Changed"
        );
        let denied =
            |r: Result<(), VaultError>| matches!(r, Err(VaultError::PermissionDenied { .. }));
        assert!(denied(vault.write_atomic(&vp("sibling.md"), b"x")));
        assert!(denied(vault.create_dir_all(&vp("new"))));
        assert!(denied(vault.remove_file(&vp("note.md"))));
        assert!(denied(vault.rename(&vp("note.md"), &vp("moved.md"))));
        assert_eq!(
            std::fs::read(dir.path().join("sibling.md")).unwrap(),
            b"# Sibling"
        );
    }
}
