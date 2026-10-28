use tempfile::TempDir;

/// Wrapped [`tempfile::tempdir`].
pub(crate) fn tempdir() -> Result<TempDir, super::Error> {
    tempfile::tempdir().map_err(|source| super::Error::CreateTempDir { source })
}
