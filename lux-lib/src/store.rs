use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use crate::{
    fs,
    lockfile::{LockedPackage, LockedPackageSpec},
    tree::{Error, RockLayout},
};
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub struct Store {
    root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Entry {
    package: LockedPackageSpec,
    path: PathBuf,
}

impl Store {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn entry(&self, package: &LockedPackage) -> Result<Entry, Error> {
        let package_root = self.root.join(package.id().to_string());
        fs::sync::create_dir_all(&package_root)?;
        let hash = content_hash(package);

        for suffix in 0.. {
            let name = if suffix == 0 {
                hash.clone()
            } else {
                format!("{hash}-{suffix}")
            };
            let path = package_root.join(name);
            match fs::sync::create_dir(&path) {
                Ok(()) => {
                    return Ok(Entry {
                        package: package.spec().clone(),
                        path,
                    });
                }
                Err(fs::Error::CreateDir { source, .. })
                    if source.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }

        unreachable!()
    }

    pub(crate) fn adopt(&self, package: &LockedPackage, path: &Path) -> Result<Entry, Error> {
        let entry = self.entry(package)?;
        fs::sync::remove_dir_all(&entry.path)?;
        fs::sync::rename(path, &entry.path)?;
        Ok(entry)
    }

    pub(crate) fn prune(&self, referenced: &HashSet<PathBuf>) -> Result<(), Error> {
        if !self.root.is_dir() {
            return Ok(());
        }

        for package in fs::sync::read_dir(&self.root)? {
            let package = package?;
            if !package.file_type().map_err(Error::Io)?.is_dir() {
                continue;
            }

            for entry in fs::sync::read_dir(package.path())? {
                let entry = entry?;
                if !entry.file_type().map_err(Error::Io)?.is_dir() {
                    continue;
                }

                let path = entry.path();
                if !referenced.contains(&canonicalize(&path)?) {
                    fs::sync::remove_dir_all(path)?;
                }
            }
        }

        Ok(())
    }
}

impl Entry {
    pub(crate) fn package(&self) -> &LockedPackageSpec {
        &self.package
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn layout(&self) -> RockLayout {
        RockLayout::new(self.path.clone())
    }

    pub(crate) fn prepare(&self) -> Result<(), Error> {
        let layout = self.layout();
        fs::sync::create_dir_all(&layout.root)?;
        fs::sync::create_dir_all(&layout.lib)?;
        fs::sync::create_dir_all(&layout.src)?;
        fs::sync::create_dir_all(&layout.etc)?;
        fs::sync::create_dir_all(&layout.bin)?;
        Ok(())
    }
}

fn content_hash(package: &LockedPackage) -> String {
    let mut hasher = Sha256::new();
    hasher.update(package.hashes().rockspec.to_hex().1);
    hasher.update(package.hashes().source.to_hex().1);
    hex::encode(hasher.finalize())
}

pub(crate) fn canonicalize(path: &Path) -> Result<PathBuf, Error> {
    std::fs::canonicalize(path).map_err(|source| {
        fs::Error::Metadata {
            path: path.to_path_buf(),
            source,
        }
        .into()
    })
}
