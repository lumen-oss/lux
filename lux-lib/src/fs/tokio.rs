#[cfg(unix)]
use std::fs::Permissions;
use std::path::Path;
use tokio::fs;

/// Wrapped [`fs::read`].
pub(crate) async fn read(path: impl AsRef<Path>) -> Result<Vec<u8>, super::Error> {
    let path = path.as_ref();
    fs::read(path).await.map_err(|source| super::Error::Read {
        path: path.to_path_buf(),
        source,
    })
}

/// Wrapped [`fs::read_to_string`].
pub(crate) async fn read_to_string(path: impl AsRef<Path>) -> Result<String, super::Error> {
    let path = path.as_ref();
    fs::read_to_string(path)
        .await
        .map_err(|source| super::Error::ReadToString {
            path: path.to_path_buf(),
            source,
        })
}

/// Wrapped [`fs::write`].
pub(crate) async fn write(
    path: impl AsRef<Path>,
    contents: impl AsRef<[u8]>,
) -> Result<(), super::Error> {
    let path = path.as_ref();
    fs::write(path, contents)
        .await
        .map_err(|source| super::Error::Write {
            path: path.to_path_buf(),
            source,
        })
}

/// Wrapped [`fs::copy`].
pub(crate) async fn copy(
    from: impl AsRef<Path>,
    to: impl AsRef<Path>,
) -> Result<u64, super::Error> {
    let from = from.as_ref();
    let to = to.as_ref();
    fs::copy(from, to)
        .await
        .map_err(|source| super::Error::Copy {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
            source,
        })
}

/// Wrapped [`fs::create_dir_all`].
pub(crate) async fn create_dir_all(path: impl AsRef<Path>) -> Result<(), super::Error> {
    let path = path.as_ref();
    fs::create_dir_all(path)
        .await
        .map_err(|source| super::Error::CreateDirAll {
            path: path.to_path_buf(),
            source,
        })
}

/// Recursively copy the contents of a directory into `dest`.
pub(crate) async fn copy_dir_all(src: &Path, dest: &Path) -> Result<(), super::Error> {
    Box::pin(async move {
        create_dir_all(dest).await?;
        let mut entries = read_dir(src).await?;
        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|source| super::Error::ReadDir {
                    path: src.to_path_buf(),
                    source,
                })?
        {
            let entry_path = entry.path();
            let target = dest.join(entry.file_name());
            if entry_path.is_dir() {
                copy_dir_all(&entry_path, &target).await?;
            } else {
                copy(&entry_path, &target).await?;
            }
        }
        Ok(())
    })
    .await
}

/// Wrapped [`fs::remove_file`].
pub(crate) async fn remove_file(path: impl AsRef<Path>) -> Result<(), super::Error> {
    let path = path.as_ref();
    fs::remove_file(path)
        .await
        .map_err(|source| super::Error::RemoveFile {
            path: path.to_path_buf(),
            source,
        })
}

/// Wrapped [`fs::remove_dir_all`].
pub(crate) async fn remove_dir_all(path: impl AsRef<Path>) -> Result<(), super::Error> {
    let path = path.as_ref();
    fs::remove_dir_all(path)
        .await
        .map_err(|source| super::Error::RemoveDirAll {
            path: path.to_path_buf(),
            source,
        })
}

/// Wrapped [`fs::read_dir`].
pub(crate) async fn read_dir(path: impl AsRef<Path>) -> Result<tokio::fs::ReadDir, super::Error> {
    let path = path.as_ref();
    fs::read_dir(path)
        .await
        .map_err(|source| super::Error::ReadDir {
            path: path.to_path_buf(),
            source,
        })
}

/// Wrapped [`fs::metadata`].
pub(crate) async fn metadata(path: impl AsRef<Path>) -> Result<std::fs::Metadata, super::Error> {
    let path = path.as_ref();
    fs::metadata(path)
        .await
        .map_err(|source| super::Error::Metadata {
            path: path.to_path_buf(),
            source,
        })
}

/// Wrapped [`fs::rename`].
pub(crate) async fn rename(
    from: impl AsRef<Path>,
    to: impl AsRef<Path>,
) -> Result<(), super::Error> {
    let from = from.as_ref();
    let to = to.as_ref();
    fs::rename(from, to)
        .await
        .map_err(|source| super::Error::Rename {
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
) -> Result<(), super::Error> {
    let path = path.as_ref();
    fs::set_permissions(path, perm)
        .await
        .map_err(|source| super::Error::SetPermissions {
            path: path.to_path_buf(),
            source,
        })
}

/// Wrapped [`fs::File::open`].
pub(crate) async fn open(path: impl AsRef<Path>) -> Result<fs::File, super::Error> {
    let path = path.as_ref();
    fs::File::open(path)
        .await
        .map_err(|source| super::Error::FileOpen {
            path: path.to_path_buf(),
            source,
        })
}

/// Wrapped [`fs::File::create`].
pub(crate) async fn create(path: impl AsRef<Path>) -> Result<fs::File, super::Error> {
    let path = path.as_ref();
    fs::File::create(path)
        .await
        .map_err(|source| super::Error::FileCreate {
            path: path.to_path_buf(),
            source,
        })
}
