use std::path::Path;

use super::FsError;

/// Wrapped [`std::os::windows::fs::symlink_dir`].
pub(crate) fn symlink_dir(target: impl AsRef<Path>, link: impl AsRef<Path>) -> Result<(), FsError> {
    let target = target.as_ref();
    let link = link.as_ref();
    std::os::windows::fs::symlink_dir(target, link).map_err(|source| FsError::Symlink {
        target: target.to_path_buf(),
        link: link.to_path_buf(),
        source,
    })
}
