//! The threads an open vault runs on.
//!
//! Each [`super::MbrApp`] owns a small tokio runtime: two workers (rendering is
//! short; anything blocking goes to the blocking pool) and at most eight
//! blocking threads (file reads, search, the scan's driver). The repository
//! scan fans out on rayon's global pool, which this module sizes and starts
//! once per process. Every one of those threads gets the dataless-file policy
//! ([`super::iopolicy`]) as it starts, so no mbr thread waits on a cloud
//! download unless asked to.
//!
//! The global pool rather than a dedicated one: the scan, content search's
//! callers and the index builds all use rayon's `par_iter` through the global
//! pool, deep inside `mbr_core`, and a dedicated pool would only cover code run
//! inside its `install`. In the app the process is mbr's, so the global pool is
//! too. When something else already started it (a test binary), the pool is
//! left as it is and logged.

use std::io;
use std::sync::Once;

use super::iopolicy;

/// Tokio worker threads per open vault.
const WORKER_THREADS: usize = 2;

/// Upper bound on each vault's blocking pool.
const MAX_BLOCKING_THREADS: usize = 8;

/// Upper bound on the rayon pool (the scan's frontmatter reads).
const MAX_RAYON_THREADS: usize = 4;

/// The runtime for one open vault.
pub(crate) fn build() -> io::Result<tokio::runtime::Runtime> {
    start_rayon_pool();
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(WORKER_THREADS)
        .max_blocking_threads(MAX_BLOCKING_THREADS)
        .thread_name("mbr-mobile")
        .on_thread_start(iopolicy::apply_to_current_thread)
        .enable_all()
        .build()
}

/// Starts rayon's global pool with mbr's size and thread-start policy, once.
fn start_rayon_pool() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let threads = std::thread::available_parallelism()
            .map_or(2, |n| n.get())
            .min(MAX_RAYON_THREADS);
        if let Err(e) = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("mbr-rayon-{i}"))
            .start_handler(|_| iopolicy::apply_to_current_thread())
            .build_global()
        {
            tracing::info!("rayon's pool was already started; keeping it: {e}");
        }
    });
}

/// A tokio runtime that shuts down without waiting.
///
/// Dropping a plain `Runtime` blocks until every `spawn_blocking` task ends,
/// and the scan may still be running. The last reference to an `MbrApp` is
/// dropped on a Swift thread, often the main one, which must not block.
pub(crate) struct BackgroundRuntime(Option<tokio::runtime::Runtime>);

impl BackgroundRuntime {
    pub(crate) fn new(runtime: tokio::runtime::Runtime) -> Self {
        Self(Some(runtime))
    }
}

impl std::ops::Deref for BackgroundRuntime {
    type Target = tokio::runtime::Runtime;

    fn deref(&self) -> &Self::Target {
        self.0
            .as_ref()
            .expect("the runtime is present until BackgroundRuntime is dropped")
    }
}

impl Drop for BackgroundRuntime {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_background();
        }
    }
}

#[cfg(all(test, target_vendor = "apple"))]
mod tests {
    #[test]
    fn runtime_threads_fail_fast_on_dataless_files() {
        let runtime = super::build().unwrap();
        let policy = runtime
            .block_on(async {
                tokio::task::spawn_blocking(super::iopolicy::current_thread_materializes).await
            })
            .unwrap();
        // Another test may flip the global toggle concurrently; what matters
        // is that the thread carries a policy of its own.
        assert!(policy.is_some(), "blocking threads get mbr's policy");
        let worker = runtime
            .block_on(async {
                tokio::spawn(async { super::iopolicy::current_thread_materializes() }).await
            })
            .unwrap();
        assert!(worker.is_some(), "workers too");
    }
}
