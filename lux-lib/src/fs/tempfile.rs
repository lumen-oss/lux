use tempfile::TempDir;

use crate::fs::FsError;

/// Wrapped [`tempfile::tempdir`].
pub(crate) fn tempdir() -> Result<TempDir, FsError> {
    tempfile::tempdir().map_err(|source| FsError::CreateTempDir { source })
}
