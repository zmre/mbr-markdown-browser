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
