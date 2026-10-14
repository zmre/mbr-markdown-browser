//! The persistent index: a [`RepoSnapshot`] kept in the app's `Caches/`.
//!
//! **The one exception to "no local cache files".** mbr never writes a cache
//! on the desktop, and never anywhere in the notes folder. The iOS app may, in
//! its own sandboxed `Caches/` directory (which the system may purge at any
//! time, and which is never synced or backed up), because a phone re-reading
//! 3k notes' frontmatter from a file provider on every launch is the slowest
//! thing it does. Only [`super::MbrApp::open`] with a `cache_dir` gets here.
//!
//! - **Keyed by vault identity**: the file name is a hash of what the app
//!   passes as the vault's identity (a bookmark's stable identifier), or of
//!   the canonical root path. Nothing about the notes is in the name.
//! - **Versioned**: [`RepoSnapshot`] carries a format number and a fingerprint
//!   of the mbr version and scan configuration. A snapshot that fails either,
//!   or does not parse, is ignored and replaced after the next scan — never an
//!   error.
//! - **Atomic**: written to a temp file beside it and renamed, so a crash
//!   mid-write leaves the previous snapshot, not half of one.

use std::io;
use std::path::{Path, PathBuf};

use mbr_core::repo::RepoSnapshot;
use sha2::{Digest, Sha256};

/// Where one vault's snapshot lives.
#[derive(Debug, Clone)]
pub(crate) struct IndexCache {
    path: PathBuf,
}

impl IndexCache {
    /// The snapshot file for the vault known as `identity`, in `cache_dir`.
    pub(crate) fn new(cache_dir: &Path, identity: &str) -> Self {
        let digest = Sha256::digest(identity.as_bytes());
        let hex: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
        Self {
            path: cache_dir.join(format!("mbr-index-{hex}.json")),
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// The saved snapshot, when there is a readable one of this build's
    /// format. A missing, unreadable or foreign file is `None` (and logged).
    pub(crate) fn load(&self) -> Option<RepoSnapshot> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
            Err(e) => {
                tracing::info!("Index cache {} unreadable: {e}", self.path.display());
                return None;
            }
        };
        RepoSnapshot::from_bytes(&bytes)
            .inspect_err(|e| tracing::info!("Index cache {} ignored: {e}", self.path.display()))
            .ok()
    }

    /// Replaces the saved snapshot. Creates the cache directory if needed.
    pub(crate) fn save(&self, snapshot: &RepoSnapshot) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        mbr_core::vault::atomic_write(&self.path, &snapshot.to_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_per_identity_and_reveal_nothing() {
        let dir = Path::new("/caches");
        let a = IndexCache::new(dir, "/Users/me/Notes");
        let b = IndexCache::new(dir, "/Users/me/Other");
        assert_ne!(a.path(), b.path());
        assert_eq!(a.path(), IndexCache::new(dir, "/Users/me/Notes").path());
        let name = a.path().file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("mbr-index-") && name.ends_with(".json"));
        assert!(!name.contains("Notes"));
    }

    #[test]
    fn missing_and_corrupt_files_load_as_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let cache = IndexCache::new(dir.path(), "v");
        assert!(cache.load().is_none());
        std::fs::write(cache.path(), b"{ not json").unwrap();
        assert!(cache.load().is_none());
        std::fs::write(cache.path(), br#"{"format": 999}"#).unwrap();
        assert!(cache.load().is_none());
    }
}
