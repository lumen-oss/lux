use std::{fs, path::Path};

use super::FsError;

/// Wrapped [`fs::read_to_string`].
pub(crate) fn read_to_string(path: impl AsRef<Path>) -> Result<String, FsError> {
    let path = path.as_ref();
    fs::read_to_string(path).map_err(|source| FsError::ReadToString {
        path: path.to_path_buf(),
        source,
    })
}

/// Wrapped [`fs::write`].
pub(crate) fn write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> Result<(), FsError> {
    let path = path.as_ref();
    fs::write(path, contents).map_err(|source| FsError::Write {
        path: path.to_path_buf(),
        source,
    })
}

/// Wrapped [`fs::copy`].
pub(crate) fn copy(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<u64, FsError> {
    let from = from.as_ref();
    let to = to.as_ref();
    fs::copy(from, to).map_err(|source| FsError::Copy {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        source,
    })
}

/// Wrapped [`fs::create_dir_all`].
pub(crate) fn create_dir_all(path: impl AsRef<Path>) -> Result<(), FsError> {
    let path = path.as_ref();
    fs::create_dir_all(path).map_err(|source| FsError::CreateDirAll {
        path: path.to_path_buf(),
        source,
    })
}

/// Wrapped [`fs::remove_file`].
pub(crate) fn remove_file(path: impl AsRef<Path>) -> Result<(), FsError> {
    let path = path.as_ref();
    fs::remove_file(path).map_err(|source| FsError::RemoveFile {
        path: path.to_path_buf(),
        source,
    })
}

/// Wrapped [`fs::remove_dir_all`].
pub(crate) fn remove_dir_all(path: impl AsRef<Path>) -> Result<(), FsError> {
    let path = path.as_ref();
    fs::remove_dir_all(path).map_err(|source| FsError::RemoveDirAll {
        path: path.to_path_buf(),
        source,
    })
}

/// Wrapped [`fs::read_dir`].
pub(crate) fn read_dir(path: impl AsRef<Path>) -> Result<fs::ReadDir, FsError> {
    let path = path.as_ref();
    fs::read_dir(path).map_err(|source| FsError::ReadDir {
        path: path.to_path_buf(),
        source,
    })
}

/// Wrapped [`fs::File::open`].
pub(crate) fn open(path: impl AsRef<Path>) -> Result<fs::File, FsError> {
    let path = path.as_ref();
    fs::File::open(path).map_err(|source| FsError::FileOpen {
        path: path.to_path_buf(),
        source,
    })
}

/// Wrapped [`fs::File::create`].
pub(crate) fn create(path: impl AsRef<Path>) -> Result<fs::File, FsError> {
    let path = path.as_ref();
    fs::File::create(path).map_err(|source| FsError::FileCreate {
        path: path.to_path_buf(),
        source,
    })
}
