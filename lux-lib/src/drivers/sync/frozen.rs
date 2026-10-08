use std::io;

use crate::operations::{PackageInstallSpec, RemoveError, Uninstall};

pub use crate::drivers::sync::SyncReport;

use crate::drivers::install_packages::{InstallPackages, InstallPackagesError};
use crate::{
    build::BuildBehaviour,
    config::Config,
    fs,
    lockfile::{
        FlushLockfileError, LockedPackage, LockedPackageLockType, Lockfile, LockfileIntegrityError,
        PackageSyncSpec, ReadOnly, ReadWrite, SyncStrategy, WorkspaceLockfile,
    },
    luarocks::luarocks_installation::LUAROCKS_VERSION,
    operations::{self, GenLuaRcError},
    package::{PackageName, PackageReq},
    pipeline::resolve::luarocks_build_backend_name,
    project::{project_toml::LocalProjectTomlValidationError, ProjectError},
    rockspec::{lua_dependency::LuaDependencySpec, Rockspec},
    tree::{self, InstallTree, TreeError},
    workspace::{Workspace, WorkspaceError, WorkspaceTreeError},
};
use bon::Builder;
use itertools::Itertools;
use miette::Diagnostic;
use thiserror::Error;

/// A rocks sync builder, for synchronising a tree with a lockfile.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Sync<'a> {
    #[builder(start_fn)]
    workspace: &'a Workspace,
    #[builder(start_fn)]
    config: &'a Config,

    #[builder(field)]
    extra_packages: Vec<PackageReq>,

    /// Whether to validate the integrity of installed packages.
    validate_integrity: Option<bool>,

    /// Whether to sync test dependencies
    test: Option<bool>,

    /// When `true`, skip filesystem existence checks and rely on the install tree's lockfile
    /// alone.
    fast: Option<bool>,
}

#[cfg(all(test, feature = "impure_tests"))]
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
    pub async fn sync(self) -> Result<SyncReport, SyncError> {
        let mut args = self._build();
        let test_report = if args.test.unwrap_or(false) {
            Some(do_sync(&args, &LockedPackageLockType::Test).await?)
        } else {
            None
        };
        let mut report = do_sync(&args, &LockedPackageLockType::Regular).await?;

        // `do_sync` removes dependencies that aren't listed in the workspace lockfile
        // or in `args.extra_packages`.
        // To prevent loss of transitive build dependencies, we resolve them, and
        // add them to `args.extra_packages` before syncing the workspace's build dependencies.
        let lockfile = args.workspace.tree(args.config)?.lockfile()?;
        let test_tree = args.workspace.test_tree(args.config)?;
        let test_build_lockfile = test_tree.build_tree(args.config)?.lockfile()?;
        let build_lockfile = args.workspace.build_tree(args.config)?.lockfile()?;
        let transitive_build_dependencies = lockfile
            .local_pkg_lock()
            .rocks()
            .values()
            .flat_map(|rock| rock.build_dependencies())
            .filter_map(|dep_id| build_lockfile.get(dep_id))
            .chain(
                test_tree
                    .lockfile()?
                    .local_pkg_lock()
                    .rocks()
                    .values()
                    .flat_map(|rock| rock.build_dependencies())
                    .filter_map(|dep_id| test_build_lockfile.get(dep_id)),
            )
            .map(|pkg| pkg.spec().as_package_req())
            .collect_vec();
        args.extra_packages.extend(transitive_build_dependencies);

        let build_report = do_sync(&args, &LockedPackageLockType::Build).await?;

        operations::GenLuaRc::new()
            .config(args.config)
            .workspace(args.workspace)
            .generate_luarc()
            .await?;

        report.merge(build_report);
        if let Some(test_report) = test_report {
            report.merge(test_report);
        }

        Ok(report)
    }
}

#[derive(Error, Debug, Diagnostic)]
pub enum SyncError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    FlushLockfile(#[from] FlushLockfileError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::FsError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Install(#[from] Box<InstallPackagesError>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Remove(#[from] RemoveError),
    #[error("integrity error for package '{package}'")]
    Integrity {
        package: PackageName,
        #[diagnostic_source]
        source: LockfileIntegrityError,
    },
    #[error(transparent)]
    #[diagnostic(transparent)]
    WorkspaceTree(#[from] WorkspaceTreeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Project(#[from] ProjectError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LocalProjectTomlValidationError(#[from] LocalProjectTomlValidationError),
    #[error("failed to generate `.luarc.json`")]
    #[diagnostic(forward(0))]
    GenLuaRc(#[from] GenLuaRcError),
}

impl From<InstallPackagesError> for SyncError {
    fn from(source: InstallPackagesError) -> Self {
        Self::Install(Box::new(source))
    }
}

#[tracing::instrument(name = "Syncing dependencies", skip_all)]
async fn do_sync(
    args: &Sync<'_>,
    lock_type: &LockedPackageLockType,
) -> Result<SyncReport, SyncError> {
    // NOTE(vhyrro): tools like cc and pkg-config leak cargo:rerun-if-env-changed
    // stdout calls, therefore gag all standard output during sync.
    let _stdout_gag = gag::Gag::stdout();

    let tree = match lock_type {
        LockedPackageLockType::Regular => args.workspace.tree(args.config)?,
        LockedPackageLockType::Test => args.workspace.test_tree(args.config)?,
        LockedPackageLockType::Build => args.workspace.build_tree(args.config)?,
    };
    fs::sync::create_dir_all(tree.root())?;

    let mut workspace_lockfile = args.workspace.lockfile()?.write_guard();
    let dest_lockfile = tree.lockfile()?;

    let packages = SyncPackages::new(args, lock_type).gather()?;
    let strategy = if args.fast.unwrap_or(false) {
        SyncStrategy::LockfileOnly
    } else {
        SyncStrategy::EnsureInstalled(&tree)
    };
    let package_sync_spec = workspace_lockfile.package_sync_spec(&packages, lock_type, &strategy);

    package_sync_spec
        .to_remove
        .iter()
        .for_each(|pkg| workspace_lockfile.remove(pkg, lock_type));

    let package_db = workspace_lockfile.local_pkg_locks().into();

    let (to_add, mut report) = reconcile_locks(&workspace_lockfile, &dest_lockfile, lock_type);
    let member_names = args.workspace.member_names();
    report
        .removed
        .retain(|package| !member_names.contains(package.name()));
    let packages_to_install = install_specs_to_force(&to_add);
    report
        .added
        .extend(to_add.iter().map(|(_, pkg)| pkg).cloned());

    InstallPackages::new(args.config, &tree)
        .package_db(package_db)
        .packages(packages_to_install)
        .install()
        .await?;

    // Read the destination lockfile after installing
    let install_tree_lockfile = tree.lockfile()?;

    validate_integrity(args, &to_add, &install_tree_lockfile)?;

    let packages_to_remove = report.removed.iter().map(|pkg| pkg.id()).collect_vec();

    Uninstall::new()
        .config(args.config)
        .packages(packages_to_remove)
        .tree(tree.clone())
        .remove()
        .await?;

    let member_packages = install_tree_lockfile
        .rocks()
        .values()
        .filter(|package| member_names.contains(package.name()))
        .cloned()
        .collect_vec();

    install_tree_lockfile.map_then_flush(|lockfile| {
        lockfile.sync(workspace_lockfile.local_pkg_lock(lock_type));
        for package in &member_packages {
            lockfile.add_entrypoint(package);
        }
        Ok::<_, io::Error>(())
    })?;

    install_missing_packages(
        args,
        &tree,
        &mut workspace_lockfile,
        lock_type,
        &package_sync_spec,
        &mut report,
    )
    .await?;

    Ok(report)
}

/// Collects the packages which must be synchronised for a given lockfile type:
/// workspace member dependencies plus any extra build backends or test dependencies.
struct SyncPackages<'a, 'b> {
    args: &'a Sync<'b>,
    lock_type: &'a LockedPackageLockType,
}

impl<'a, 'b> SyncPackages<'a, 'b> {
    fn new(args: &'a Sync<'b>, lock_type: &'a LockedPackageLockType) -> Self {
        Self { args, lock_type }
    }

    fn gather(self) -> Result<Vec<LuaDependencySpec>, SyncError> {
        let mut packages = Vec::new();
        for project in self.args.workspace.members() {
            let toml = project.toml().into_local()?;
            match self.lock_type {
                LockedPackageLockType::Regular => {
                    packages.extend(toml.dependencies().current_platform().clone())
                }
                LockedPackageLockType::Build => {
                    packages.extend(toml.build_dependencies().current_platform().clone())
                }
                LockedPackageLockType::Test => {
                    packages.extend(toml.test_dependencies().current_platform().clone())
                }
            }
        }

        let mut extra_packages = self.args.extra_packages.iter().cloned().collect_vec();
        match self.lock_type {
            LockedPackageLockType::Build => {
                for project in self.args.workspace.members() {
                    let toml = project.toml().into_local()?;
                    if let Some(backend) = luarocks_build_backend_name(&toml) {
                        extra_packages.push(backend.into());
                        if cfg!(target_family = "unix") {
                            let luarocks = unsafe {
                                PackageReq::new_unchecked(
                                    "luarocks".into(),
                                    Some(LUAROCKS_VERSION.into()),
                                )
                            };
                            extra_packages.push(luarocks);
                        }
                    }
                }
            }
            LockedPackageLockType::Test => {
                for project in self.args.workspace.members() {
                    let toml = project.toml().into_local()?;
                    for test_dep in toml
                        .test()
                        .current_platform()
                        .test_dependencies(project)
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
                        extra_packages.push(test_dep);
                    }
                }
            }
            LockedPackageLockType::Regular => {}
        }

        Ok(packages
            .into_iter()
            .chain(extra_packages.into_iter().unique().map_into())
            .collect())
    }
}

/// Determines which packages are present in one lockfile but not the other, producing the
/// packages to add (with their entry type) and the report of packages that must be removed.
fn reconcile_locks(
    workspace_lockfile: &WorkspaceLockfile<ReadWrite>,
    dest_lockfile: &Lockfile<ReadOnly>,
    lock_type: &LockedPackageLockType,
) -> (Vec<(tree::EntryType, LockedPackage)>, SyncReport) {
    let mut to_add: Vec<(tree::EntryType, LockedPackage)> = Vec::new();
    let mut report = SyncReport {
        added: Vec::new(),
        removed: Vec::new(),
    };

    for (id, local_package) in workspace_lockfile.rocks(lock_type) {
        if dest_lockfile.get(id).is_none() {
            let entry_type = if workspace_lockfile.is_entrypoint(&local_package.id(), lock_type) {
                tree::EntryType::Entrypoint
            } else {
                tree::EntryType::DependencyOnly
            };
            to_add.push((entry_type, local_package.clone()));
        }
    }
    for (id, local_package) in dest_lockfile.rocks() {
        if workspace_lockfile.get(id, lock_type).is_none() {
            report.removed.push(local_package.clone());
        }
    }

    (to_add, report)
}

fn install_specs_to_force(to_add: &[(tree::EntryType, LockedPackage)]) -> Vec<PackageInstallSpec> {
    to_add
        .iter()
        .map(|(entry_type, pkg)| {
            PackageInstallSpec::new(pkg.clone().into_package_req(), *entry_type)
                .build_behaviour(BuildBehaviour::Force)
                .pin(pkg.pinned())
                .opt(pkg.opt())
                .constraint(pkg.constraint())
                .build()
        })
        .unique()
        .collect()
}

fn validate_integrity(
    args: &Sync<'_>,
    to_add: &[(tree::EntryType, LockedPackage)],
    install_tree_lockfile: &Lockfile<ReadOnly>,
) -> Result<(), SyncError> {
    if !args.validate_integrity.unwrap_or(true) {
        return Ok(());
    }
    for (_, package) in to_add {
        install_tree_lockfile
            .validate_integrity(package)
            .map_err(|source| SyncError::Integrity {
                package: package.name().clone(),
                source,
            })?;
    }
    Ok(())
}

/// Installs packages that were newly added to the workspace lockfile but are not yet present in
/// the install tree, using the default package database.
async fn install_missing_packages<T>(
    args: &Sync<'_>,
    tree: &T,
    workspace_lockfile: &mut WorkspaceLockfile<ReadWrite>,
    lock_type: &LockedPackageLockType,
    package_sync_spec: &PackageSyncSpec,
    report: &mut SyncReport,
) -> Result<(), SyncError>
where
    T: InstallTree + Clone + Send + std::marker::Sync + 'static,
{
    if package_sync_spec.to_add.is_empty() {
        return Ok(());
    }

    let missing_packages = package_sync_spec
        .to_add
        .iter()
        .map(|dep| {
            PackageInstallSpec::new(dep.package_req().clone(), tree::EntryType::Entrypoint)
                .build_behaviour(BuildBehaviour::Force)
                .pin(*dep.pin())
                .opt(*dep.opt())
                .maybe_source(dep.source.clone())
                .build()
        })
        .unique()
        .collect();

    let added = InstallPackages::new(args.config, tree)
        .packages(missing_packages)
        .install()
        .await?
        .0;

    report.added.extend(added);

    // Sync the newly added packages back to the workspace lockfile
    let dest_lockfile = tree.lockfile()?;
    workspace_lockfile.sync(dest_lockfile.local_pkg_lock(), lock_type);

    Ok(())
}

#[cfg(all(test, feature = "impure_tests"))]
mod tests {
    use super::Sync;
    use crate::{
        config::ConfigBuilder, lockfile::LockedPackageLockType, package::PackageReq,
        workspace::Workspace,
    };
    use assert_fs::{prelude::PathCopy, TempDir};
    use flaky_test::flaky_test;
    use std::path::PathBuf;

    #[flaky_test(tokio, times = 5)]
    async fn test_sync_add_rocks() {
        let temp_dir = TempDir::new().unwrap();
        temp_dir
            .copy_from(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("resources/test/sample-projects/dependencies/"),
                &["**"],
            )
            .unwrap();
        let workspace = Workspace::from_exact(temp_dir.path()).unwrap().unwrap();
        let config = ConfigBuilder::new().unwrap().build().unwrap();
        let report = Sync::new(&workspace, &config).sync().await.unwrap();
        assert!(report.removed.is_empty());
        assert!(!report.added.is_empty());

        let lockfile_after_sync = workspace.lockfile().unwrap();
        assert!(!lockfile_after_sync
            .rocks(&LockedPackageLockType::Regular)
            .is_empty());
    }

    #[flaky_test(tokio, times = 5)]
    async fn test_sync_add_rocks_with_new_package() {
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
        let workspace = Workspace::from_exact(temp_dir.path()).unwrap().unwrap();
        {
            let report = Sync::new(&workspace, &config)
                .add_package(PackageReq::new("toml-edit".into(), None).unwrap())
                .sync()
                .await
                .unwrap();
            assert!(report.removed.is_empty());
            assert!(!report.added.is_empty());
            assert!(report
                .added
                .iter()
                .any(|pkg| pkg.name().to_string() == "toml-edit"));
        }
        let lockfile_after_sync = workspace.lockfile().unwrap();
        assert!(!lockfile_after_sync
            .rocks(&LockedPackageLockType::Regular)
            .is_empty());
    }

    #[flaky_test(tokio, times = 5)]
    async fn regression_sync_nonexistent_lock() {
        // This test checks that we can sync a lockfile that doesn't exist yet, and whether
        // the sync report is valid.
        let temp_dir = TempDir::new().unwrap();
        temp_dir
            .copy_from(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("resources/test/sample-projects/dependencies/"),
                &["**"],
            )
            .unwrap();
        let config = ConfigBuilder::new().unwrap().build().unwrap();
        let workspace = Workspace::from_exact(temp_dir.path()).unwrap().unwrap();
        {
            let report = Sync::new(&workspace, &config)
                .add_package(PackageReq::new("toml-edit".into(), None).unwrap())
                .sync()
                .await
                .unwrap();
            assert!(report.removed.is_empty());
            assert!(!report.added.is_empty());
            assert!(report
                .added
                .iter()
                .any(|pkg| pkg.name().to_string() == "toml-edit"));
        }
        let lockfile_after_sync = workspace.lockfile().unwrap();
        assert!(!lockfile_after_sync
            .rocks(&LockedPackageLockType::Regular)
            .is_empty());
    }

    #[flaky_test(tokio, times = 5)]
    async fn test_sync_remove_rocks() {
        let temp_dir = TempDir::new().unwrap();
        temp_dir
            .copy_from(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("resources/test/sample-projects/dependencies/"),
                &["**"],
            )
            .unwrap();
        let config = ConfigBuilder::new().unwrap().build().unwrap();
        let workspace = Workspace::from_exact(temp_dir.path()).unwrap().unwrap();
        // First sync to create the tree and lockfile
        Sync::new(&workspace, &config)
            .add_package(PackageReq::new("toml-edit".into(), None).unwrap())
            .sync()
            .await
            .unwrap();
        let report = Sync::new(&workspace, &config).sync().await.unwrap();
        assert!(!report.removed.is_empty());
        assert!(report.added.is_empty());

        let lockfile_after_sync = workspace.lockfile().unwrap();
        assert!(!lockfile_after_sync
            .rocks(&LockedPackageLockType::Regular)
            .is_empty());
    }
}
