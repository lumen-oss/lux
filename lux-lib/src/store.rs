use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use crate::{
    build::Id as BuildId,
    fs,
    lockfile::{LockedPackage, LockedPackageSpec},
    tree::{Error, RockLayout},
};

#[derive(Debug, Clone)]
pub(crate) struct Store {
    root: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct Entry {
    package: LockedPackageSpec,
    path: PathBuf,
}

impl Entry {
    pub(crate) fn package(&self) -> &LockedPackageSpec {
        &self.package
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn complete(&self) -> bool {
        self.layout().rockspec_path().is_file()
    }

    pub(crate) fn layout(&self) -> RockLayout {
        RockLayout::new(self.path.clone())
    }
}

/// An entry that is still being built.
#[derive(Debug)]
pub(crate) struct StagingEntry {
    package: LockedPackageSpec,
    staging: tempfile::TempDir,
    final_path: PathBuf,
}

impl StagingEntry {
    pub(crate) fn layout(&self) -> RockLayout {
        RockLayout::new(self.staging.path().to_path_buf())
    }

    pub(crate) fn prepare(&self) -> Result<(), Error> {
        prepare_layout(&self.layout())
    }

    /// Move the fully built entry into its final location, replacing any
    /// existing entry.
    pub(crate) fn promote(self) -> Result<Entry, Error> {
        // NOTE(vhyrro): Even if this crashes mid-execution things still get
        // cleaned up and the next sync restores order.
        fs::sync::remove_path(&self.final_path)?;
        fs::sync::rename(self.staging.path(), &self.final_path)?;
        let _ = self.staging.keep();

        Ok(Entry {
            package: self.package,
            path: self.final_path,
        })
    }
}

fn prepare_layout(layout: &RockLayout) -> Result<(), Error> {
    fs::sync::create_dir_all(&layout.root)?;
    fs::sync::create_dir_all(&layout.lib)?;
    fs::sync::create_dir_all(&layout.src)?;
    fs::sync::create_dir_all(&layout.etc)?;
    fs::sync::create_dir_all(&layout.bin)?;
    Ok(())
}

impl Store {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Creates a staging entry for a given package.
    pub(crate) fn staging_entry(&self, package: &LockedPackage) -> Result<StagingEntry, Error> {
        fs::sync::create_dir_all(&self.root)?;

        let build_id = BuildId::from_package(package);
        let prefix = format!(".staging-{build_id}{}", Self::entry_suffix(package.spec()));
        let staging = tempfile::Builder::new()
            .prefix(&prefix)
            .tempdir_in(&self.root)
            .map_err(|source| fs::Error::CreateTempDir { source })?;

        Ok(StagingEntry {
            package: package.spec().clone(),
            staging,
            final_path: self.entry_path(package.spec(), &build_id),
        })
    }

    /// Tries to find an already existing entry for a given package (does not create a new entry).
    pub(crate) fn find(&self, package: &LockedPackageSpec, build_id: &BuildId) -> Option<Entry> {
        let path = self.entry_path(package, build_id);

        path.is_dir().then(|| Entry {
            package: package.clone(),
            path,
        })
    }

    /// Tries to find an already complete entry for a given package.
    pub(crate) fn find_complete(&self, package: &LockedPackage) -> Option<Entry> {
        self.find(&package.spec, &BuildId::from_package(package))
            .filter(Entry::complete)
    }

    pub(crate) fn prune(&self, referenced: &HashSet<PathBuf>) -> Result<(), Error> {
        if !self.root.is_dir() {
            return Ok(());
        }

        for entry in fs::sync::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_type().map_err(Error::Io)?.is_dir() {
                continue;
            }

            let path = entry.path();
            if !referenced.contains(&fs::sync::canonicalize(&path)?) {
                fs::sync::remove_dir_all(path)?;
            }
        }

        Ok(())
    }

    fn entry_path(&self, package: &LockedPackageSpec, build_id: &BuildId) -> PathBuf {
        self.root
            .join(format!("{build_id}{}", Self::entry_suffix(package)))
    }

    fn entry_suffix(package: &LockedPackageSpec) -> String {
        format!("-{}@{}", package.name(), package.version())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::Id as BuildId;
    use crate::lockfile::{Constraint, LockedPackageHashes};
    use crate::{package, remote};

    fn package() -> LockedPackage {
        LockedPackage::from(
            &package::Spec::parse("neorg".into(), "8.0.0-1".into()).unwrap(),
            Constraint::Unconstrained,
            remote::package::Source::Test,
            None,
            LockedPackageHashes {
                rockspec: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                    .parse()
                    .unwrap(),
                source: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                    .parse()
                    .unwrap(),
            },
        )
    }

    #[test]
    fn entry_path_is_flat_and_readable() {
        let temp = assert_fs::TempDir::new().unwrap();
        let store = Store::new(temp.path().join("store/5.1"));
        let package = package();
        let path = store.entry_path(&package.spec(), &BuildId::from_package(&package));

        assert_eq!(path.parent(), Some(temp.path().join("store/5.1").as_path()));
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            format!("{}-neorg@8.0.0-1", BuildId::from_package(&package))
        );
    }

    #[test]
    fn build_id_is_stable() {
        let package = package();
        assert_eq!(
            BuildId::from_package(&package),
            BuildId::from_package(&package)
        );
    }

    #[test]
    fn entry_complete_requires_a_rockspec_marker() {
        let temp = assert_fs::TempDir::new().unwrap();
        let store = Store::new(temp.path().join("store/5.1"));
        let package = package();
        let staging = store.staging_entry(&package).unwrap();
        staging.prepare().unwrap();

        assert!(store.find_complete(&package).is_none());

        fs::sync::write(staging.layout().rockspec_path(), "return {}").unwrap();
        let entry = staging.promote().unwrap();

        assert!(entry.complete());
        assert!(store.find_complete(&package).is_some());
    }

    #[test]
    fn dropped_staging_entry_leaves_the_final_entry_alone() {
        let temp = assert_fs::TempDir::new().unwrap();
        let store = Store::new(temp.path().join("store/5.1"));
        let package = package();

        let original = store.staging_entry(&package).unwrap();
        fs::sync::write(original.layout().rockspec_path(), "original").unwrap();
        let original = original.promote().unwrap();

        let replacement = store.staging_entry(&package).unwrap();
        let replacement_root = replacement.layout().root;
        fs::sync::write(replacement.layout().rockspec_path(), "replacement").unwrap();
        drop(replacement);

        assert!(!replacement_root.exists());
        assert_eq!(
            fs::sync::read_to_string(original.layout().rockspec_path()).unwrap(),
            "original"
        );
    }

    #[test]
    fn prune_removes_unreferenced_entries() {
        let temp = assert_fs::TempDir::new().unwrap();
        let store = Store::new(temp.path().join("store/5.1"));
        let package = package();
        let kept = store
            .staging_entry(&package)
            .unwrap()
            .promote()
            .unwrap()
            .path()
            .to_path_buf();

        let unreferenced = store.root.join(format!("{}-other@1.0.0", "a".repeat(64)));
        fs::sync::create_dir_all(&unreferenced).unwrap();

        let referenced = HashSet::from([fs::sync::canonicalize(&kept).unwrap()]);
        store.prune(&referenced).unwrap();

        assert!(kept.is_dir());
        assert!(!unreferenced.exists());
    }
}
