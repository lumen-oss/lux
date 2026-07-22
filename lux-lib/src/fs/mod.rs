use miette::Diagnostic;
use thiserror::Error;

pub mod sync;
pub mod tempfile;
pub mod tokio;

#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum FsError {
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
}
