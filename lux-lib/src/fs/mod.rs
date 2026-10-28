use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use miette::Diagnostic;
use thiserror::Error;

pub mod sync;
pub mod tempfile;
pub mod tokio;
#[cfg(unix)]
pub mod unix;
#[cfg(windows)]
pub mod windows;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("failed to read file '{}'", path.display())]
    #[diagnostic(
        code(lux_lib::fs::read),
        help("ensure '{}' exists and is readable", path.display())
    )]
    Read {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to read '{}' as UTF-8 string", path.display())]
    #[diagnostic(
        code(lux_lib::fs::read_to_string),
        help("ensure '{}' exists, is readable, and contains valid UTF-8", path.display())
    )]
    ReadToString {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write to '{}'", path.display())]
    #[diagnostic(
        code(lux_lib::fs::write),
        help("ensure the parent directory of '{}' exists and is writable", path.display())
    )]
    Write {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to copy '{}' to '{}'", from.display(), to.display())]
    #[diagnostic(
        code(lux_lib::fs::copy),
        help("ensure '{}' exists and the parent directory of '{}' is writable", from.display(), to.display())
    )]
    Copy {
        from: std::path::PathBuf,
        to: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to create directory '{}'", path.display())]
    #[diagnostic(
        code(lux_lib::fs::create_dir),
        help("ensure the parent directory of '{}' exists and is writable", path.display())
    )]
    CreateDir {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to create a temporaty directory")]
    #[diagnostic(
        code(lux_lib::fs::create_tempdir),
        help("ensure the '{}' exists and is writable", std::env::temp_dir().display())
    )]
    CreateTempDir { source: std::io::Error },
    #[error("failed to create directory tree '{}'", path.display())]
    #[diagnostic(
        code(lux_lib::fs::create_dir_all),
        help("ensure the path to '{}' is accessible", path.display())
    )]
    CreateDirAll {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to remove file '{}'", path.display())]
    #[diagnostic(
        code(lux_lib::fs::remove_file),
        help("ensure '{}' exists and is writable", path.display())
    )]
    RemoveFile {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to remove directory '{}'", path.display())]
    #[diagnostic(
        code(lux_lib::fs::remove_dir_all),
        help("ensure '{}' exists and is writable", path.display())
    )]
    RemoveDirAll {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to read directory '{}'", path.display())]
    #[diagnostic(
        code(lux_lib::fs::read_dir),
        help("ensure '{}' exists and is a directory", path.display())
    )]
    ReadDir {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to get metadata for '{}'", path.display())]
    #[diagnostic(
        code(lux_lib::fs::metadata),
        help("ensure '{}' exists", path.display())
    )]
    Metadata {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to rename '{}' to '{}'", from.display(), to.display())]
    #[diagnostic(
        code(lux_lib::fs::rename),
        help("ensure '{}' exists and the parent directory of '{}' is writable", from.display(), to.display())
    )]
    Rename {
        from: std::path::PathBuf,
        to: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to set permissions on '{}'", path.display())]
    #[diagnostic(
        code(lux_lib::fs::set_permissions),
        help("ensure '{}' exists and is writable", path.display())
    )]
    SetPermissions {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to open file '{}'", path.display())]
    #[diagnostic(
        code(lux_lib::fs::file_open),
        help("ensure '{}' exists and is readable", path.display())
    )]
    FileOpen {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to create file '{}'", path.display())]
    #[diagnostic(
        code(lux_lib::fs::file_create),
        help("ensure the parent directory of '{}' exists and is writable", path.display())
    )]
    FileCreate {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to get the absolute path for '{}'", path.display())]
    #[diagnostic(code(lux_lib::fs::get_absolute_path))]
    GetAbsolutePath {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to create symlink '{}' pointing to '{}'", link.display(), target.display())]
    #[diagnostic(
        code(lux_lib::fs::symlink),
        help("ensure the parent directory of '{}' exists and is writable", link.display())
    )]
    Symlink {
        target: std::path::PathBuf,
        link: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to create hard link '{}' pointing to '{}'", to.display(), from.display())]
    #[diagnostic(
        code(lux_lib::fs::hard_link),
        help("ensure '{}' is on a filesystem that supports hard links", to.display())
    )]
    HardLink {
        from: std::path::PathBuf,
        to: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("{0}")]
    Other(String),
}

/// Ensures the file at `path` is writable by its owner, so that it can be
/// overwritten on a later install, even when the source is read-only
/// (e.g. a vendored rock in a read-only Nix store).
#[cfg(unix)]
pub(crate) async fn make_writable(path: &Path) -> Result<()> {
    let mut perms = self::tokio::metadata(path).await?.permissions();
    perms.set_mode(perms.mode() | 0o200);
    self::tokio::set_permissions(path, perms).await?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) async fn make_writable(_path: &Path) -> Result<()> {
    Ok(())
}

/// See [`make_writable`].
#[cfg(unix)]
pub(crate) fn make_writable_sync(path: &Path) -> Result<()> {
    let mut perms = std::fs::metadata(path)
        .map_err(|source| Error::Metadata {
            path: path.to_path_buf(),
            source,
        })?
        .permissions();
    perms.set_mode(perms.mode() | 0o200);
    std::fs::set_permissions(path, perms).map_err(|source| Error::SetPermissions {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn make_writable_sync(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
#[tracing::instrument(level = "trace")]
pub(crate) async fn set_executable_permissions(script: &Path) -> Result<()> {
    let mut perms = self::tokio::metadata(script).await?.permissions();
    perms.set_mode(perms.mode() | 0o111);
    self::tokio::set_permissions(script, perms).await?;
    Ok(())
}
