use std::{
    fs,
    ops::{Deref, DerefMut},
    path::Path,
};

/// Wrapper around a temporary file that gets deleted when the file is dropped.
/// // NOTE: We can't use the `tempfile` crate because it doesn't support creating
/// // named files without adding its own random bytes.
pub struct TempFile {
    file: fs::File,
    path: std::path::PathBuf,
}

impl TempFile {
    pub fn create(path: &Path) -> std::io::Result<Self> {
        let path = path.into();
        let file = fs::File::create(&path)?;
        Ok(Self { file, path })
    }
}

impl Deref for TempFile {
    type Target = fs::File;

    fn deref(&self) -> &Self::Target {
        &self.file
    }
}

impl DerefMut for TempFile {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.file
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
