//! Errors owned by the HTTP server.
//!
//! Everything else the server can fail with is a core error
//! ([`mbr_core::errors`]); only what needs the server's own types lives here.

use mbr_core::errors::{MbrError, TemplateError};
use thiserror::Error;

/// Errors related to the HTTP server.
#[derive(Debug, Error)]
pub enum ServerError {
    #[error("Failed to bind to {addr}")]
    BindFailed {
        addr: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Server failed to start")]
    StartFailed(#[source] std::io::Error),

    #[error("Failed to get local address")]
    LocalAddrFailed(#[source] std::io::Error),

    #[error("Template initialization failed: {0}")]
    TemplateInit(#[from] TemplateError),

    #[error("Tracing initialization failed")]
    TracingInit,
}

/// Lets `?` lift a [`ServerError`] into [`MbrError::Server`], as the derived
/// `#[from]` did when both enums lived in one crate.
impl From<ServerError> for MbrError {
    fn from(err: ServerError) -> Self {
        MbrError::Server(Box::new(err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error as IoError, ErrorKind};

    #[test]
    fn test_server_error_display() {
        let err = ServerError::BindFailed {
            addr: "127.0.0.1:8080".to_string(),
            source: IoError::new(ErrorKind::AddrInUse, "address in use"),
        };
        assert!(err.to_string().contains("127.0.0.1:8080"));
        assert!(err.to_string().contains("Failed to bind"));
    }

    #[test]
    fn test_server_error_to_mbr_error() {
        let server_err = ServerError::TracingInit;
        let mbr_err: MbrError = server_err.into();

        match &mbr_err {
            MbrError::Server(inner) => assert!(matches!(
                inner.downcast_ref::<ServerError>(),
                Some(ServerError::TracingInit)
            )),
            _ => panic!("Expected MbrError::Server(TracingInit), got {:?}", mbr_err),
        }
    }

    /// The message is the one the single-crate `#[from]` variant printed:
    /// `MbrError`'s prefix plus the inner error's own Display.
    #[test]
    fn test_server_error_message_through_mbr_error() {
        let mbr_err: MbrError = ServerError::TracingInit.into();
        assert_eq!(
            mbr_err.to_string(),
            "Server error: Tracing initialization failed"
        );
    }
}
