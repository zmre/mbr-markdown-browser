//! Test-only helpers shared across unit-test modules.
//!
//! Compiled only under `cfg(test)`, so nothing here reaches a release binary.

use std::io::Write;
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;

/// A `tracing` writer that appends everything to a shared buffer.
#[derive(Clone)]
struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // A poisoned mutex here means another test thread panicked mid-write;
        // there is nothing useful to recover, and returning an error would mask
        // that panic behind a confusing I/O failure.
        let mut guard = self.0.lock().expect("log buffer mutex poisoned");
        guard.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for SharedBuffer {
    type Writer = SharedBuffer;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Runs `body` with `tracing` output captured, returning its value alongside the
/// captured log text.
///
/// Used to assert that a diagnostic actually names the file (or note) it is
/// about — the whole point of several warnings in this crate is *which* file is
/// broken, and only reading the emitted line can prove that.
///
/// The subscriber is installed for the calling thread only
/// (`tracing::subscriber::with_default`), so tests running in parallel do not
/// capture each other's output. `body` must therefore do its logging on this
/// thread: synchronous code, or an async block driven to completion here.
pub(crate) fn capture_tracing<T>(body: impl FnOnce() -> T) -> (T, String) {
    let buffer = SharedBuffer(Arc::new(Mutex::new(Vec::new())));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buffer.clone())
        .with_ansi(false)
        .finish();
    let value = tracing::subscriber::with_default(subscriber, body);
    let logs =
        String::from_utf8_lossy(&buffer.0.lock().expect("log buffer mutex poisoned")).into_owned();
    (value, logs)
}
