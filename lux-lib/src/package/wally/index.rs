use std::{
    io,
    path::{Path, PathBuf},
};

use git2::Repository;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use super::{Manifest, PackageName, PackageReq};

/// Configuration in a wally index's `config.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WallyIndexConfig {
    /// The HTTP registry that serves package contents.
    pub(crate) api: Url,

    #[serde(default)]
    pub(crate) fallback_registries: Vec<String>,
}

/// A local checkout of a wally package index.
#[derive(Debug, Clone)]
pub(crate) struct WallyIndex {
    path: PathBuf,
    config: WallyIndexConfig,
}

impl WallyIndex {
    pub(crate) fn new(path: PathBuf) -> Result<Self, WallyIndexError> {
        let config = serde_json::from_str(&std::fs::read_to_string(path.join("config.json"))?)?;
        Ok(Self { path, config })
    }

    /// Open a wally index, cloning it into `cache_dir` and updating it if needed.
    pub(crate) fn open(url: &Url, cache_dir: &Path) -> Result<Self, WallyIndexError> {
        let path = Self::cache_path(url, cache_dir);
        if path.join(".git").is_dir() {
            let repo = Repository::open(&path)?;
            let mut remote = repo.find_remote("origin")?;
            remote.fetch(&[] as &[&str], None, None)?;
            let head = repo.refname_to_id("refs/remotes/origin/HEAD")?;
            repo.reset(&repo.find_object(head, None)?, git2::ResetType::Hard, None)?;
        } else {
            if path.exists() {
                std::fs::remove_dir_all(&path)?;
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            Repository::clone(url.as_str(), &path)?;
        }
        Self::new(path)
    }

    fn cache_path(url: &Url, cache_dir: &Path) -> PathBuf {
        let ident: String = url
            .as_str()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        cache_dir.join("wally").join("index").join(ident)
    }

    pub(crate) fn config(&self) -> &WallyIndexConfig {
        &self.config
    }

    /// All published versions of a package, newest first.
    pub(crate) fn versions(&self, name: &PackageName) -> Result<Vec<Manifest>, WallyIndexError> {
        let path = self.path.join(name.scope()).join(name.name());
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err.into()),
        };
        let mut versions: Vec<Manifest> = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).map_err(WallyIndexError::from))
            .collect::<Result<_, _>>()?;
        versions.sort_by(|a, b| b.package.version.cmp(&a.package.version));
        Ok(versions)
    }

    /// The latest version of a package matching `req`.
    pub(crate) fn find(&self, req: &PackageReq) -> Result<Option<Manifest>, WallyIndexError> {
        Ok(self
            .versions(req.name())?
            .into_iter()
            .find(|manifest| req.matches(&manifest.package.name, &manifest.package.version)))
    }
}

#[derive(Debug, Error)]
pub enum WallyIndexError {
    #[error("failed to read wally index")]
    Io(#[from] io::Error),
    #[error("failed to parse wally index")]
    Json(#[from] serde_json::Error),
    #[error("failed to fetch wally index")]
    Git(#[from] git2::Error),
}
