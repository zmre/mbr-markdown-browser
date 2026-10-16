//! The dataless-file I/O policy of the mobile runtime's threads.
//!
//! On Apple platforms a read of a *dataless* file — one a file provider (iCloud
//! Drive, Dropbox, …) lists but has not downloaded — blocks while the system
//! downloads it, possibly for seconds, possibly forever when offline. A scan
//! of 4.8k files must never do that. `setiopolicy_np` with
//! `IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES` set to `OFF` makes such a read
//! fail at once with `EDEADLK` instead, which `mbr_core::vault::VaultError`
//! maps to `NotDownloaded` (the scan lists the file without frontmatter, a
//! page answers "downloading", search skips it).
//!
//! The policy is set **per thread** (`IOPOL_SCOPE_THREAD`), on exactly the
//! threads mbr's mobile runtime owns: every tokio worker and blocking thread
//! ([`super::runtime`]'s `on_thread_start`) and every thread of the rayon pool
//! the scan runs on (its `start_handler`). Swift's threads, and the rest of the
//! process, keep the system default.
//!
//! [`set_materialize_dataless_files`] is the debug toggle for comparing both
//! behaviours on a device: it decides what threads started *afterwards* get.
//! Tokio threads are per vault, so it applies to the next vault opened; the
//! rayon pool is the process's and starts once, so for the scan it applies
//! from the next app launch.
//!
//! **Directory listings are the exception** ([`with_materialization`]):
//! `CoordinatedVault::list_dir` turns materialization *on* for the calling
//! thread for the length of one listing. A file provider's folder can be
//! *dataless* too — iCloud Drive on iOS fetches a folder's entries only when
//! something lists it with materialization allowed — and with it off, `readdir`
//! does not fail: it silently returns only the entries already on the device
//! (seen on an iPhone: a vault root listing 5 notes and 1 of ~30 subfolders).
//! Materializing a directory fetches its entries — names and metadata — never
//! a file's contents, so the listing may wait while reads still fail fast.
//! This is what Foundation does: the kernel's own comment on the per-thread
//! flag (`vfs_context_dataless_materialization_is_prevented`, xnu
//! `bsd/vfs/vfs_syscalls.c`) says "Foundation uses this", and the same check
//! guards `vfs_materialize_dir` and file materialization alike — there is no
//! directory-only policy (`getiopolicy_np(3)`), hence the scoped switch.
//!
//! Everything here is a no-op off Apple platforms.

use std::sync::atomic::{AtomicBool, Ordering};

/// `true`: reads of dataless files on mbr's threads wait for the download
/// (the system default). `false` (mbr's default): they fail fast.
static MATERIALIZE: AtomicBool = AtomicBool::new(false);

/// Sets whether threads mbr starts from now on wait for dataless files to
/// download (`true`) or fail fast (`false`, the default). A debug toggle for
/// timing the two on a device; see the module docs for when it takes effect.
#[cfg_attr(target_vendor = "apple", uniffi::export)]
pub fn set_materialize_dataless_files(materialize: bool) {
    MATERIALIZE.store(materialize, Ordering::Relaxed);
}

/// The current setting of [`set_materialize_dataless_files`].
#[cfg_attr(target_vendor = "apple", uniffi::export)]
pub fn materializes_dataless_files() -> bool {
    MATERIALIZE.load(Ordering::Relaxed)
}

/// Applies the current setting to the calling thread. Called from thread-start
/// hooks; a failure is logged and leaves the system default.
pub(crate) fn apply_to_current_thread() {
    #[cfg(target_vendor = "apple")]
    if let Err(e) = sys::set_thread_policy(materializes_dataless_files()) {
        tracing::warn!("setiopolicy_np on an mbr thread failed: {e}");
    }
}

/// Runs `f` with materialization of dataless items **on** for the calling
/// thread, then puts the thread's previous policy back — also when `f`
/// panics. For directory listings only (see the module docs): `f` must not
/// read file contents. Off Apple platforms it just runs `f`.
pub(crate) fn with_materialization<T>(f: impl FnOnce() -> T) -> T {
    #[cfg(target_vendor = "apple")]
    let _restore = sys::ThreadPolicyGuard::materializing();
    f()
}

/// Whether the calling thread's own policy materializes dataless files, or
/// `None` when it has none of its own (it follows the process). Apple only.
#[cfg(all(test, target_vendor = "apple"))]
pub(crate) fn current_thread_materializes() -> Option<bool> {
    sys::thread_policy().ok().flatten()
}

/// `setiopolicy_np(3)`. Not in the `libc` crate, so declared here; the values
/// are `<sys/resource.h>`'s.
#[cfg(target_vendor = "apple")]
mod sys {
    use std::ffi::c_int;
    use std::io;

    const IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES: c_int = 3;
    const IOPOL_SCOPE_THREAD: c_int = 1;
    const IOPOL_MATERIALIZE_DATALESS_FILES_DEFAULT: c_int = 0;
    const IOPOL_MATERIALIZE_DATALESS_FILES_OFF: c_int = 1;
    const IOPOL_MATERIALIZE_DATALESS_FILES_ON: c_int = 2;
    #[cfg(test)]
    const IOPOL_MATERIALIZE_DATALESS_FILES_BASIC_MASK: c_int = 3;

    unsafe extern "C" {
        fn setiopolicy_np(iotype: c_int, scope: c_int, policy: c_int) -> c_int;
        fn getiopolicy_np(iotype: c_int, scope: c_int) -> c_int;
    }

    pub(super) fn set_thread_policy(materialize: bool) -> io::Result<()> {
        set_raw_thread_policy(if materialize {
            IOPOL_MATERIALIZE_DATALESS_FILES_ON
        } else {
            IOPOL_MATERIALIZE_DATALESS_FILES_OFF
        })
    }

    /// The calling thread's own policy value: `OFF`, `ON` or `DEFAULT`
    /// (follows the process).
    fn raw_thread_policy() -> io::Result<c_int> {
        // SAFETY: a system call taking two integers and returning one.
        let rc = unsafe {
            getiopolicy_np(
                IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
                IOPOL_SCOPE_THREAD,
            )
        };
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(rc & 3)
        }
    }

    /// Materialization on for the calling thread while it lives; the
    /// thread's previous value back on drop. Never sent to another thread
    /// (the policy is per thread), which the raw-pointer marker enforces.
    pub(super) struct ThreadPolicyGuard {
        previous: Option<c_int>,
        _not_send: std::marker::PhantomData<*const ()>,
    }

    impl ThreadPolicyGuard {
        pub(super) fn materializing() -> Self {
            let previous = match raw_thread_policy() {
                Ok(previous) if previous == IOPOL_MATERIALIZE_DATALESS_FILES_ON => None,
                Ok(previous) => match set_raw_thread_policy(IOPOL_MATERIALIZE_DATALESS_FILES_ON) {
                    Ok(()) => Some(previous),
                    Err(e) => {
                        tracing::warn!("setiopolicy_np(materialize on) failed: {e}");
                        None
                    }
                },
                Err(e) => {
                    tracing::warn!("getiopolicy_np failed: {e}");
                    None
                }
            };
            Self {
                previous,
                _not_send: std::marker::PhantomData,
            }
        }
    }

    impl Drop for ThreadPolicyGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous
                && let Err(e) = set_raw_thread_policy(previous)
            {
                tracing::warn!("setiopolicy_np(restore {previous}) failed: {e}");
            }
        }
    }

    fn set_raw_thread_policy(policy: c_int) -> io::Result<()> {
        debug_assert!(matches!(
            policy,
            IOPOL_MATERIALIZE_DATALESS_FILES_DEFAULT
                | IOPOL_MATERIALIZE_DATALESS_FILES_OFF
                | IOPOL_MATERIALIZE_DATALESS_FILES_ON
        ));
        // SAFETY: a system call taking three integers; no pointers, no state
        // beyond the calling thread's I/O policy.
        let rc = unsafe {
            setiopolicy_np(
                IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
                IOPOL_SCOPE_THREAD,
                policy,
            )
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[cfg(test)]
    pub(super) fn thread_policy() -> io::Result<Option<bool>> {
        Ok(
            match raw_thread_policy()? & IOPOL_MATERIALIZE_DATALESS_FILES_BASIC_MASK {
                IOPOL_MATERIALIZE_DATALESS_FILES_OFF => Some(false),
                IOPOL_MATERIALIZE_DATALESS_FILES_ON => Some(true),
                IOPOL_MATERIALIZE_DATALESS_FILES_DEFAULT => None,
                _ => None,
            },
        )
    }
}

#[cfg(all(test, target_vendor = "apple"))]
mod tests {
    use super::*;

    #[test]
    fn the_thread_policy_follows_the_toggle_and_stays_on_its_thread() {
        // A fresh thread each time: the policy is the thread's own.
        let on_thread = |materialize: bool| {
            std::thread::spawn(move || {
                let before = current_thread_materializes();
                set_materialize_dataless_files(materialize);
                apply_to_current_thread();
                (before, current_thread_materializes())
            })
            .join()
            .unwrap()
        };
        let original = materializes_dataless_files();
        assert_eq!(on_thread(false), (None, Some(false)), "fail fast");
        assert_eq!(on_thread(true), (None, Some(true)), "wait for downloads");
        set_materialize_dataless_files(original);
        assert_eq!(current_thread_materializes(), None, "this thread untouched");
    }

    #[test]
    fn a_listing_materializes_and_puts_the_thread_policy_back() {
        let run = |start: Option<bool>| {
            std::thread::spawn(move || {
                if let Some(materialize) = start {
                    sys::set_thread_policy(materialize).unwrap();
                }
                let inside = with_materialization(current_thread_materializes);
                (inside, current_thread_materializes())
            })
            .join()
            .unwrap()
        };
        assert_eq!(
            run(Some(false)),
            (Some(true), Some(false)),
            "fail-fast thread"
        );
        assert_eq!(
            run(None),
            (Some(true), None),
            "a thread with no policy of its own"
        );
        assert_eq!(run(Some(true)), (Some(true), Some(true)));

        // Restored on unwind too.
        let after_panic = std::thread::spawn(|| {
            sys::set_thread_policy(false).unwrap();
            let caught = std::panic::catch_unwind(|| with_materialization(|| panic!("boom")));
            assert!(caught.is_err());
            current_thread_materializes()
        })
        .join()
        .unwrap();
        assert_eq!(after_panic, Some(false));
    }
}
