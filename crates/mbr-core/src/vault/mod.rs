//! The storage seam: every repository-shaped read and write in the core goes
//! through a [`Vault`].
//!
//! mbr used to assume `std::fs` everywhere. That is right for the desktop and
//! wrong for everything else mbr is meant to run on: an iOS file provider can
//! hand back *dataless* files that block on `open()` while they download and
//! wants coordinated writes; Android's storage access framework has no paths at
//! all (`content://` tree URIs); an encrypted sync library is a linked API, not
//! a directory; and tests want a repository that lives in memory. The trait is
//! the one place those differences are allowed to live.
//!
//! # Rules
//!
//! - **No platform `cfg` in core.** No `#[cfg(target_os = "ios"/"android")]`
//!   anywhere in this crate. Platform behaviour is *injected* by implementing
//!   [`Vault`] in the platform crate (`mbr-ffi`), never by branching here.
//! - **Bulk-first.** Listing is [`Vault::list_dir`] — one call per directory,
//!   returning full [`Entry`]s (kind, size, times, availability) — so a
//!   foreign implementation is crossed once per directory, never once per
//!   file, and the desktop pays one `stat` per entry (fewer than the scanner
//!   paid before).
//! - **`local_path` fast path.** Where a real path exists, [`Vault::local_path`]
//!   returns it and hot consumers use it directly: grep's `search_path`,
//!   `ServeFile`, ffmpeg/pdfium probes, and the task scanner's buffer-reusing
//!   read. Every such consumer has a byte-reading fallback for when it is
//!   `None`, which is what the [`MemVault`]-backed tests exercise.
//! - **Containment at the boundary.** A [`VaultPath`] cannot name anything
//!   outside the vault lexically. Symlinks are resolved where untrusted input
//!   enters — [`Vault::canonicalize`] and [`resolve_under`] — exactly where the
//!   desktop always canonicalized, not on every read.
//!
//! # Keys
//!
//! The repository index ([`crate::repo::Repo`]) is still keyed by absolute
//! `PathBuf`s, as it was before this seam existed; for a vault those keys are
//! [`Vault::key`] — the vault path under [`Vault::root`]. For [`LocalVault`]
//! that is the canonical on-disk path, identical to the old keys; for any other
//! vault it is an opaque identifier that must only be turned back into a
//! [`VaultPath`] ([`Vault::vault_path`]), never opened.

mod local;
mod mem;
mod path;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub use local::{LocalVault, atomic_write, create_unique_temp_file, read_prefix_native};
pub use mem::MemVault;
pub use path::{VaultPath, VaultPathError};

/// What an [`Entry`] is. Symlinks are followed, as the desktop scanner always
/// followed them, so this is the kind of the link's *target*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntryKind {
    File,
    Dir,
}

/// Whether a file's bytes are on this device.
///
/// A file provider can list a file whose content is still in the cloud: iOS
/// "dataless" files report their real size from `stat`, but reading them
/// blocks (or, with the scanner's I/O policy, fails fast) until they download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Availability {
    /// The bytes are local; reading will not wait on the network.
    #[default]
    Local,
    /// Listed, but the content has to be downloaded first. Reads return
    /// [`VaultError::NotDownloaded`]; [`Vault::request_download`] asks for it.
    NotDownloaded,
    /// The vault cannot tell cheaply.
    Unknown,
}

/// One file or directory, as listed or stat'ed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: VaultPath,
    pub kind: EntryKind,
    /// Size in bytes (0 for directories on most backends).
    pub size: u64,
    /// Last modification time. [`SystemTime::UNIX_EPOCH`] when the backend
    /// cannot report one.
    pub modified: SystemTime,
    /// Creation (birth) time, where the backend has one. Many Linux
    /// filesystems and network mounts do not.
    pub created: Option<SystemTime>,
    pub availability: Availability,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Dir
    }

    pub fn is_file(&self) -> bool {
        self.kind == EntryKind::File
    }

    /// The entry's own name (last path segment); empty for the root.
    pub fn name(&self) -> &str {
        self.path.file_name().unwrap_or("")
    }

    /// `(created, modified)` in whole seconds since the Unix epoch, the shape
    /// the repository index stores.
    ///
    /// `None` when the modification time predates the epoch, which the scanner
    /// has always treated as "cannot process this file". A missing creation
    /// time falls back to the modification time, so a file is never dropped
    /// from listings just because its filesystem has no birth time.
    pub fn epoch_secs(&self) -> Option<(u64, u64)> {
        let secs = |t: SystemTime| {
            t.duration_since(SystemTime::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_secs())
        };
        let modified = secs(self.modified)?;
        let created = self.created.and_then(secs).unwrap_or(modified);
        Some((created, modified))
    }
}

/// Storage failures, classified so callers can map them without inspecting
/// `io::ErrorKind`s themselves.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("not found: {path}")]
    NotFound { path: String },

    /// The file is listed but its content is not on this device (see
    /// [`Availability::NotDownloaded`]).
    #[error("not downloaded: {path}")]
    NotDownloaded { path: String },

    #[error("permission denied: {path}")]
    PermissionDenied { path: String },

    /// The path resolves (through a symlink) to somewhere outside the vault.
    /// `path` names the resolved target.
    #[error("outside the vault root: {path}")]
    OutsideRoot { path: String },

    #[error(transparent)]
    InvalidPath(#[from] VaultPathError),

    #[error("{path}: {source}")]
    Io {
        path: String,
        #[source]
        source: io::Error,
    },
}

impl VaultError {
    /// Classifies an `io::Error` raised for `path`.
    ///
    /// `ErrorKind::Deadlock` is `EDEADLK`, which is what a read of a dataless
    /// file returns on Apple platforms when the reading thread has opted out of
    /// materializing it — so it means "not downloaded" wherever it occurs. The
    /// mapping needs no platform `cfg`: no other read returns it.
    pub fn from_io(path: impl fmt::Display, source: io::Error) -> Self {
        let path = path.to_string();
        match source.kind() {
            io::ErrorKind::NotFound => Self::NotFound { path },
            io::ErrorKind::PermissionDenied => Self::PermissionDenied { path },
            io::ErrorKind::Deadlock => Self::NotDownloaded { path },
            _ => Self::Io { path, source },
        }
    }

    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound { .. })
    }

    pub fn not_found(path: impl fmt::Display) -> Self {
        Self::NotFound {
            path: path.to_string(),
        }
    }
}

/// Lossy but kind-preserving: handlers that answer with an HTTP status derived
/// from `io::ErrorKind` (404 for `NotFound`, 403 for `PermissionDenied`) keep
/// answering the same status.
impl From<VaultError> for io::Error {
    fn from(error: VaultError) -> Self {
        let kind = match &error {
            VaultError::NotFound { .. } => io::ErrorKind::NotFound,
            VaultError::PermissionDenied { .. } | VaultError::OutsideRoot { .. } => {
                io::ErrorKind::PermissionDenied
            }
            VaultError::NotDownloaded { .. } => io::ErrorKind::WouldBlock,
            VaultError::InvalidPath(_) => io::ErrorKind::InvalidInput,
            VaultError::Io { .. } => {
                let VaultError::Io { source, .. } = error else {
                    unreachable!()
                };
                return source;
            }
        };
        io::Error::new(kind, error)
    }
}

/// A repository's storage.
///
/// See the [module docs](self) for the rules every implementation and caller
/// follows. Methods are synchronous: the desktop implementation is blocking
/// `std::fs`, and async callers already move repository I/O to
/// `spawn_blocking` (or run it on rayon), which is where a foreign
/// implementation's blocking calls belong too.
pub trait Vault: Send + Sync + fmt::Debug {
    /// The namespace [`Vault::key`]s live under. For [`LocalVault`], the
    /// canonical repository root; for any other vault, an identifier that is
    /// never opened.
    fn root(&self) -> &Path;

    /// The entry at `path` (following symlinks), or `None` when nothing is
    /// there.
    fn stat(&self, path: &VaultPath) -> Result<Option<Entry>, VaultError>;

    /// The immediate children of directory `dir`, in no particular order.
    ///
    /// Children that cannot be described are skipped rather than failing the
    /// listing: a broken symlink, a name that is not valid UTF-8 or not a valid
    /// [`VaultPath`] segment. That is what the scanner's old `WalkDir` loop did
    /// (`filter_map(Result::ok)`).
    fn list_dir(&self, dir: &VaultPath) -> Result<Vec<Entry>, VaultError>;

    /// Resolves symlinks in `path` and returns where it really is inside the
    /// vault.
    ///
    /// [`VaultError::NotFound`] when it does not exist;
    /// [`VaultError::OutsideRoot`] when it resolves outside the vault. A vault
    /// without links returns `path` itself for anything that exists.
    fn canonicalize(&self, path: &VaultPath) -> Result<VaultPath, VaultError>;

    /// The whole file.
    fn read(&self, path: &VaultPath) -> Result<Vec<u8>, VaultError>;

    /// At most the first `max_len` bytes (frontmatter extraction reads 8 KB).
    fn read_prefix(&self, path: &VaultPath, max_len: usize) -> Result<Vec<u8>, VaultError>;

    /// Replaces (or creates) `path` with `bytes` atomically: readers see the
    /// old content or the new, never a mix, and a failure leaves the old file.
    /// The parent directory must exist. An existing file's permissions are
    /// kept.
    ///
    /// Does not serialize writers; callers that read before writing hold a
    /// per-file lock across both (`mbr_server::file_write::FileWriteLocks`).
    fn write_atomic(&self, path: &VaultPath, bytes: &[u8]) -> Result<(), VaultError>;

    /// Creates `path` and any missing ancestors as directories.
    fn create_dir_all(&self, path: &VaultPath) -> Result<(), VaultError>;

    /// Removes the file at `path`.
    fn remove_file(&self, path: &VaultPath) -> Result<(), VaultError>;

    /// Renames `from` to `to`, replacing a file at `to`.
    fn rename(&self, from: &VaultPath, to: &VaultPath) -> Result<(), VaultError>;

    /// Asks for not-downloaded files to be fetched. Returns immediately;
    /// completion shows up as the files becoming readable. A no-op for vaults
    /// whose files are always local.
    fn request_download(&self, _paths: &[VaultPath]) {}

    /// A real filesystem path for `path`, when one exists — the fast path for
    /// consumers that need one (`ServeFile`, grep's `search_path`, ffmpeg,
    /// pdfium, mmap). `None` means "read it through the vault".
    fn local_path(&self, _path: &VaultPath) -> Option<PathBuf> {
        None
    }

    /// The whole file as UTF-8. Invalid UTF-8 is an [`VaultError::Io`] with
    /// `ErrorKind::InvalidData`, as `std::fs::read_to_string` reports it.
    fn read_to_string(&self, path: &VaultPath) -> Result<String, VaultError> {
        let bytes = self.read(path)?;
        String::from_utf8(bytes).map_err(|e| VaultError::Io {
            path: path.to_string(),
            source: io::Error::new(io::ErrorKind::InvalidData, e),
        })
    }

    /// The index key for `path`: `path` under [`Vault::root`].
    fn key(&self, path: &VaultPath) -> PathBuf {
        path.to_native(self.root())
    }

    /// The vault path for an index key — the inverse of [`Vault::key`].
    /// Lexical: it does not resolve symlinks.
    fn vault_path(&self, key: &Path) -> Result<VaultPath, VaultError> {
        VaultPath::from_absolute(self.root(), key).map_err(|e| match e {
            VaultPathError::NotUnderRoot { path, .. } => VaultError::OutsideRoot {
                path: path.display().to_string(),
            },
            other => VaultError::InvalidPath(other),
        })
    }

    /// Whether `path` is an existing file. Errors read as `false`, as
    /// `Path::is_file` does.
    fn is_file(&self, path: &VaultPath) -> bool {
        matches!(self.stat(path), Ok(Some(entry)) if entry.is_file())
    }

    /// Whether `path` is an existing directory. Errors read as `false`.
    fn is_dir(&self, path: &VaultPath) -> bool {
        matches!(self.stat(path), Ok(Some(entry)) if entry.is_dir())
    }
}

/// Places a configured folder (`static_folder`) inside `vault`, or `None`
/// when it lands nowhere in it.
///
/// The usual value is a plain relative path (`static`, `./assets/static`),
/// which is parsed lexically and needs no storage access. An absolute or
/// `..`-climbing spelling that still lands inside the root (`/abs/root/static`,
/// `../root/static`) can only be placed by resolving it on disk, so that half
/// needs a [`Vault::local_path`] — exactly as the old
/// `canonical_root.join(static_folder)` did. A value resolving *outside* the
/// root is an external overlay, which is a vault of its own and not this
/// function's business.
pub fn configured_folder(vault: &dyn Vault, configured: &str) -> Option<VaultPath> {
    VaultPath::new(configured).ok().or_else(|| {
        let local_root = vault.local_path(&VaultPath::root())?;
        let resolved = crate::config::resolve_existing_or_lexical(&local_root.join(configured));
        vault.vault_path(&resolved).ok()
    })
}

/// Resolves `relative` beneath `base`, refusing anything that ends up outside
/// `base` once symlinks are followed.
///
/// This is the vault form of the resolver's old `safe_join`, with the same
/// answers:
///
/// - `base` is canonicalized first; a `base` that does not exist resolves
///   nothing. The vault root is taken as canonical without asking.
/// - When `base/relative` exists, the answer is its **canonical** path, and
///   only if that is still under the canonical `base`. A final component that
///   is a symlink out of `base` is refused here rather than falling through to
///   the "does not exist yet" branch — that fall-through would validate only
///   the parent and hand back the unresolved link.
/// - When it does not exist (probing `foo.md` for `/foo/`), its parent must
///   canonicalize under `base`, and the answer is the canonical parent joined
///   with the requested name.
///
/// `relative` is already lexically normalized (it is a [`VaultPath`]), which is
/// the one difference from `safe_join`: a request's `..` is resolved by
/// position, as a browser resolves it, before any symlink is followed.
pub fn resolve_under(
    vault: &dyn Vault,
    base: &VaultPath,
    relative: &VaultPath,
) -> Option<VaultPath> {
    // The root is canonical by construction (nothing a `VaultPath` can name is
    // above it), so the common case costs no `canonicalize` for the base —
    // the resolver used to cache the canonical root for the same reason.
    let canonical_base = if base.is_root() {
        VaultPath::root()
    } else {
        vault.canonicalize(base).ok()?
    };
    let candidate = canonical_base.join_path(relative);
    match vault.canonicalize(&candidate) {
        Ok(canonical) => canonical.starts_with(&canonical_base).then_some(canonical),
        Err(VaultError::OutsideRoot { .. }) => None,
        Err(_) => {
            let name = candidate.file_name()?;
            let canonical_parent = vault.canonicalize(&candidate.parent()?).ok()?;
            if !canonical_parent.starts_with(&canonical_base) {
                return None;
            }
            canonical_parent.child(name).ok()
        }
    }
}

/// Every file in the vault, recursively, for a repository-wide pass that is
/// not the scanner (backlink grep, link rewriting on a move).
///
/// Follows symlinks the way the scanner does: each directory is listed once,
/// under its **canonical** path, so one reached twice (a link to a folder that
/// is also walked directly) contributes its files once, and a link cycle ends;
/// a directory that resolves outside the vault is not entered at all — so a
/// repository-wide *write* can never land outside the repository through a
/// link. `enter` prunes directories (it sees each directory's entry before it
/// is listed); every file below an entered directory is returned, with its
/// path under the canonical directory.
///
/// Sequential and one [`Vault::list_dir`] per directory. Directories that
/// cannot be listed are skipped, as `WalkDir`'s error entries were.
pub fn walk_files(vault: &dyn Vault, enter: impl Fn(&Entry) -> bool) -> Vec<Entry> {
    let mut files = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut pending = vec![VaultPath::root()];
    while let Some(dir) = pending.pop() {
        let Ok(canonical) = vault.canonicalize(&dir) else {
            continue;
        };
        if !visited.insert(canonical.clone()) {
            continue;
        }
        let Ok(entries) = vault.list_dir(&canonical) else {
            continue;
        };
        for entry in entries {
            if entry.is_dir() {
                if enter(&entry) {
                    pending.push(entry.path);
                }
            } else {
                files.push(entry);
            }
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_errors_are_classified() {
        let nf = VaultError::from_io("a.md", io::Error::from(io::ErrorKind::NotFound));
        assert!(nf.is_not_found());
        assert!(matches!(
            VaultError::from_io("a.md", io::Error::from(io::ErrorKind::PermissionDenied)),
            VaultError::PermissionDenied { .. }
        ));
        assert!(matches!(
            VaultError::from_io("a.md", io::Error::from(io::ErrorKind::Deadlock)),
            VaultError::NotDownloaded { .. }
        ));
        assert!(matches!(
            VaultError::from_io("a.md", io::Error::other("boom")),
            VaultError::Io { .. }
        ));
    }

    #[test]
    fn vault_errors_keep_their_io_kind() {
        let kind = |e: VaultError| io::Error::from(e).kind();
        assert_eq!(kind(VaultError::not_found("x")), io::ErrorKind::NotFound);
        assert_eq!(
            kind(VaultError::PermissionDenied { path: "x".into() }),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(
            kind(VaultError::OutsideRoot { path: "x".into() }),
            io::ErrorKind::PermissionDenied
        );
        let original = io::Error::new(io::ErrorKind::StorageFull, "disk full");
        assert_eq!(
            kind(VaultError::from_io("x", original)),
            io::ErrorKind::StorageFull
        );
    }

    #[test]
    fn epoch_secs_falls_back_to_modified_and_refuses_pre_epoch() {
        let at = |secs| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs);
        let entry = Entry {
            path: VaultPath::new("a.md").unwrap(),
            kind: EntryKind::File,
            size: 1,
            modified: at(200),
            created: None,
            availability: Availability::Local,
        };
        assert_eq!(entry.epoch_secs(), Some((200, 200)));
        let with_created = Entry {
            created: Some(at(100)),
            ..entry.clone()
        };
        assert_eq!(with_created.epoch_secs(), Some((100, 200)));
        let pre_epoch = Entry {
            modified: SystemTime::UNIX_EPOCH - std::time::Duration::from_secs(1),
            ..entry
        };
        assert_eq!(pre_epoch.epoch_secs(), None);
    }

    #[test]
    fn walk_files_lists_every_file_and_prunes_directories() {
        let vault = MemVault::new()
            .with_file("a.md", "a")
            .with_file("docs/b.md", "b")
            .with_file("docs/deep/c.md", "c")
            .with_file("node_modules/x.md", "x")
            .with_dir("empty");
        let mut found: Vec<String> = walk_files(&vault, |dir| dir.name() != "node_modules")
            .into_iter()
            .map(|e| e.path.to_string())
            .collect();
        found.sort();
        assert_eq!(found, vec!["a.md", "docs/b.md", "docs/deep/c.md"]);
    }

    #[cfg(unix)]
    #[test]
    fn walk_files_follows_links_inside_once_and_never_leaves_the_vault() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("real")).unwrap();
        std::fs::write(dir.path().join("real/note.md"), "n").unwrap();
        std::fs::write(outside.path().join("secret.md"), "s").unwrap();
        // A second route to `real`, a cycle, and a link out of the vault.
        symlink(dir.path().join("real"), dir.path().join("alias")).unwrap();
        symlink(dir.path(), dir.path().join("real/loop")).unwrap();
        symlink(outside.path(), dir.path().join("leak")).unwrap();
        let vault = LocalVault::new(dir.path());
        let found: Vec<String> = walk_files(&vault, |_| true)
            .into_iter()
            .map(|e| e.path.to_string())
            .collect();
        assert_eq!(
            found,
            vec!["real/note.md"],
            "listed once, canonically, nothing outside"
        );
    }

    #[test]
    fn resolve_under_on_a_mem_vault() {
        let vault = MemVault::new()
            .with_file("docs/a.md", "# A")
            .with_file("static/img.png", "png");
        let root = VaultPath::root();
        let p = |s: &str| VaultPath::new(s).unwrap();
        assert_eq!(
            resolve_under(&vault, &root, &p("docs/a.md")),
            Some(p("docs/a.md"))
        );
        // Missing file under an existing parent: the probe answer.
        assert_eq!(
            resolve_under(&vault, &root, &p("docs/b.md")),
            Some(p("docs/b.md"))
        );
        // Missing parent: nothing.
        assert_eq!(resolve_under(&vault, &root, &p("nope/b.md")), None);
        // Relative to a sub-base.
        assert_eq!(
            resolve_under(&vault, &p("static"), &p("img.png")),
            Some(p("static/img.png"))
        );
        // A base that does not exist resolves nothing.
        assert_eq!(resolve_under(&vault, &p("missing"), &p("img.png")), None);
    }
}
