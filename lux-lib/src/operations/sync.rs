use std::{io, sync::Arc};

use crate::{
    build::BuildBehaviour,
    config::Config,
    lockfile::{FlushLockfileError, LocalPackage, LocalPackageLockType, LockfileIntegrityError},
    luarocks::luarocks_installation::LUAROCKS_VERSION,
    operations::{self, GenLuaRcError},
    package::{PackageName, PackageReq},
    progress::{MultiProgress, Progress},
    project::{
        project_toml::LocalProjectTomlValidationError, Project, ProjectError, ProjectTreeError,
    },
    rockspec::Rockspec,
    tree::{self, TreeError},
};
use bon::Builder;
use itertools::Itertools;
use thiserror::Error;

use super::{Install, InstallError, PackageInstallSpec, RemoveError, Uninstall};

/// A rocks sync builder, for synchronising a tree with a lockfile.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Sync<'a> {
    #[builder(start_fn)]
    project: &'a Project,
    #[builder(start_fn)]
    config: &'a Config,

    #[builder(field)]
    extra_packages: Vec<PackageReq>,

    progress: Option<Arc<Progress<MultiProgress>>>,
    /// Whether to validate the integrity of installed packages.
    validate_integrity: Option<bool>,
}

impl<State> SyncBuilder<'_, State>
where
    State: sync_builder::State,
{
    pub fn add_package(mut self, package: PackageReq) -> Self {
        self.extra_packages.push(package);
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
        let toml = self.project.toml().into_local()?;
        for test_dep in toml
            .test()
            .current_platform()
            .test_dependencies(self.project)
            .iter()
            .filter(|test_dep| {
                !toml
                    .test_dependencies()
                    .current_platform()
                    .iter()
                    .any(|dep| dep.name() == test_dep.name())
            })
            .cloned()
        {
            self.extra_packages.push(test_dep);
        }
        do_sync(self._build(), &LocalPackageLockType::Test).await
    }

    pub async fn sync_build_dependencies(mut self) -> Result<SyncReport, SyncError> {
        if cfg!(target_family = "unix") && !self.extra_packages.is_empty() {
            let toml = self.project.toml().into_local()?;
            if toml
                .build()
                .current_platform()
                .build_backend
                .as_ref()
                .is_some_and(|build_backend| {
                    matches!(
                        build_backend,
                        crate::lua_rockspec::BuildBackendSpec::LuaRock(_)
                    )
                })
            {
                let luarocks = unsafe {
                    PackageReq::new_unchecked("luarocks".into(), Some(LUAROCKS_VERSION.into()))
                };
                self = self.add_package(luarocks);
            }
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
    FlushLockfile(#[from] FlushLockfileError),
    #[error("failed to create install tree at {0}:\n{1}")]
    FailedToCreateDirectory(String, io::Error),
    #[error(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    Install(#[from] InstallError),
    #[error(transparent)]
    Remove(#[from] RemoveError),
    #[error("integrity error for package {0}: {1}\n")]
    Integrity(PackageName, LockfileIntegrityError),
    #[error(transparent)]
    ProjectTreeError(#[from] ProjectTreeError),
    #[error(transparent)]
    ProjectError(#[from] ProjectError),
    #[error(transparent)]
    LocalProjectTomlValidationError(#[from] LocalProjectTomlValidationError),
    #[error("failed to generate `.luarc.json`:\n{0}")]
    GenLuaRc(#[from] GenLuaRcError),
}

async fn do_sync(
    args: Sync<'_>,
    lock_type: &LocalPackageLockType,
) -> Result<SyncReport, SyncError> {
    let tree = match lock_type {
        LocalPackageLockType::Regular => args.project.tree(args.config)?,
        LocalPackageLockType::Test => args.project.test_tree(args.config)?,
        LocalPackageLockType::Build => args.project.build_tree(args.config)?,
    };
    std::fs::create_dir_all(tree.root()).map_err(|err| {
        SyncError::FailedToCreateDirectory(tree.root().to_string_lossy().to_string(), err)
    })?;

    let mut project_lockfile = args.project.lockfile()?.write_guard();
    let dest_lockfile = tree.lockfile()?;

    let progress = args.progress.unwrap_or(MultiProgress::new_arc(args.config));

    let packages = match lock_type {
        LocalPackageLockType::Regular => args
            .project
            .toml()
            .into_local()?
            .dependencies()
            .current_platform()
            .clone(),
        LocalPackageLockType::Build => args
            .project
            .toml()
            .into_local()?
            .build_dependencies()
            .current_platform()
            .clone(),
        LocalPackageLockType::Test => args
            .project
            .toml()
            .into_local()?
            .test_dependencies()
            .current_platform()
            .clone(),
    }
    .into_iter()
    .chain(args.extra_packages.into_iter().map_into())
    .collect_vec();

    let package_sync_spec = project_lockfile.package_sync_spec(&packages, lock_type);

    package_sync_spec
        .to_remove
        .iter()
        .for_each(|pkg| project_lockfile.remove(pkg, lock_type));

    let mut to_add: Vec<(tree::EntryType, LocalPackage)> = Vec::new();

    let mut report = SyncReport {
        added: Vec::new(),
        removed: Vec::new(),
    };
    for (id, local_package) in project_lockfile.rocks(lock_type) {
        if dest_lockfile.get(id).is_none() {
            let entry_type = if project_lockfile.is_entrypoint(&local_package.id(), lock_type) {
                tree::EntryType::Entrypoint
            } else {
                tree::EntryType::DependencyOnly
            };
            to_add.push((entry_type, local_package.clone()));
        }
    }
    for (id, local_package) in dest_lockfile.rocks() {
        if project_lockfile.get(id, lock_type).is_none() {
            report.removed.push(local_package.clone());
        }
    }

    let packages_to_install = to_add
        .iter()
        .map(|(entry_type, pkg)| {
            PackageInstallSpec::new(pkg.clone().into_package_req(), *entry_type)
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

    let package_db = project_lockfile.local_pkg_lock(lock_type).clone().into();

    Install::new(args.config)
        .package_db(package_db)
        .packages(packages_to_install)
        .tree(tree.clone())
        .progress(progress.clone())
        .install()
        .await?;

    // Read the destination lockfile after installing
    let install_tree_lockfile = tree.lockfile()?;

    if args.validate_integrity.unwrap_or(true) {
        for (_, package) in &to_add {
            install_tree_lockfile
                .validate_integrity(package)
                .map_err(|err| SyncError::Integrity(package.name().clone(), err))?;
        }
    }

    let packages_to_remove = report.removed.iter().map(|pkg| pkg.id()).collect_vec();

    Uninstall::new()
        .config(args.config)
        .packages(packages_to_remove)
        .progress(progress.clone())
        .tree(tree.clone())
        .remove()
        .await?;

    install_tree_lockfile.map_then_flush(|lockfile| {
        lockfile.sync(project_lockfile.local_pkg_lock(lock_type));
        Ok::<_, io::Error>(())
    })?;

    if !package_sync_spec.to_add.is_empty() {
        // Install missing packages using the default package_db.
        let missing_packages = package_sync_spec
            .to_add
            .into_iter()
            .map(|dep| {
                PackageInstallSpec::new(dep.package_req().clone(), tree::EntryType::Entrypoint)
                    .build_behaviour(BuildBehaviour::Force)
                    .pin(*dep.pin())
                    .opt(*dep.opt())
                    .maybe_source(dep.source.clone())
                    .build()
            })
            .collect();

        let added = Install::new(args.config)
            .packages(missing_packages)
            .tree(tree.clone())
            .progress(progress.clone())
            .install()
            .await?;

        report.added.extend(added);

        // Sync the newly added packages back to the project lockfile
        let dest_lockfile = tree.lockfile()?;
        project_lockfile.sync(dest_lockfile.local_pkg_lock(), lock_type);
    }

    operations::GenLuaRc::new()
        .config(args.config)
        .project(args.project)
        .generate_luarc()
        .await?;

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::Sync;
    use crate::{
        config::ConfigBuilder, lockfile::LocalPackageLockType, package::PackageReq,
        project::Project,
    };
    use assert_fs::{prelude::PathCopy, TempDir};
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_sync_add_rocks() {
        if std::env::var("LUX_SKIP_IMPURE_TESTS").unwrap_or("0".into()) == "1" {
            println!("Skipping impure test");
            return;
        }
        let temp_dir = TempDir::new().unwrap();
        temp_dir
            .copy_from(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("resources/test/sample-projects/dependencies/"),
                &["**"],
            )
            .unwrap();
        let project = Project::from_exact(temp_dir.path()).unwrap().unwrap();
        let config = ConfigBuilder::new().unwrap().build().unwrap();
        let report = Sync::new(&project, &config)
            .sync_dependencies()
            .await
            .unwrap();
        assert!(report.removed.is_empty());
        assert!(!report.added.is_empty());

        let lockfile_after_sync = project.lockfile().unwrap();
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
        let temp_dir = TempDir::new().unwrap();
        temp_dir
            .copy_from(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("resources/test/sample-projects/dependencies/"),
                &["**"],
            )
            .unwrap();
        let temp_dir = temp_dir.into_persistent();
        let config = ConfigBuilder::new().unwrap().build().unwrap();
        let project = Project::from_exact(temp_dir.path()).unwrap().unwrap();
        {
            let report = Sync::new(&project, &config)
                .add_package(PackageReq::new("toml-edit".into(), None).unwrap())
                .sync_dependencies()
                .await
                .unwrap();
            assert!(report.removed.is_empty());
            assert!(!report.added.is_empty());
            assert!(report
                .added
                .iter()
                .any(|pkg| pkg.name().to_string() == "toml-edit"));
        }
        let lockfile_after_sync = project.lockfile().unwrap();
        assert!(!lockfile_after_sync
            .rocks(&LocalPackageLockType::Regular)
            .is_empty());
    }

    #[tokio::test]
    async fn regression_sync_nonexistent_lock() {
        // This test checks that we can sync a lockfile that doesn't exist yet, and whether
        // the sync report is valid.
        if std::env::var("LUX_SKIP_IMPURE_TESTS").unwrap_or("0".into()) == "1" {
            println!("Skipping impure test");
            return;
        }
        let temp_dir = TempDir::new().unwrap();
        temp_dir
            .copy_from(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("resources/test/sample-projects/dependencies/"),
                &["**"],
            )
            .unwrap();
        let config = ConfigBuilder::new().unwrap().build().unwrap();
        let project = Project::from_exact(temp_dir.path()).unwrap().unwrap();
        {
            let report = Sync::new(&project, &config)
                .add_package(PackageReq::new("toml-edit".into(), None).unwrap())
                .sync_dependencies()
                .await
                .unwrap();
            assert!(report.removed.is_empty());
            assert!(!report.added.is_empty());
            assert!(report
                .added
                .iter()
                .any(|pkg| pkg.name().to_string() == "toml-edit"));
        }
        let lockfile_after_sync = project.lockfile().unwrap();
        assert!(!lockfile_after_sync
            .rocks(&LocalPackageLockType::Regular)
            .is_empty());
    }

    #[tokio::test]
    async fn test_sync_remove_rocks() {
        if std::env::var("LUX_SKIP_IMPURE_TESTS").unwrap_or("0".into()) == "1" {
            println!("Skipping impure test");
            return;
        }
        let temp_dir = TempDir::new().unwrap();
        temp_dir
            .copy_from(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("resources/test/sample-projects/dependencies/"),
                &["**"],
            )
            .unwrap();
        let config = ConfigBuilder::new().unwrap().build().unwrap();
        let project = Project::from_exact(temp_dir.path()).unwrap().unwrap();
        // First sync to create the tree and lockfile
        Sync::new(&project, &config)
            .add_package(PackageReq::new("toml-edit".into(), None).unwrap())
            .sync_dependencies()
            .await
            .unwrap();
        let report = Sync::new(&project, &config)
            .sync_dependencies()
            .await
            .unwrap();
        assert!(!report.removed.is_empty());
        assert!(report.added.is_empty());

        let lockfile_after_sync = project.lockfile().unwrap();
        assert!(!lockfile_after_sync
            .rocks(&LocalPackageLockType::Regular)
            .is_empty());
    }
}
