//! The callback the app implements for everything that must go through the
//! platform's file coordination rather than plain `std::fs`.

use mbr_core::vault::VaultError;

/// File operations the app performs on mbr's behalf.
///
/// Implemented in Swift with `NSFileCoordinator` (a `.forReplacing` write
/// through the item-replacement directory, never a move out of
/// `NSTemporaryDirectory()`) and `startDownloadingUbiquitousItem(at:)`. Only
/// writes and downloads cross the boundary: every read stays in Rust on plain
/// `std::fs`, so a scan never calls back once per file.
///
/// Paths are absolute, as the vault's root (the picked folder) plus the file's
/// place in it. Every method is called from one of mbr's background threads,
/// never the main thread, and may block until the operation is done — that is
/// what a coordinated write does anyway.
#[cfg_attr(target_vendor = "apple", uniffi::export(with_foreign))]
pub trait FileCoordinator: Send + Sync {
    /// Replaces (or creates) the file at `path` with `contents`, atomically
    /// where the provider allows it. The parent directory exists.
    fn coordinated_write(&self, path: String, contents: Vec<u8>) -> Result<(), CoordinatorError>;

    /// Creates `path` and any missing parents as directories.
    fn coordinated_create_directory(&self, path: String) -> Result<(), CoordinatorError>;

    /// Moves `from` to `to`, replacing a file at `to`.
    fn coordinated_move(&self, from: String, to: String) -> Result<(), CoordinatorError>;

    /// Deletes the file at `path`.
    fn coordinated_remove(&self, path: String) -> Result<(), CoordinatorError>;

    /// Starts downloading files that are listed but not on the device, and
    /// returns at once. `startDownloadingUbiquitousItem` for iCloud; for a
    /// third-party provider (Dropbox, Working Copy), a coordinated read on a
    /// background queue, which is what makes those materialize. Completion
    /// needs no answer: the files simply become readable.
    fn request_download(&self, paths: Vec<String>);
}

/// Why a coordinated operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(target_vendor = "apple", derive(uniffi::Error))]
pub enum CoordinatorError {
    #[error("not found: {message}")]
    NotFound { message: String },
    #[error("permission denied: {message}")]
    PermissionDenied { message: String },
    #[error("{message}")]
    Failed { message: String },
}

/// A Swift error that is not a `CoordinatorError` (or a panic across the
/// boundary) still has to come back as one.
#[cfg(target_vendor = "apple")]
impl From<uniffi::UnexpectedUniFFICallbackError> for CoordinatorError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed {
            message: error.reason,
        }
    }
}

impl CoordinatorError {
    /// The vault error for a failed operation on `path`.
    pub(crate) fn into_vault_error(self, path: impl std::fmt::Display) -> VaultError {
        let path = path.to_string();
        match self {
            Self::NotFound { .. } => VaultError::NotFound { path },
            Self::PermissionDenied { .. } => VaultError::PermissionDenied { path },
            Self::Failed { message } => VaultError::Io {
                path,
                source: std::io::Error::other(message),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_keep_their_kind_across_the_boundary() {
        let msg = || "x".to_string();
        assert!(
            CoordinatorError::NotFound { message: msg() }
                .into_vault_error("a.md")
                .is_not_found()
        );
        assert!(matches!(
            CoordinatorError::PermissionDenied { message: msg() }.into_vault_error("a.md"),
            VaultError::PermissionDenied { .. }
        ));
        let VaultError::Io { source, .. } = CoordinatorError::Failed {
            message: "disk full".into(),
        }
        .into_vault_error("a.md") else {
            panic!("an I/O error");
        };
        assert_eq!(source.to_string(), "disk full");
    }
}
