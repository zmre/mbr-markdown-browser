//! Error types for the `mbr` application crate.
//!
//! Re-exports every core error ([`mbr_core::errors`]) and the server's
//! [`ServerError`], so `mbr::errors::X` names the same types it did before the
//! workspace split, and adds the GUI-only errors, which need `tao`/`wry`.

pub use mbr_core::errors::*;
#[cfg(feature = "server")]
pub use mbr_server::errors::ServerError;

#[cfg(feature = "gui")]
use thiserror::Error;

/// Errors related to the browser/GUI window.
#[cfg(feature = "gui")]
#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("Failed to create window")]
    WindowCreationFailed(#[source] tao::error::OsError),

    #[error("Failed to create webview")]
    WebViewCreationFailed(#[source] wry::Error),

    #[error("Failed to load icon")]
    IconLoadFailed(String),

    #[error("Failed to create icon from RGBA data")]
    IconCreationFailed(#[source] tao::window::BadIcon),

    #[error("Server failed to start for new folder")]
    ServerStartFailed,
}

/// Why handing a URL to the operating system's default handler failed.
///
/// The reason is carried as a `String` rather than the platform error type so
/// this enum stays free of `cfg` noise: the three back ends in
/// [`crate::external_open`] fail with an `NSWorkspace` bool, a Win32 status code
/// and a `glib::Error` respectively, and nothing downstream does more than log.
#[cfg(feature = "gui")]
#[derive(Debug, Error)]
pub enum ExternalOpenError {
    /// The fail-closed refusal: no GUI window is running in this process, so
    /// nothing was handed to the operating system.
    ///
    /// Not a failure to launch — nothing was attempted. Launching applications
    /// is a thing an *interactive window* does on behalf of the person sitting
    /// in front of it. A process answering HTTP has no such person, and must
    /// never be induced to start applications on its host, so
    /// [`crate::external_open::open_external`] refuses before it touches the OS.
    /// The `gui` feature is on by default, so a server-mode process still
    /// *contains* the launcher; this is what keeps it unreachable.
    #[error(
        "Refusing to open {url}: handing URLs to the operating system is GUI-only, \
         and no GUI window is running in this process"
    )]
    GuiOnly { url: String },

    #[error("The system URL parser rejected {url}")]
    Malformed { url: String },

    #[error("The system refused to open {url}: {reason}")]
    LaunchFailed { url: String, reason: String },
}

/// Why a PDF export (`mbr --pdf`, the GUI's Save as PDF…) did not produce a
/// file. See [`crate::pdf_export`].
///
/// Messages name the cause and the path, because the CLI prints them as they
/// are and the GUI shows them in a dialog.
#[cfg(feature = "gui")]
#[derive(Debug, Error)]
pub enum PdfError {
    /// This platform has no export backend yet.
    #[error("PDF export is not available on {platform} yet")]
    Unsupported { platform: &'static str },

    /// No display to run a webview on (Linux without X11/Wayland).
    #[error("{0}")]
    NoDisplay(String),

    #[error("The folder for the PDF does not exist: {}", dir.display())]
    OutputDirMissing { dir: std::path::PathBuf },

    #[error("Not a folder: {}", dir.display())]
    OutputDirNotDirectory { dir: std::path::PathBuf },

    #[error("Cannot write to the folder {}", dir.display())]
    OutputDirNotWritable {
        dir: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Could not start the server for the export: {0}")]
    Server(String),

    #[error("Could not create the export window")]
    Window(#[source] tao::error::OsError),

    #[error("Could not create the export webview")]
    WebView(#[source] wry::Error),

    /// The page reported that it could not prepare itself for paper.
    #[error("The page could not be prepared for export: {message}")]
    PageFailed { message: String },

    #[error("Printing to PDF failed: {message}")]
    PrintFailed { message: String },

    /// The print system reported success but left no file (macOS does this
    /// for a folder that vanished).
    #[error("No PDF was written to {}", path.display())]
    NotWritten { path: std::path::PathBuf },

    #[error("The PDF written to {} is empty", path.display())]
    Empty { path: std::path::PathBuf },

    #[error("The file written to {} is not a PDF", path.display())]
    NotAPdf { path: std::path::PathBuf },

    #[error("Could not move the PDF into place at {}", path.display())]
    Finalize {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Could not write the PDF to standard output")]
    Stdout(#[source] std::io::Error),
}

#[cfg(feature = "gui")]
impl PdfError {
    /// Refused before the export started — no backend, no display, an
    /// output folder that is missing, not a folder or not writable, or a
    /// server that would not start: the CLI's exit code 2 rather than 1.
    /// Everything from creating the export window on is a failure (1).
    pub fn is_refusal(&self) -> bool {
        matches!(
            self,
            PdfError::Unsupported { .. }
                | PdfError::NoDisplay(_)
                | PdfError::OutputDirMissing { .. }
                | PdfError::OutputDirNotDirectory { .. }
                | PdfError::OutputDirNotWritable { .. }
                | PdfError::Server(_)
        )
    }
}

/// Lets `?` lift a [`BrowserError`] into [`MbrError::Browser`], as the derived
/// `#[from]` did when both enums lived in one crate.
#[cfg(feature = "gui")]
impl From<BrowserError> for MbrError {
    fn from(err: BrowserError) -> Self {
        MbrError::Browser(Box::new(err))
    }
}

#[cfg(all(test, feature = "gui"))]
mod tests {
    use super::*;

    #[test]
    fn browser_error_keeps_its_message_through_mbr_error() {
        let mbr_err: MbrError = BrowserError::ServerStartFailed.into();
        assert_eq!(
            mbr_err.to_string(),
            "Browser error: Server failed to start for new folder"
        );
        match &mbr_err {
            MbrError::Browser(inner) => assert!(matches!(
                inner.downcast_ref::<BrowserError>(),
                Some(BrowserError::ServerStartFailed)
            )),
            _ => panic!("Expected MbrError::Browser, got {mbr_err:?}"),
        }
    }
}
