use bon::Builder;
use miette::Diagnostic;
use nonempty::NonEmpty;
use thiserror::Error;

mod frozen;

use crate::{
    build::BuildBehaviour,
    config::Config,
    drivers::install_packages::{InstallPackages, InstallPackagesError},
    lockfile::{LockedPackage, LockedPackageLockType},
    operations::{
        GenLuaRc, GenLuaRcError, InstallProject, InstallProjectError, PackageInstallSpec,
    },
    package::PackageName,
    package_db::{PackageDB, PackageDBError},
    project::project_toml::LocalProjectTomlValidationError,
    rockspec::Rockspec,
    tree::{EntryType, InstallTree, TreeError},
    workspace::{Workspace, WorkspaceError, WorkspaceTreeError},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncMode {
    /// The lockfile may be rewritten. Missing dependencies are resolved from
    /// manifests, preferring locked versions.
    Open,
    /// The lockfile is authoritative. Nothing is resolved from manifests.
    Frozen,
}

#[derive(Clone, Debug)]
pub struct TargetSet {
    pub test: bool,
    pub members: NonEmpty<PackageName>,
}

impl TargetSet {
    pub fn all(workspace: &Workspace, test: bool) -> Self {
        Self {
            test,
            members: workspace
                .members()
                .clone()
                .map(|project| project.toml().package.clone()),
        }
    }

    pub fn member(member: PackageName, test: bool) -> Self {
        Self {
            test,
            members: NonEmpty::new(member),
        }
    }
}

#[derive(Debug)]
pub struct SyncReport {
    pub(crate) added: Vec<LockedPackage>,
    pub(crate) removed: Vec<LockedPackage>,
}

impl SyncReport {
    pub fn added(&self) -> &[LockedPackage] {
        &self.added
    }
    pub fn removed(&self) -> &[LockedPackage] {
        &self.removed
    }

    pub(crate) fn merge(&mut self, other: SyncReport) {
        self.added.extend(other.added);
        self.removed.extend(other.removed);
    }
}

#[derive(Error, Debug, Diagnostic)]
pub enum SyncError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    InstallPackages(#[from] Box<InstallPackagesError>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    InstallProject(#[from] InstallProjectError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    WorkspaceTree(#[from] WorkspaceTreeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Project(#[from] LocalProjectTomlValidationError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    PackageDB(#[from] PackageDBError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    GenLuaRc(#[from] GenLuaRcError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Frozen(#[from] frozen::SyncError),
}

impl From<InstallPackagesError> for SyncError {
    fn from(source: InstallPackagesError) -> Self {
        Self::InstallPackages(Box::new(source))
    }
}

/// Reconciles a workspace's install trees with its dependencies.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Sync<'a> {
    #[builder(start_fn)]
    workspace: &'a Workspace,
    #[builder(start_fn)]
    config: &'a Config,

    mode: SyncMode,

    targets: TargetSet,

    #[builder(default = BuildBehaviour::Force)]
    behaviour: BuildBehaviour,

    /// Build only the dependencies, not the workspace members.
    #[builder(default = false)]
    only_deps: bool,

    /// Ignore the project's lockfile and don't create one.
    #[builder(default = false)]
    no_lock: bool,

    /// Whether to validate the integrity of installed packages (`Frozen` mode).
    validate_integrity: Option<bool>,
}

impl<State> SyncBuilder<'_, State>
where
    State: sync_builder::State + sync_builder::IsComplete,
{
    pub async fn sync(self) -> Result<SyncReport, SyncError> {
        let args = self._build();
        match args.mode {
            SyncMode::Open => sync_open(&args).await,
            SyncMode::Frozen => sync_frozen(&args).await,
        }
    }
}

async fn sync_open(args: &Sync<'_>) -> Result<SyncReport, SyncError> {
    let config = args.config;
    let workspace = args.workspace;

    let tree = workspace.tree(config)?;
    let build_tree = tree.build_tree(config)?;

    let regular = gather_dependencies(
        workspace,
        DependencyKind::Regular,
        &workspace.member_names()?,
    )?
    .into_iter()
    .filter(|spec| !tree.match_rocks(&spec.package).is_ok_and(|m| m.is_found()))
    .map(|spec| PackageInstallSpec {
        build_behaviour: BuildBehaviour::Force,
        ..spec
    })
    .collect::<Vec<_>>();

    let build = gather_dependencies(workspace, DependencyKind::Build, &workspace.member_names()?)?
        .into_iter()
        .filter(|spec| {
            !build_tree
                .match_rocks(&spec.package)
                .is_ok_and(|m| m.is_found())
        })
        .map(|spec| PackageInstallSpec {
            build_behaviour: BuildBehaviour::Force,
            ..spec
        })
        .collect::<Vec<_>>();

    let test_tree = tree.test_tree(config)?;
    let test = if args.targets.test {
        gather_dependencies(workspace, DependencyKind::Test, &args.targets.members)?
            .into_iter()
            .filter(|spec| {
                !test_tree
                    .match_rocks(&spec.package)
                    .is_ok_and(|m| m.is_found())
            })
            .map(|spec| PackageInstallSpec {
                build_behaviour: BuildBehaviour::Force,
                ..spec
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let package_db = PackageDB::open(config, workspace).await?;

    let mut install = InstallPackages::new(config, &tree)
        .packages(regular)
        .build_packages(build)
        .package_db(package_db);
    if !test.is_empty() {
        install = install.test_packages(test);
    }

    let (mut added, lockfile) = install.install().await?;

    if !args.no_lock {
        let mut workspace_lockfile = workspace.lockfile()?.write_guard();
        lockfile.commit(&mut workspace_lockfile);
        if args.targets.test {
            workspace_lockfile.sync(
                test_tree.lockfile()?.local_pkg_lock(),
                &LockedPackageLockType::Test,
            );
        }
    }

    if !args.only_deps {
        let projects = args
            .targets
            .members
            .iter()
            .map(|name| workspace.select_member(name))
            .collect::<Result<Vec<_>, _>>()?;
        for project in projects {
            added.push(
                InstallProject::new()
                    .project(project)
                    .config(config)
                    .tree(&tree)
                    .behaviour(args.behaviour)
                    .workspace(workspace)
                    .build()
                    .await?,
            );
        }
    }

    if !args.no_lock {
        GenLuaRc::new()
            .config(config)
            .workspace(workspace)
            .generate_luarc()
            .await?;
    }

    Ok(SyncReport {
        added,
        removed: Vec::new(),
    })
}

#[derive(Clone, Copy)]
enum DependencyKind {
    Regular,
    Build,
    Test,
}

fn gather_dependencies(
    workspace: &Workspace,
    kind: DependencyKind,
    members: &NonEmpty<PackageName>,
) -> Result<Vec<PackageInstallSpec>, SyncError> {
    let mut packages = Vec::new();
    for project in workspace.members() {
        if !members.contains(project.toml().package()) {
            continue;
        }
        let toml = project.toml().into_local()?;
        if let DependencyKind::Test = kind {
            let explicit = toml.test_dependencies().current_platform();
            for dependency in toml.test().current_platform().test_dependencies(project) {
                if explicit.iter().any(|dep| dep.name() == dependency.name()) {
                    continue;
                }
                packages.push(
                    PackageInstallSpec::new(dependency, EntryType::Entrypoint)
                        .build_behaviour(BuildBehaviour::Ignore)
                        .build(),
                );
            }
            for dependency in explicit {
                if dependency.name().eq(&PackageName::new("lua".into())) {
                    continue;
                }
                packages.push(
                    PackageInstallSpec::new(
                        dependency.package_req().clone(),
                        EntryType::Entrypoint,
                    )
                    .build_behaviour(BuildBehaviour::Ignore)
                    .pin(*dependency.pin())
                    .opt(*dependency.opt())
                    .maybe_source(dependency.source.clone())
                    .build(),
                );
            }
        } else {
            let dependencies = match kind {
                DependencyKind::Regular => toml.dependencies().current_platform().clone(),
                _ => toml.build_dependencies().current_platform().clone(),
            };
            for dependency in dependencies {
                // From the perspective of a project, dependencies are entrypoints.
                packages.push(
                    PackageInstallSpec::new(
                        dependency.package_req().clone(),
                        EntryType::Entrypoint,
                    )
                    .build_behaviour(BuildBehaviour::Ignore)
                    .maybe_source(dependency.source().clone())
                    .build(),
                );
            }
        }
    }
    Ok(packages)
}

async fn sync_frozen(args: &Sync<'_>) -> Result<SyncReport, SyncError> {
    let report = frozen::Sync::new(args.workspace, args.config)
        .validate_integrity(args.validate_integrity.unwrap_or(true))
        .test(args.targets.test)
        .sync()
        .await?;
    Ok(report)
}

#[cfg(all(test, feature = "impure_tests"))]
mod tests {
    use std::path::PathBuf;

    use assert_fs::prelude::{FileWriteStr, PathChild, PathCopy};

    use crate::{
        config::ConfigBuilder, lockfile::LockedPackageLockType, lua_version::LuaVersion,
        package::PackageName, package_db::PackageDB, tree::InstallTree, workspace::Workspace,
    };

    use super::{Sync, SyncMode, TargetSet};

    #[tokio::test]
    async fn installs_workspace_dependencies() {
        let sample = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample-projects/dependencies/");
        let temp = assert_fs::TempDir::new().unwrap();
        temp.copy_from(sample, &["**"]).unwrap();

        let workspace = Workspace::from_exact(temp.path()).unwrap().unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .lua_version(Some(LuaVersion::Lua51))
            .build()
            .unwrap();

        Sync::new(&workspace, &config)
            .mode(SyncMode::Open)
            .targets(TargetSet::all(&workspace, false))
            .only_deps(true)
            .sync()
            .await
            .unwrap();

        let lockfile = workspace.lockfile().unwrap();
        let rocks = lockfile.rocks(&LockedPackageLockType::Regular);
        assert!(rocks
            .values()
            .any(|pkg| pkg.name().to_string() == "lua-cjson"));
        assert!(rocks
            .values()
            .any(|pkg| pkg.name().to_string() == "plenary.nvim"));
    }

    #[tokio::test]
    async fn install_honors_locked_version() {
        let temp = assert_fs::TempDir::new().unwrap();
        let toml = temp.child("lux.toml");
        toml.write_str(
            r#"
            package = "locktest"
            version = "0.1.0"
            lua = ">=5.1"

            [build]
            type = "builtin"

            [dependencies]
            argparse = "==0.7.1-1"
            "#,
        )
        .unwrap();

        let config = ConfigBuilder::new()
            .unwrap()
            .lua_version(Some(LuaVersion::Lua51))
            .build()
            .unwrap();
        let name = PackageName::from("argparse");

        let workspace = Workspace::from_exact(temp.path()).unwrap().unwrap();
        Sync::new(&workspace, &config)
            .mode(SyncMode::Open)
            .targets(TargetSet::all(&workspace, false))
            .only_deps(true)
            .sync()
            .await
            .unwrap();
        let tree = workspace.tree(&config).unwrap();
        assert_eq!(
            tree.lockfile()
                .unwrap()
                .entrypoint(&name)
                .unwrap()
                .version()
                .to_string(),
            "0.7.1-1"
        );

        toml.write_str(
            r#"
            package = "locktest"
            version = "0.1.0"
            lua = ">=5.1"

            [build]
            type = "builtin"

            [dependencies]
            argparse = ">=0.7.1"
            "#,
        )
        .unwrap();
        std::fs::remove_dir_all(tree.root()).unwrap();

        let workspace = Workspace::from_exact(temp.path()).unwrap().unwrap();
        Sync::new(&workspace, &config)
            .mode(SyncMode::Open)
            .targets(TargetSet::all(&workspace, false))
            .only_deps(true)
            .sync()
            .await
            .unwrap();
        let tree = workspace.tree(&config).unwrap();
        assert_eq!(
            tree.lockfile()
                .unwrap()
                .entrypoint(&name)
                .unwrap()
                .version()
                .to_string(),
            "0.7.1-1"
        );
    }

    #[tokio::test]
    async fn install_without_lock_resolves_latest() {
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("lux.toml")
            .write_str(
                r#"
                package = "locktest"
                version = "0.1.0"
                lua = ">=5.1"

                [build]
                type = "builtin"

                [dependencies]
                argparse = ">=0.7.1"
                "#,
            )
            .unwrap();

        let config = ConfigBuilder::new()
            .unwrap()
            .lua_version(Some(LuaVersion::Lua51))
            .build()
            .unwrap();
        let name = PackageName::from("argparse");

        let workspace = Workspace::from_exact(temp.path()).unwrap().unwrap();
        Sync::new(&workspace, &config)
            .mode(SyncMode::Open)
            .targets(TargetSet::all(&workspace, false))
            .only_deps(true)
            .sync()
            .await
            .unwrap();
        let tree = workspace.tree(&config).unwrap();
        let installed = tree
            .lockfile()
            .unwrap()
            .entrypoint(&name)
            .unwrap()
            .version()
            .clone();
        let latest = PackageDB::from_config(&config)
            .await
            .unwrap()
            .latest_version(&name)
            .unwrap();
        assert_eq!(installed, latest);
        assert_ne!(installed.to_string(), "0.7.1-1");
    }

    #[tokio::test]
    async fn replaces_dependency_on_version_change() {
        let temp = assert_fs::TempDir::new().unwrap();
        let toml = temp.child("lux.toml");
        toml.write_str(
            r#"
            package = "versionchange"
            version = "0.1.0"
            lua = ">=5.1"

            [build]
            type = "builtin"

            [dependencies]
            argparse = "==0.7.1-1"
            "#,
        )
        .unwrap();

        let config = ConfigBuilder::new()
            .unwrap()
            .lua_version(Some(LuaVersion::Lua51))
            .build()
            .unwrap();
        let name = PackageName::from("argparse");

        let workspace = Workspace::from_exact(temp.path()).unwrap().unwrap();
        Sync::new(&workspace, &config)
            .mode(SyncMode::Open)
            .targets(TargetSet::all(&workspace, false))
            .only_deps(true)
            .sync()
            .await
            .unwrap();
        let tree = workspace.tree(&config).unwrap();
        assert_eq!(
            tree.lockfile()
                .unwrap()
                .entrypoint(&name)
                .unwrap()
                .version()
                .to_string(),
            "0.7.1-1"
        );

        toml.write_str(
            r#"
            package = "versionchange"
            version = "0.1.0"
            lua = ">=5.1"

            [build]
            type = "builtin"

            [dependencies]
            argparse = "==0.7.2-1"
            "#,
        )
        .unwrap();

        let workspace = Workspace::from_exact(temp.path()).unwrap().unwrap();
        Sync::new(&workspace, &config)
            .mode(SyncMode::Open)
            .targets(TargetSet::all(&workspace, false))
            .only_deps(true)
            .sync()
            .await
            .unwrap();
        let tree = workspace.tree(&config).unwrap();
        assert_eq!(
            tree.lockfile()
                .unwrap()
                .entrypoint(&name)
                .unwrap()
                .version()
                .to_string(),
            "0.7.2-1"
        );
    }
}
