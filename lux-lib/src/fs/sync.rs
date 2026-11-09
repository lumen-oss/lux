use std::{
    fs::{self, DirEntry},
    io,
    path::{Path, PathBuf},
};

/// Wrapped [`fs::read_to_string`].
pub(crate) fn read_to_string(path: impl AsRef<Path>) -> Result<String, super::Error> {
    let path = path.as_ref();
    fs::read_to_string(path).map_err(|source| super::Error::ReadToString {
        path: path.to_path_buf(),
        source,
    })
}

/// Wrapped [`fs::write`].
pub(crate) fn write(
    path: impl AsRef<Path>,
    contents: impl AsRef<[u8]>,
) -> Result<(), super::Error> {
    let path = path.as_ref();
    fs::write(path, contents).map_err(|source| super::Error::Write {
        path: path.to_path_buf(),
        source,
    })
}

/// Atomically writes `contents` to `path`.
///
/// The contents are written to a temporary file in the destination directory
/// and then renamed into place, so readers never observe a partially-written
/// file. The temporary file is flushed to disk before the rename.
pub(crate) fn write_atomic(
    path: impl AsRef<Path>,
    contents: impl AsRef<[u8]>,
) -> Result<(), super::Error> {
    use std::io::Write;

    let path = path.as_ref();
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));

    let mut file =
        tempfile::NamedTempFile::new_in(dir).map_err(|source| super::Error::FileCreate {
            path: path.to_path_buf(),
            source,
        })?;
    file.write_all(contents.as_ref())
        .and_then(|()| file.as_file().sync_all())
        .map_err(|source| super::Error::Write {
            path: path.to_path_buf(),
            source,
        })?;
    file.persist(path).map_err(|err| super::Error::Write {
        path: path.to_path_buf(),
        source: err.error,
    })?;
    Ok(())
}

/// Wrapped [`fs::copy`].
pub(crate) fn copy(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<u64, super::Error> {
    let from = from.as_ref();
    let to = to.as_ref();
    fs::copy(from, to).map_err(|source| super::Error::Copy {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        source,
    })
}

/// Wrapped [`fs::hard_link`].
pub(crate) fn hard_link(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<(), super::Error> {
    let from = from.as_ref();
    let to = to.as_ref();
    fs::hard_link(from, to).map_err(|source| super::Error::HardLink {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        source,
    })
}

pub(crate) fn link_file(
    target: impl AsRef<Path>,
    link: impl AsRef<Path>,
) -> Result<(), super::Error> {
    let target = absolute(target)?;
    let link = absolute(link)?;
    prepare_link(&link)?;
    let relative = relative_target(&target, &link);

    match symlink_file(&relative, &link) {
        Ok(()) => Ok(()),
        Err(symlink) => match hard_link(&target, &link) {
            Ok(()) => Ok(()),
            Err(hard_link) => Err(super::Error::Link {
                target,
                link,
                symlink: Box::new(symlink),
                hard_link: Box::new(hard_link),
            }),
        },
    }
}

pub(crate) fn link_dir(
    target: impl AsRef<Path>,
    link: impl AsRef<Path>,
) -> Result<(), super::Error> {
    let target = absolute(target)?;
    let link = absolute(link)?;
    prepare_link(&link)?;
    let relative = relative_target(&target, &link);
    symlink_dir(&relative, &link)
}

pub(crate) fn replace_symlink(
    target: impl AsRef<Path>,
    link: impl AsRef<Path>,
) -> Result<(), super::Error> {
    let target = absolute(target)?;
    let link = absolute(link)?;
    if let Some(parent) = link.parent() {
        create_dir_all(parent)?;
    }

    let file_name = link
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    let temp = link.with_file_name(format!(".{file_name}.tmp"));
    remove_path(&temp)?;

    let relative = relative_target(&target, &temp);
    symlink_dir(&relative, &temp)?;

    #[cfg(unix)]
    {
        rename(&temp, &link)?;
    }
    #[cfg(windows)]
    {
        if link.symlink_metadata().is_ok() {
            remove_link(&link)?;
        }
        rename(&temp, &link)?;
    }

    Ok(())
}

pub(crate) fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<(), super::Error> {
    let from = from.as_ref();
    let to = to.as_ref();
    fs::rename(from, to).map_err(|source| super::Error::Rename {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        source,
    })
}

pub(crate) fn remove_path(path: impl AsRef<Path>) -> Result<(), super::Error> {
    let path = path.as_ref();
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => remove_link(path),
        Ok(metadata) if metadata.is_dir() => remove_dir_all(path),
        Ok(_) => remove_file(path),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(super::Error::Metadata {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn relative_target(target: &Path, link: &Path) -> PathBuf {
    link.parent()
        .and_then(|parent| pathdiff::diff_paths(target, parent))
        .unwrap_or_else(|| target.to_path_buf())
}

fn prepare_link(link: &Path) -> Result<(), super::Error> {
    if let Some(parent) = link.parent() {
        create_dir_all(parent)?;
    }

    match fs::symlink_metadata(link) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            Err(super::Error::Other(format!(
                "cannot replace directory '{}' with a link",
                link.display()
            )))
        }
        Ok(_) => remove_link(link),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(super::Error::Metadata {
            path: link.to_path_buf(),
            source,
        }),
    }
}

#[cfg(unix)]
fn remove_link(link: &Path) -> Result<(), super::Error> {
    remove_file(link)
}

#[cfg(windows)]
fn remove_link(link: &Path) -> Result<(), super::Error> {
    fs::remove_dir(link)
        .or_else(|_| fs::remove_file(link))
        .map_err(|source| super::Error::RemoveFile {
            path: link.to_path_buf(),
            source,
        })
}

#[cfg(unix)]
fn symlink_file(target: &Path, link: &Path) -> Result<(), super::Error> {
    super::unix::symlink(target, link)
}

#[cfg(windows)]
fn symlink_file(target: &Path, link: &Path) -> Result<(), super::Error> {
    super::windows::symlink_file(target, link)
}

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> Result<(), super::Error> {
    super::unix::symlink(target, link)
}

#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> Result<(), super::Error> {
    super::windows::symlink_dir(target, link)
}

/// Recursively hard link the contents of a directory into `dest`.
pub(crate) fn link_dir_all(src: &Path, dest: &Path) -> Result<(), super::Error> {
    create_dir_all(dest)?;
    for entry in read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            link_dir_all(&from, &to)?;
        } else {
            hard_link(&from, &to)?;
        }
    }
    Ok(())
}

/// Wrapped [`fs::create_dir`].
pub(crate) fn create_dir(path: impl AsRef<Path>) -> Result<(), super::Error> {
    let path = path.as_ref();
    fs::create_dir(path).map_err(|source| super::Error::CreateDir {
        path: path.to_path_buf(),
        source,
    })
}

/// Wrapped [`fs::create_dir_all`].
pub(crate) fn create_dir_all(path: impl AsRef<Path>) -> Result<(), super::Error> {
    let path = path.as_ref();
    fs::create_dir_all(path).map_err(|source| super::Error::CreateDirAll {
        path: path.to_path_buf(),
        source,
    })
}

/// Wrapped [`fs::remove_file`].
pub(crate) fn remove_file(path: impl AsRef<Path>) -> Result<(), super::Error> {
    let path = path.as_ref();
    fs::remove_file(path).map_err(|source| super::Error::RemoveFile {
        path: path.to_path_buf(),
        source,
    })
}

/// Wrapped [`fs::remove_dir_all`].
pub(crate) fn remove_dir_all(path: impl AsRef<Path>) -> Result<(), super::Error> {
    let path = path.as_ref();
    fs::remove_dir_all(path).map_err(|source| super::Error::RemoveDirAll {
        path: path.to_path_buf(),
        source,
    })
}

/// Wrapped [`fs::read_dir`].
pub(crate) fn read_dir(
    path: impl AsRef<Path>,
) -> Result<impl Iterator<Item = Result<DirEntry, super::Error>>, super::Error> {
    let path = path.as_ref();
    fs::read_dir(path)
        .map_err(|source| super::Error::ReadDir {
            path: path.to_path_buf(),
            source,
        })
        .map(|entries| {
            let path = path.to_path_buf();

            entries.map(move |entry| {
                entry.map_err(|source| super::Error::ReadDir {
                    path: path.clone(),
                    source,
                })
            })
        })
}

/// Wrapped [`fs::File::open`].
pub(crate) fn open(path: impl AsRef<Path>) -> Result<fs::File, super::Error> {
    let path = path.as_ref();
    fs::File::open(path).map_err(|source| super::Error::FileOpen {
        path: path.to_path_buf(),
        source,
    })
}

/// Wrapped [`fs::File::create`].
pub(crate) fn create(path: impl AsRef<Path>) -> Result<fs::File, super::Error> {
    let path = path.as_ref();
    fs::File::create(path).map_err(|source| super::Error::FileCreate {
        path: path.to_path_buf(),
        source,
    })
}

/// Wrapped [`std::path::absolute`].
pub(crate) fn absolute(path: impl AsRef<Path>) -> Result<PathBuf, super::Error> {
    let path = path.as_ref();
    std::path::absolute(path).map_err(|source| super::Error::GetAbsolutePath {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use assert_fs::prelude::*;

    use super::*;

    #[test]
    fn link_file_replaces_existing_link() {
        let temp = assert_fs::TempDir::new().unwrap();
        let first = temp.child("first");
        let second = temp.child("second");
        let link = temp.child("link");
        first.write_str("first").unwrap();
        second.write_str("second").unwrap();

        link_file(first.path(), link.path()).unwrap();
        assert_eq!(read_to_string(link.path()).unwrap(), "first");

        link_file(second.path(), link.path()).unwrap();
        assert_eq!(read_to_string(link.path()).unwrap(), "second");
    }

    #[cfg(unix)]
    #[test]
    fn replace_symlink_swaps_target() {
        let temp = assert_fs::TempDir::new().unwrap();
        let first = temp.child("first");
        let second = temp.child("second");
        let link = temp.child("link");
        first.create_dir_all().unwrap();
        second.create_dir_all().unwrap();
        first.child("first.txt").write_str("first").unwrap();
        second.child("second.txt").write_str("second").unwrap();

        replace_symlink(first.path(), link.path()).unwrap();
        assert_eq!(
            read_to_string(link.child("first.txt").path()).unwrap(),
            "first"
        );

        replace_symlink(second.path(), link.path()).unwrap();
        assert_eq!(
            read_to_string(link.child("second.txt").path()).unwrap(),
            "second"
        );
    }

    #[cfg(unix)]
    #[test]
    fn link_dir_replaces_existing_link() {
        let temp = assert_fs::TempDir::new().unwrap();
        let first = temp.child("first");
        let second = temp.child("second");
        let link = temp.child("link");
        first.create_dir_all().unwrap();
        second.create_dir_all().unwrap();
        first.child("first.txt").write_str("first").unwrap();
        second.child("second.txt").write_str("second").unwrap();

        link_dir(first.path(), link.path()).unwrap();
        assert_eq!(
            read_to_string(link.child("first.txt").path()).unwrap(),
            "first"
        );

        link_dir(second.path(), link.path()).unwrap();
        assert_eq!(
            read_to_string(link.child("second.txt").path()).unwrap(),
            "second"
        );
    }
}
