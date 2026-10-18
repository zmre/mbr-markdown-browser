//! [`CoordinatedVault`]: the picked folder, read with `std::fs`, written
//! through the app's [`FileCoordinator`].

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mbr_core::vault::{
    Availability, Entry, EntryKind, LocalVault, Mount, Vault, VaultError, VaultPath,
};

use super::coordinator::FileCoordinator;

/// A folder the app was granted, as a [`Vault`].
///
/// **Reads** are the desktop's: a [`LocalVault`] on the real path the folder
/// picker handed over, so the scan, rendering and grep run on plain `std::fs`
/// with no call into Swift per file (`design/ios-architecture-research.md` §5.3).
///
/// **Writes** — `write_atomic`, `create_dir_all`, `rename`, `remove_file` — and
/// [`Vault::request_download`] go through the [`FileCoordinator`], because a
/// file provider must see them coordinated. Without a coordinator (desktop
/// tests, a folder in the app's own container) they are the [`LocalVault`]'s.
///
/// **Availability.** On Apple platforms a file provider's not-downloaded files
/// are reported [`Availability::NotDownloaded`]:
///
/// - a *dataless* file — real name, real size, `SF_DATALESS` in `st_flags` —
///   is classified from the metadata the listing already has
///   ([`LocalVault::with_availability_probe`]), so it costs no extra call;
/// - a legacy iCloud placeholder `.Name.ext.icloud` (still seen on some
///   versions) is listed as `Name.ext`, not downloaded, and never as a file of
///   its own; when both exist, the real file wins. Reads, `stat` and
///   `availability` of `Name.ext` know about the placeholder too.
///
/// Reading a not-downloaded file fails with [`VaultError::NotDownloaded`]
/// (`EDEADLK` under the runtime's fail-fast I/O policy, or the placeholder);
/// nothing here waits for a file's download. **Listings do wait** for the
/// provider to fetch a folder's entries (`iopolicy::with_materialization`):
/// with materialization off, a not-yet-fetched iCloud Drive folder lists only
/// what is already on the device, with no error at all.
pub struct CoordinatedVault {
    local: LocalVault,
    coordinator: Option<Arc<dyn FileCoordinator>>,
    /// Map `.Name.ext.icloud` placeholders to `Name.ext`. On by default on
    /// Apple platforms only, where they exist.
    icloud_placeholders: bool,
}

impl fmt::Debug for CoordinatedVault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CoordinatedVault")
            .field("local", &self.local)
            .field("coordinated", &self.coordinator.is_some())
            .field("icloud_placeholders", &self.icloud_placeholders)
            .finish()
    }
}

impl CoordinatedVault {
    /// The folder at `root` (canonicalized, as [`LocalVault::new`] does).
    pub fn new(root: impl Into<PathBuf>, coordinator: Option<Arc<dyn FileCoordinator>>) -> Self {
        let local = LocalVault::new(root);
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        let local = local.with_availability_probe(dataless_probe);
        Self {
            local,
            coordinator,
            icloud_placeholders: cfg!(target_vendor = "apple"),
        }
    }

    /// Turns placeholder mapping on or off (tests exercise it everywhere).
    #[must_use]
    pub fn with_icloud_placeholders(mut self, enabled: bool) -> Self {
        self.icloud_placeholders = enabled;
        self
    }

    fn native(&self, path: &VaultPath) -> PathBuf {
        self.local.native(path)
    }

    fn native_string(&self, path: &VaultPath) -> String {
        self.native(path).to_string_lossy().into_owned()
    }

    /// The placeholder entry standing in for `path`, when there is one and no
    /// real file.
    fn placeholder_for(&self, path: &VaultPath) -> Option<Entry> {
        if !self.icloud_placeholders {
            return None;
        }
        let name = path.file_name()?;
        let stub = path.parent()?.child(&placeholder_name(name)).ok()?;
        let entry = self.local.stat(&stub).ok()??;
        entry.is_file().then(|| Entry {
            path: path.clone(),
            // The stub is a small plist, not the file: its size says nothing.
            size: 0,
            availability: Availability::NotDownloaded,
            ..entry
        })
    }

    /// A read that found nothing: not downloaded when a placeholder says so.
    fn missing(&self, path: &VaultPath, error: VaultError) -> VaultError {
        if error.is_not_found() && self.placeholder_for(path).is_some() {
            VaultError::NotDownloaded {
                path: path.to_string(),
            }
        } else {
            error
        }
    }
}

/// `.Name.ext.icloud`, iCloud's placeholder for `Name.ext`.
fn placeholder_name(name: &str) -> String {
    format!(".{name}.icloud")
}

/// `Name.ext` for a placeholder name `.Name.ext.icloud`.
fn placeholder_target(name: &str) -> Option<&str> {
    let inner = name.strip_prefix('.')?.strip_suffix(".icloud")?;
    (!inner.is_empty()).then_some(inner)
}

/// `SF_DATALESS` (`<sys/stat.h>`): the file's content is not on the device.
#[cfg(any(target_os = "macos", target_os = "ios", test))]
const SF_DATALESS: u32 = 0x4000_0000;

/// Availability from a file's BSD flags.
#[cfg(any(target_os = "macos", target_os = "ios", test))]
fn availability_from_flags(flags: u32) -> Availability {
    if flags & SF_DATALESS != 0 {
        Availability::NotDownloaded
    } else {
        Availability::Local
    }
}

/// The [`LocalVault`] probe: `st_flags` of the metadata already fetched.
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn dataless_probe(metadata: &std::fs::Metadata) -> Availability {
    #[cfg(target_os = "ios")]
    use std::os::ios::fs::MetadataExt;
    #[cfg(target_os = "macos")]
    use std::os::macos::fs::MetadataExt;
    availability_from_flags(metadata.st_flags())
}

impl Vault for CoordinatedVault {
    fn root(&self) -> &Path {
        self.local.root()
    }

    fn stat(&self, path: &VaultPath) -> Result<Option<Entry>, VaultError> {
        match self.local.stat(path)? {
            Some(entry) => Ok(Some(entry)),
            None => Ok(self.placeholder_for(path)),
        }
    }

    fn list_dir(&self, dir: &VaultPath) -> Result<Vec<Entry>, VaultError> {
        // With materialization off, a file provider's not-yet-fetched folder
        // lists only what is already on the device, *without* an error — so
        // the listing alone may wait for the provider (entries only, never a
        // file's contents; see `iopolicy`). Should that fail (offline, the
        // provider refusing), what is local still lists, as before.
        let entries = match super::iopolicy::with_materialization(|| self.local.list_dir(dir)) {
            Ok(entries) => entries,
            Err(e) if e.is_not_found() => return Err(e),
            Err(e) => {
                tracing::info!(
                    "Listing {dir:?} with materialization failed ({e}); listing what is local"
                );
                self.local.list_dir(dir)?
            }
        };
        if !self.icloud_placeholders {
            return Ok(entries);
        }
        let real: std::collections::HashSet<String> = entries
            .iter()
            .filter(|e| placeholder_target(e.name()).is_none())
            .map(|e| e.name().to_owned())
            .collect();
        Ok(entries
            .into_iter()
            .filter_map(|entry| match placeholder_target(entry.name()) {
                None => Some(entry),
                Some(_) if entry.kind != EntryKind::File => Some(entry),
                Some(target) if real.contains(target) => None,
                Some(target) => Some(Entry {
                    path: dir.child(target).ok()?,
                    size: 0,
                    availability: Availability::NotDownloaded,
                    ..entry
                }),
            })
            .collect())
    }

    fn canonicalize(&self, path: &VaultPath) -> Result<VaultPath, VaultError> {
        self.local.canonicalize(path).or_else(|e| {
            // A placeholder's file does not exist on disk under its own name;
            // it resolves to itself under its (real, canonical) parent.
            match (e.is_not_found(), self.placeholder_for(path), path.parent()) {
                (true, Some(_), Some(parent)) => {
                    let parent = self.local.canonicalize(&parent)?;
                    Ok(parent.child(path.file_name().unwrap_or_default())?)
                }
                _ => Err(e),
            }
        })
    }

    fn read(&self, path: &VaultPath) -> Result<Vec<u8>, VaultError> {
        self.local.read(path).map_err(|e| self.missing(path, e))
    }

    fn read_prefix(&self, path: &VaultPath, max_len: usize) -> Result<Vec<u8>, VaultError> {
        self.local
            .read_prefix(path, max_len)
            .map_err(|e| self.missing(path, e))
    }

    fn read_range(&self, path: &VaultPath, offset: u64, len: usize) -> Result<Vec<u8>, VaultError> {
        self.local
            .read_range(path, offset, len)
            .map_err(|e| self.missing(path, e))
    }

    fn availability(&self, path: &VaultPath) -> Availability {
        match self.local.availability(path) {
            // A missing file answers `Unknown` with the dataless probe but
            // `Local` without one (no I/O), so consult the placeholder in both
            // cases — `stat` and `list_dir` already do, and the three must agree.
            Availability::Local | Availability::Unknown
                if self.icloud_placeholders
                    && !matches!(self.local.stat(path), Ok(Some(_)))
                    && self.placeholder_for(path).is_some() =>
            {
                Availability::NotDownloaded
            }
            availability => availability,
        }
    }

    fn write_atomic(&self, path: &VaultPath, bytes: &[u8]) -> Result<(), VaultError> {
        match &self.coordinator {
            Some(coordinator) => coordinator
                .coordinated_write(self.native_string(path), bytes.to_vec())
                .map_err(|e| e.into_vault_error(path)),
            None => self.local.write_atomic(path, bytes),
        }
    }

    fn create_dir_all(&self, path: &VaultPath) -> Result<(), VaultError> {
        match &self.coordinator {
            Some(coordinator) => coordinator
                .coordinated_create_directory(self.native_string(path))
                .map_err(|e| e.into_vault_error(path)),
            None => self.local.create_dir_all(path),
        }
    }

    fn remove_file(&self, path: &VaultPath) -> Result<(), VaultError> {
        match &self.coordinator {
            Some(coordinator) => coordinator
                .coordinated_remove(self.native_string(path))
                .map_err(|e| e.into_vault_error(path)),
            None => self.local.remove_file(path),
        }
    }

    fn rename(&self, from: &VaultPath, to: &VaultPath) -> Result<(), VaultError> {
        match &self.coordinator {
            Some(coordinator) => coordinator
                .coordinated_move(self.native_string(from), self.native_string(to))
                .map_err(|e| e.into_vault_error(from)),
            None => self.local.rename(from, to),
        }
    }

    fn request_download(&self, paths: &[VaultPath]) {
        if let Some(coordinator) = &self.coordinator
            && !paths.is_empty()
        {
            coordinator.request_download(paths.iter().map(|p| self.native_string(p)).collect());
        }
    }

    fn local_path(&self, path: &VaultPath) -> Option<PathBuf> {
        self.local.local_path(path)
    }

    fn is_read_only(&self, path: &VaultPath) -> bool {
        self.local.is_read_only(path)
    }

    fn mounts(&self) -> Vec<Mount> {
        self.local.mounts()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::mobile::coordinator::CoordinatorError;
    use std::sync::Mutex;

    /// A coordinator that records every call and then does the operation
    /// with `std::fs`, so the folder stays consistent for the test.
    #[derive(Default)]
    pub(crate) struct RecordingCoordinator {
        pub(crate) calls: Mutex<Vec<String>>,
        pub(crate) downloads: Mutex<Vec<String>>,
        /// Files a download request delivers, by native path: written, and
        /// their `.Name.icloud` placeholder removed, as the provider would.
        pub(crate) deliveries: Mutex<std::collections::HashMap<String, Vec<u8>>>,
    }

    impl RecordingCoordinator {
        pub(crate) fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    fn io(e: std::io::Error) -> CoordinatorError {
        CoordinatorError::Failed {
            message: e.to_string(),
        }
    }

    impl FileCoordinator for RecordingCoordinator {
        fn coordinated_write(
            &self,
            path: String,
            contents: Vec<u8>,
        ) -> Result<(), CoordinatorError> {
            self.calls.lock().unwrap().push(format!("write {path}"));
            mbr_core::vault::atomic_write(Path::new(&path), &contents).map_err(io)
        }
        fn coordinated_create_directory(&self, path: String) -> Result<(), CoordinatorError> {
            self.calls.lock().unwrap().push(format!("mkdir {path}"));
            std::fs::create_dir_all(path).map_err(io)
        }
        fn coordinated_move(&self, from: String, to: String) -> Result<(), CoordinatorError> {
            self.calls.lock().unwrap().push(format!("move {from} {to}"));
            std::fs::rename(from, to).map_err(io)
        }
        fn coordinated_remove(&self, path: String) -> Result<(), CoordinatorError> {
            self.calls.lock().unwrap().push(format!("remove {path}"));
            std::fs::remove_file(&path).map_err(|e| CoordinatorError::NotFound {
                message: e.to_string(),
            })
        }
        fn request_download(&self, paths: Vec<String>) {
            let mut deliveries = self.deliveries.lock().unwrap();
            for path in &paths {
                if let Some(bytes) = deliveries.remove(path) {
                    let path = Path::new(path);
                    let name = path.file_name().unwrap().to_string_lossy();
                    let stub = path.with_file_name(placeholder_name(&name));
                    std::fs::write(path, bytes).unwrap();
                    let _ = std::fs::remove_file(stub);
                }
            }
            self.downloads.lock().unwrap().extend(paths);
        }
    }

    fn vp(s: &str) -> VaultPath {
        VaultPath::new(s).unwrap()
    }

    fn folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("docs")).unwrap();
        std::fs::write(dir.path().join("docs/a.md"), "# A").unwrap();
        // A placeholder alone, and a placeholder beside its real file.
        std::fs::write(dir.path().join("docs/.cloud.md.icloud"), "plist").unwrap();
        std::fs::write(dir.path().join("docs/.a.md.icloud"), "plist").unwrap();
        dir
    }

    #[test]
    fn writes_and_downloads_go_through_the_coordinator() {
        let dir = folder();
        let coordinator = Arc::new(RecordingCoordinator::default());
        let vault = CoordinatedVault::new(
            dir.path(),
            Some(Arc::clone(&coordinator) as Arc<dyn FileCoordinator>),
        );
        let root = vault.root().to_path_buf();
        vault.write_atomic(&vp("docs/a.md"), b"# A2").unwrap();
        vault.create_dir_all(&vp("new/deep")).unwrap();
        vault
            .rename(&vp("docs/a.md"), &vp("new/deep/a.md"))
            .unwrap();
        vault.remove_file(&vp("new/deep/a.md")).unwrap();
        assert!(matches!(
            vault.remove_file(&vp("new/deep/a.md")),
            Err(VaultError::NotFound { .. })
        ));
        let p = |s: &str| root.join(s).display().to_string();
        assert_eq!(
            coordinator.calls(),
            vec![
                format!("write {}", p("docs/a.md")),
                format!("mkdir {}", p("new/deep")),
                format!("move {} {}", p("docs/a.md"), p("new/deep/a.md")),
                format!("remove {}", p("new/deep/a.md")),
                format!("remove {}", p("new/deep/a.md")),
            ]
        );
        vault.request_download(&[vp("docs/cloud.md")]);
        assert_eq!(
            *coordinator.downloads.lock().unwrap(),
            vec![p("docs/cloud.md")]
        );
        // Reads never call back.
        assert!(vault.read(&vp("docs/.cloud.md.icloud")).is_ok());
        assert_eq!(coordinator.calls().len(), 5);
    }

    #[test]
    fn without_a_coordinator_writes_are_local() {
        let dir = folder();
        let vault = CoordinatedVault::new(dir.path(), None);
        vault.write_atomic(&vp("docs/a.md"), b"# local").unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("docs/a.md")).unwrap(),
            b"# local"
        );
        vault.request_download(&[vp("docs/a.md")]);
    }

    #[test]
    fn icloud_placeholders_stand_for_their_files() {
        let dir = folder();
        let vault = CoordinatedVault::new(dir.path(), None).with_icloud_placeholders(true);
        let mut names: Vec<(String, Availability)> = vault
            .list_dir(&vp("docs"))
            .unwrap()
            .into_iter()
            .map(|e| (e.name().to_owned(), e.availability))
            .collect();
        names.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            names,
            vec![
                ("a.md".to_string(), Availability::Local),
                ("cloud.md".to_string(), Availability::NotDownloaded),
            ],
            "the real file wins; a placeholder is never listed as itself"
        );
        let cloud = vp("docs/cloud.md");
        let entry = vault.stat(&cloud).unwrap().unwrap();
        assert!(entry.is_file());
        assert_eq!(entry.availability, Availability::NotDownloaded);
        assert_eq!(vault.availability(&cloud), Availability::NotDownloaded);
        assert!(matches!(
            vault.read(&cloud),
            Err(VaultError::NotDownloaded { .. })
        ));
        assert!(matches!(
            vault.read_prefix(&cloud, 10),
            Err(VaultError::NotDownloaded { .. })
        ));
        assert_eq!(vault.canonicalize(&cloud).unwrap(), cloud);
        assert!(
            vault
                .read(&vp("docs/missing.md"))
                .unwrap_err()
                .is_not_found()
        );
        assert_eq!(vault.stat(&vp("docs/missing.md")).unwrap(), None);

        let off = CoordinatedVault::new(dir.path(), None).with_icloud_placeholders(false);
        assert_eq!(off.list_dir(&vp("docs")).unwrap().len(), 3);
        assert_eq!(off.stat(&cloud).unwrap(), None);
    }

    #[test]
    fn placeholder_names_round_trip() {
        assert_eq!(placeholder_name("a.md"), ".a.md.icloud");
        assert_eq!(placeholder_target(".a.md.icloud"), Some("a.md"));
        assert_eq!(placeholder_target(".icloud"), None);
        assert_eq!(placeholder_target("..icloud"), None);
        assert_eq!(placeholder_target("a.md.icloud"), None);
        assert_eq!(placeholder_target(".a.md"), None);
    }

    #[test]
    fn the_dataless_flag_means_not_downloaded() {
        assert_eq!(availability_from_flags(0), Availability::Local);
        assert_eq!(
            availability_from_flags(SF_DATALESS | 0x20),
            Availability::NotDownloaded
        );
        assert_eq!(availability_from_flags(0x20), Availability::Local);
    }

    /// The probe is wired in on Apple: an ordinary file is local.
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    #[test]
    fn ordinary_files_are_local_through_the_probe() {
        let dir = folder();
        let vault = CoordinatedVault::new(dir.path(), None);
        assert_eq!(vault.availability(&vp("docs/a.md")), Availability::Local);
        assert_eq!(
            vault.stat(&vp("docs/a.md")).unwrap().unwrap().availability,
            Availability::Local
        );
    }
}
