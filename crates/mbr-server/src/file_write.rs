//! Writing the user's files in place: atomic replacement and per-file locks.
//!
//! Every server endpoint that rewrites an existing note — the editor, task
//! toggles, flashcard reviews, moves and the link rewriting a move triggers —
//! goes through here, so they agree on two things:
//!
//! * **Atomic replacement** ([`atomic_write`], implemented by
//!   [`mbr_core::vault::LocalVault`] and re-exported here): a temp file with a
//!   name of its own, beside the target, renamed over it, carrying the
//!   target's permissions.
//! * **Serialization** ([`FileWriteLocks`]): a read-patch-write cycle on one
//!   file excludes every other one on the same file, so no write is silently
//!   overwritten by a second writer that read the older text.
//!
//! Knows nothing about HTTP; callers map `std::io::Error` themselves.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use tokio::sync::{Mutex, OwnedMutexGuard};

/// One async mutex per file, for code that reads a file, patches it and
/// writes it back.
///
/// Without it two writes to the same note — a task toggle and a flashcard
/// review a moment apart, two windows, or a move rewriting a link in a note
/// being toggled — both read the old text and the second rename silently
/// drops the first one's change.
///
/// Entries are `Weak`: a file's mutex lives exactly as long as someone holds
/// or awaits it, and dead entries are swept on every acquisition, so the map
/// only ever holds the files being written right now.
///
/// Keys are compared as given, so callers must pass **canonical** paths (two
/// spellings of one file would otherwise get two locks). Not reentrant: never
/// lock a path the current task already holds.
#[derive(Default)]
pub struct FileWriteLocks {
    locks: parking_lot::Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>,
}

impl FileWriteLocks {
    /// The mutex for `path`, created on first use.
    fn mutex_for(&self, path: &Path) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock();
        locks.retain(|_, weak| weak.strong_count() > 0);
        if let Some(mutex) = locks.get(path).and_then(Weak::upgrade) {
            return mutex;
        }
        let mutex = Arc::new(Mutex::new(()));
        locks.insert(path.to_path_buf(), Arc::downgrade(&mutex));
        mutex
    }

    /// Waits for exclusive write access to `path`.
    pub async fn lock(&self, path: &Path) -> OwnedMutexGuard<()> {
        self.mutex_for(path).lock_owned().await
    }

    /// [`Self::lock`] for blocking code (`spawn_blocking`, plain threads).
    ///
    /// # Panics
    ///
    /// When called from within an async execution context, like tokio's own
    /// `blocking_lock`.
    pub fn lock_blocking(&self, path: &Path) -> OwnedMutexGuard<()> {
        self.mutex_for(path).blocking_lock_owned()
    }

    /// Locks every path in `paths`, in sorted order and once each, so two
    /// callers locking overlapping sets can never deadlock. Blocking.
    pub fn lock_all_blocking(&self, paths: &[&Path]) -> Vec<OwnedMutexGuard<()>> {
        let mut ordered: Vec<&Path> = paths.to_vec();
        ordered.sort_unstable();
        ordered.dedup();
        ordered
            .into_iter()
            .map(|path| self.lock_blocking(path))
            .collect()
    }

    /// Files with a live lock (held or awaited). For tests.
    #[cfg(test)]
    fn live(&self) -> usize {
        self.locks
            .lock()
            .values()
            .filter(|weak| weak.strong_count() > 0)
            .count()
    }
}

// The atomic-replacement primitives live with the local vault, which is the
// one implementation of `Vault::write_atomic` on disk; re-exported here so the
// rest of the server keeps a single import for "how files are written".
pub use mbr_core::vault::{atomic_write, create_unique_temp_file};

#[cfg(test)]
mod tests {
    use super::*;

    /// Names in `dir` that look like one of our temp files.
    fn temp_files_in(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .expect("read dir")
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .filter(|name| name.ends_with(".mbr-tmp"))
            .collect()
    }

    #[test]
    fn unique_temp_files_never_share_a_name() {
        let temp = tempfile::tempdir().expect("temp dir");
        let (a, _fa) = create_unique_temp_file(temp.path(), "note.md").expect("first");
        let (b, _fb) = create_unique_temp_file(temp.path(), "note.md").expect("second");
        assert_ne!(a, b);
        for path in [&a, &b] {
            let name = path.file_name().and_then(|n| n.to_str()).expect("name");
            assert!(
                name.starts_with(".note.md.") && name.ends_with(".mbr-tmp"),
                "{name}"
            );
        }
    }

    /// Concurrent writers to one file each get their own temp file, so every
    /// rename succeeds (the shared `.{name}.mbr-tmp` made the loser `ENOENT`)
    /// and nothing is left behind.
    #[test]
    fn concurrent_atomic_writes_all_succeed_and_leave_no_temp_files() {
        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("note.md");
        std::fs::write(&path, "start\n").expect("seed");
        let results: Vec<_> = std::thread::scope(|scope| {
            (0..16)
                .map(|i| {
                    let path = &path;
                    scope.spawn(move || atomic_write(path, format!("writer {i}\n").as_bytes()))
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| handle.join().expect("writer thread"))
                .collect()
        });
        for result in &results {
            assert!(result.is_ok(), "{result:?}");
        }
        let text = std::fs::read_to_string(&path).expect("read back");
        assert!(text.starts_with("writer "), "{text}");
        assert_eq!(temp_files_in(temp.path()), Vec::<String>::new());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_keeps_the_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("private.md");
        std::fs::write(&path, "secret\n").expect("seed");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
        atomic_write(&path, b"still secret\n").expect("write");
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "still secret\n");
    }

    #[tokio::test]
    async fn locks_serialize_one_path_and_forget_it_afterwards() {
        let locks = Arc::new(FileWriteLocks::default());
        let a = Path::new("/repo/a.md");
        let guard = locks.lock(a).await;
        // Another file is independent.
        drop(locks.lock(Path::new("/repo/b.md")).await);

        // The same file waits until the first guard is dropped.
        let waiter = {
            let locks = Arc::clone(&locks);
            tokio::spawn(async move { drop(locks.lock(Path::new("/repo/a.md")).await) })
        };
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(!waiter.is_finished(), "second lock on a.md must wait");
        drop(guard);
        waiter.await.expect("waiter");

        // Nothing holds or awaits a lock now, so nothing is kept alive.
        assert_eq!(locks.live(), 0);
        drop(locks.lock(a).await);
        assert!(locks.locks.lock().len() <= 1, "dead entries are swept");
    }

    /// The blocking and async halves share one mutex per path.
    #[tokio::test]
    async fn blocking_and_async_locks_exclude_each_other() {
        let locks = Arc::new(FileWriteLocks::default());
        let guard = locks.lock(Path::new("/repo/a.md")).await;
        let blocked = {
            let locks = Arc::clone(&locks);
            tokio::task::spawn_blocking(move || drop(locks.lock_blocking(Path::new("/repo/a.md"))))
        };
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(
            !blocked.is_finished(),
            "blocking lock must wait for async holder"
        );
        drop(guard);
        blocked.await.expect("blocking locker");
    }

    /// Two threads locking the same pair in opposite orders: sorting inside
    /// `lock_all_blocking` is what keeps this from deadlocking.
    #[test]
    fn lock_all_blocking_orders_paths_and_dedups() {
        let locks = Arc::new(FileWriteLocks::default());
        let (a, b) = (Path::new("/repo/a.md"), Path::new("/repo/b.md"));
        std::thread::scope(|scope| {
            for round in 0..200 {
                let locks = &locks;
                let (first, second) = if round % 2 == 0 { (a, b) } else { (b, a) };
                scope.spawn(move || drop(locks.lock_all_blocking(&[first, second])));
            }
        });
        // A path named twice is locked once (a second lock would self-deadlock).
        assert_eq!(locks.lock_all_blocking(&[a, a]).len(), 1);
    }
}
