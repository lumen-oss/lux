use lux_lib::workspace::Workspace;
use miette::{bail, IntoDiagnostic, Result};
use std::path::{Path, PathBuf};

pub enum PathTarget {
    Workspace(Box<Workspace>),
    Directory(PathBuf),
    File(PathBuf),
}

pub fn classify_path(path: &Path) -> Result<PathTarget> {
    if !path.exists() {
        bail!("path does not exist: {}", path.display());
    }
    if let Some(workspace) = Workspace::from_exact(path)? {
        return Ok(PathTarget::Workspace(Box::new(workspace)));
    }
    let path = std::path::absolute(path).into_diagnostic()?;
    if path.is_file() {
        Ok(PathTarget::File(path))
    } else {
        Ok(PathTarget::Directory(path))
    }
}
