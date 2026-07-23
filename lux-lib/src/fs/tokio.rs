use super::FsError;
#[cfg(unix)]
use std::fs::Permissions;
use std::path::Path;
use tokio::fs;

/// Wrapped [`fs::read`].
pub(crate) async fn read(path: impl AsRef<Path>) -> Result<Vec<u8>, FsError> {
    let path = path.as_ref();
    fs::read(path).await.map_err(|source| FsError::Read {
        path: path.to_path_buf(),
        source,
    })
}

/// Wrapped [`fs::read_to_string`].
pub(crate) async fn read_to_string(path: impl AsRef<Path>) -> Result<String, FsError> {
    let path = path.as_ref();
    fs::read_to_string(path)
        .await
        .map_err(|source| FsError::ReadToString {
            path: path.to_path_buf(),
            source,
        })
}

/// Wrapped [`fs::write`].
pub(crate) async fn write(
    path: impl AsRef<Path>,
    contents: impl AsRef<[u8]>,
) -> Result<(), FsError> {
    let path = path.as_ref();
    fs::write(path, contents)
        .await
        .map_err(|source| FsError::Write {
            path: path.to_path_buf(),
            source,
        })
}

/// Wrapped [`fs::copy`].
pub(crate) async fn copy(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<u64, FsError> {
    let from = from.as_ref();
    let to = to.as_ref();
    fs::copy(from, to).await.map_err(|source| FsError::Copy {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        source,
    })
}

/// Wrapped [`fs::create_dir_all`].
pub(crate) async fn create_dir_all(path: impl AsRef<Path>) -> Result<(), FsError> {
    let path = path.as_ref();
    fs::create_dir_all(path)
        .await
        .map_err(|source| FsError::CreateDirAll {
            path: path.to_path_buf(),
            source,
        })
}

/// Wrapped [`fs::remove_file`].
pub(crate) async fn remove_file(path: impl AsRef<Path>) -> Result<(), FsError> {
    let path = path.as_ref();
    fs::remove_file(path)
        .await
        .map_err(|source| FsError::RemoveFile {
            path: path.to_path_buf(),
            source,
        })
}

/// Wrapped [`fs::remove_dir_all`].
pub(crate) async fn remove_dir_all(path: impl AsRef<Path>) -> Result<(), FsError> {
    let path = path.as_ref();
    fs::remove_dir_all(path)
        .await
        .map_err(|source| FsError::RemoveDirAll {
            path: path.to_path_buf(),
            source,
        })
}

/// Wrapped [`fs::read_dir`].
pub(crate) async fn read_dir(path: impl AsRef<Path>) -> Result<tokio::fs::ReadDir, FsError> {
    let path = path.as_ref();
    fs::read_dir(path).await.map_err(|source| FsError::ReadDir {
        path: path.to_path_buf(),
        source,
    })
}

/// Wrapped [`fs::metadata`].
pub(crate) async fn metadata(path: impl AsRef<Path>) -> Result<std::fs::Metadata, FsError> {
    let path = path.as_ref();
    fs::metadata(path)
        .await
        .map_err(|source| FsError::Metadata {
            path: path.to_path_buf(),
            source,
        })
}

/// Wrapped [`fs::rename`].
pub(crate) async fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<(), FsError> {
    let from = from.as_ref();
    let to = to.as_ref();
    fs::rename(from, to)
        .await
        .map_err(|source| FsError::Rename {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
            source,
        })
}

/// Wrapped [`fs::set_permissions`].
#[cfg(unix)]
pub(crate) async fn set_permissions(
    path: impl AsRef<Path>,
    perm: Permissions,
) -> Result<(), FsError> {
    let path = path.as_ref();
    fs::set_permissions(path, perm)
        .await
        .map_err(|source| FsError::SetPermissions {
            path: path.to_path_buf(),
            source,
        })
}

/// Wrapped [`fs::File::open`].
pub(crate) async fn open(path: impl AsRef<Path>) -> Result<fs::File, FsError> {
    let path = path.as_ref();
    fs::File::open(path)
        .await
        .map_err(|source| FsError::FileOpen {
            path: path.to_path_buf(),
            source,
        })
}

/// Wrapped [`fs::File::create`].
pub(crate) async fn create(path: impl AsRef<Path>) -> Result<fs::File, FsError> {
    let path = path.as_ref();
    fs::File::create(path)
        .await
        .map_err(|source| FsError::FileCreate {
            path: path.to_path_buf(),
            source,
        })
}
