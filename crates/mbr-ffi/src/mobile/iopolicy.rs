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
    #[cfg(test)]
    const IOPOL_MATERIALIZE_DATALESS_FILES_DEFAULT: c_int = 0;
    const IOPOL_MATERIALIZE_DATALESS_FILES_OFF: c_int = 1;
    const IOPOL_MATERIALIZE_DATALESS_FILES_ON: c_int = 2;
    #[cfg(test)]
    const IOPOL_MATERIALIZE_DATALESS_FILES_BASIC_MASK: c_int = 3;

    unsafe extern "C" {
        fn setiopolicy_np(iotype: c_int, scope: c_int, policy: c_int) -> c_int;
        #[cfg(test)]
        fn getiopolicy_np(iotype: c_int, scope: c_int) -> c_int;
    }

    pub(super) fn set_thread_policy(materialize: bool) -> io::Result<()> {
        let policy = if materialize {
            IOPOL_MATERIALIZE_DATALESS_FILES_ON
        } else {
            IOPOL_MATERIALIZE_DATALESS_FILES_OFF
        };
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
        // SAFETY: as above, two integers in, one out.
        let rc = unsafe {
            getiopolicy_np(
                IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
                IOPOL_SCOPE_THREAD,
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(match rc & IOPOL_MATERIALIZE_DATALESS_FILES_BASIC_MASK {
            IOPOL_MATERIALIZE_DATALESS_FILES_OFF => Some(false),
            IOPOL_MATERIALIZE_DATALESS_FILES_ON => Some(true),
            IOPOL_MATERIALIZE_DATALESS_FILES_DEFAULT => None,
            _ => None,
        })
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
}
