use std::{io, sync::Arc};

use crate::{
    build::BuildBehaviour,
    config::Config,
    lockfile::{
        LocalPackage, LocalPackageLockType, LockfileIntegrityError, PackageSyncSpec,
        ProjectLockfile, ReadWrite,
    },
    luarocks::luarocks_installation::LUAROCKS_VERSION,
    package::{PackageName, PackageReq},
    progress::{MultiProgress, Progress},
    rockspec::lua_dependency::LuaDependencySpec,
    tree::{self, Tree, TreeError},
};
use bon::{builder, Builder};
use itertools::Itertools;
use thiserror::Error;

use super::{Install, InstallError, PackageInstallSpec, Remove, RemoveError};

/// A rocks sync builder, for synchronising a tree with a lockfile.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Sync<'a> {
    /// The tree to sync
    #[builder(start_fn)]
    tree: &'a Tree,
    /// The project lockfile to sync the tree with
    #[builder(start_fn)]
    project_lockfile: &'a mut ProjectLockfile<ReadWrite>,
    #[builder(start_fn)]
    config: &'a Config,
    #[builder(field)]
    progress: Option<Arc<Progress<MultiProgress>>>,
    /// Sync the source lockfile with these package requirements.
    #[builder(field)]
    packages: Option<Vec<LuaDependencySpec>>,
    /// Whether to validate the integrity of installed packages.
    validate_integrity: Option<bool>,
}

impl<State> SyncBuilder<'_, State>
where
    State: sync_builder::State,
{
    pub fn progress(mut self, progress: Arc<Progress<MultiProgress>>) -> Self {
        self.progress = Some(progress);
        self
    }

    pub fn packages(mut self, packages: Vec<LuaDependencySpec>) -> Self {
        self.packages = Some(packages);
        self
    }

    pub fn add_packages(&mut self, packages: Vec<LuaDependencySpec>) -> &Self {
        self.packages = Some(packages);
        self
    }

    fn add_package(&mut self, package: LuaDependencySpec) -> &Self {
        match &mut self.packages {
            Some(packages) => packages.push(package),
            None => self.packages = Some(vec![package]),
        }
        self
    }
}

impl<State> SyncBuilder<'_, State>
where
    State: sync_builder::State + sync_builder::IsComplete,
{
    pub async fn sync_dependencies(self) -> Result<SyncReport, SyncError> {
        do_sync(self._build(), &LocalPackageLockType::Regular).await
    }

    pub async fn sync_test_dependencies(mut self) -> Result<SyncReport, SyncError> {
        let busted = PackageReq::new("busted".into(), None).unwrap().into();
        self.add_package(busted);
        do_sync(self._build(), &LocalPackageLockType::Test).await
    }

    pub async fn sync_build_dependencies(mut self) -> Result<SyncReport, SyncError> {
        if cfg!(target_family = "unix")
            && self
                .packages
                .as_ref()
                .is_some_and(|packages| !packages.is_empty())
        {
            let luarocks = PackageReq::new("luarocks".into(), Some(LUAROCKS_VERSION.into()))
                .unwrap()
                .into();
            self.add_package(luarocks);
        }
        do_sync(self._build(), &LocalPackageLockType::Build).await
    }
}

#[derive(Debug)]
pub struct SyncReport {
    pub(crate) added: Vec<LocalPackage>,
    pub(crate) removed: Vec<LocalPackage>,
}

#[derive(Error, Debug)]
pub enum SyncError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    Install(#[from] InstallError),
    #[error(transparent)]
    Remove(#[from] RemoveError),
    #[error("integrity error for package {0}: {1}\n")]
    Integrity(PackageName, LockfileIntegrityError),
}

async fn do_sync(
    args: Sync<'_>,
    lock_type: &LocalPackageLockType,
) -> Result<SyncReport, SyncError> {
    let progress = args.progress.unwrap_or(MultiProgress::new_arc());
    std::fs::create_dir_all(args.tree.root())?;
    let dest_lockfile = args.tree.lockfile()?;

    let package_sync_spec = match &args.packages {
        Some(packages) => args.project_lockfile.package_sync_spec(packages, lock_type),
        None => PackageSyncSpec::default(),
    };

    package_sync_spec
        .to_remove
        .iter()
        .for_each(|pkg| args.project_lockfile.remove(pkg, lock_type));

    let mut to_add: Vec<(tree::EntryType, LocalPackage)> = Vec::new();

    let mut report = SyncReport {
        added: Vec::new(),
        removed: Vec::new(),
    };
    for (id, local_package) in args.project_lockfile.rocks(lock_type) {
        if dest_lockfile.get(id).is_none() {
            let entry_type = if args
                .project_lockfile
                .is_entrypoint(&local_package.id(), lock_type)
            {
                tree::EntryType::Entrypoint
            } else {
                tree::EntryType::DependencyOnly
            };
            to_add.push((entry_type, local_package.clone()));
        }
    }
    for (id, local_package) in dest_lockfile.rocks() {
        if args.project_lockfile.get(id, lock_type).is_none() {
            report.removed.push(local_package.clone());
        }
    }

    let packages_to_install = to_add
        .iter()
        .cloned()
        .map(|(entry_type, pkg)| {
            PackageInstallSpec::new(pkg.clone().into_package_req(), entry_type)
                .build_behaviour(BuildBehaviour::Force)
                .pin(pkg.pinned())
                .opt(pkg.opt())
                .constraint(pkg.constraint())
                .build()
        })
        .collect_vec();
    report
        .added
        .extend(to_add.iter().map(|(_, pkg)| pkg).cloned());

    let package_db = args
        .project_lockfile
        .local_pkg_lock(lock_type)
        .clone()
        .into();
    Install::new(args.tree, args.config)
        .package_db(package_db)
        .packages(packages_to_install)
        .progress(progress.clone())
        .install()
        .await?;

    // Read the destination lockfile after installing
    let dest_lockfile = args.tree.lockfile()?;

    if args.validate_integrity.unwrap_or(true) {
        for (_, package) in &to_add {
            dest_lockfile
                .validate_integrity(package)
                .map_err(|err| SyncError::Integrity(package.name().clone(), err))?;
        }
    }

    let packages_to_remove = report
        .removed
        .iter()
        .cloned()
        .map(|pkg| pkg.id())
        .collect_vec();

    Remove::new(args.config)
        .packages(packages_to_remove)
        .progress(progress.clone())
        .remove()
        .await?;

    dest_lockfile.map_then_flush(|lockfile| -> Result<(), io::Error> {
        lockfile.sync(args.project_lockfile.local_pkg_lock(lock_type));
        Ok(())
    })?;

    if !package_sync_spec.to_add.is_empty() {
        // Install missing packages using the default package_db.
        let missing_packages = package_sync_spec.to_add.into_iter().map(|pkg| {
            PackageInstallSpec::new(pkg.package_req().clone(), tree::EntryType::Entrypoint)
                .build_behaviour(BuildBehaviour::Force)
                .pin(*pkg.pin())
                .opt(*pkg.opt())
                .build()
        });

        let added = Install::new(args.tree, args.config)
            .packages(missing_packages)
            .progress(progress.clone())
            .install()
            .await?;

        report.added.extend(added);

        // Sync the newly added packages back to the project lockfile
        let dest_lockfile = args.tree.lockfile()?;
        args.project_lockfile
            .sync(dest_lockfile.local_pkg_lock(), lock_type);
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use assert_fs::{prelude::PathCopy, TempDir};

    use crate::config::{ConfigBuilder, LuaVersion};

    use super::*;

    #[tokio::test]
    async fn test_sync_add_rocks() {
        if std::env::var("LUX_SKIP_IMPURE_TESTS").unwrap_or("0".into()) == "1" {
            println!("Skipping impure test");
            return;
        }
        let project_lockfile_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/lux.lock");
        let mut source_lockfile = ProjectLockfile::new(project_lockfile_path.clone())
            .unwrap()
            .write_guard();
        let temp_dir = TempDir::new().unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .tree(Some(temp_dir.path().into()))
            .build()
            .unwrap();
        let dest_tree = config.tree(LuaVersion::Lua51).unwrap();
        let report = Sync::new(&dest_tree, &mut source_lockfile, &config)
            .sync_dependencies()
            .await
            .unwrap();
        assert!(report.removed.is_empty());
        assert!(!report.added.is_empty());

        let lockfile_after_sync = ProjectLockfile::new(project_lockfile_path).unwrap();
        assert!(!lockfile_after_sync
            .rocks(&LocalPackageLockType::Regular)
            .is_empty());
    }

    #[tokio::test]
    async fn test_sync_add_rocks_with_new_package() {
        if std::env::var("LUX_SKIP_IMPURE_TESTS").unwrap_or("0".into()) == "1" {
            println!("Skipping impure test");
            return;
        }
        let empty_lockfile_dir = TempDir::new().unwrap();
        let lockfile_path = empty_lockfile_dir.path().join("lux.lock");
        {
            let mut empty_lockfile = ProjectLockfile::new(lockfile_path.clone())
                .unwrap()
                .write_guard();
            let temp_dir = TempDir::new().unwrap();
            let config = ConfigBuilder::new()
                .unwrap()
                .tree(Some(temp_dir.path().into()))
                .build()
                .unwrap();
            let dest_tree = config.tree(LuaVersion::Lua51).unwrap();
            let report = Sync::new(&dest_tree, &mut empty_lockfile, &config)
                .packages(vec![PackageReq::new("toml-edit".into(), None)
                    .unwrap()
                    .into()])
                .sync_dependencies()
                .await
                .unwrap();
            assert!(report.removed.is_empty());
            assert!(!report.added.is_empty());
            assert!(!report
                .added
                .iter()
                .filter(|pkg| pkg.name().to_string() == "toml-edit")
                .collect_vec()
                .is_empty());
        }
        let lockfile_after_sync = ProjectLockfile::new(lockfile_path).unwrap();
        assert!(!lockfile_after_sync
            .rocks(&LocalPackageLockType::Regular)
            .is_empty());
    }

    #[tokio::test]
    async fn test_sync_remove_rocks() {
        let tree_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-tree");
        let temp_dir = TempDir::new().unwrap();
        temp_dir.copy_from(&tree_path, &["**"]).unwrap();
        let empty_lockfile_dir = TempDir::new().unwrap();
        let lockfile_path = empty_lockfile_dir.path().join("lux.lock");
        let mut empty_lockfile = ProjectLockfile::new(lockfile_path.clone())
            .unwrap()
            .write_guard();
        let config = ConfigBuilder::new()
            .unwrap()
            .tree(Some(temp_dir.path().into()))
            .build()
            .unwrap();
        let dest_tree = config.tree(LuaVersion::Lua51).unwrap();
        let report = Sync::new(&dest_tree, &mut empty_lockfile, &config)
            .sync_dependencies()
            .await
            .unwrap();
        assert!(!report.removed.is_empty());
        assert!(report.added.is_empty());

        let lockfile_after_sync = ProjectLockfile::new(lockfile_path).unwrap();
        assert!(lockfile_after_sync
            .rocks(&LocalPackageLockType::Regular)
            .is_empty());
    }
}
