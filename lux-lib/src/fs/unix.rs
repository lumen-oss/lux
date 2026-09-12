use std::path::Path;

use super::FsError;

/// Wrapped [`std::os::unix::fs::symlink`].
pub(crate) fn symlink(target: impl AsRef<Path>, link: impl AsRef<Path>) -> Result<(), FsError> {
    let target = target.as_ref();
    let link = link.as_ref();
    std::os::unix::fs::symlink(target, link).map_err(|source| FsError::Symlink {
        target: target.to_path_buf(),
        link: link.to_path_buf(),
        source,
    })
}
