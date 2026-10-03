use std::sync::Arc;

use bon::Builder;
use miette::Diagnostic;
use thiserror::Error;

use crate::{
    config::Config,
    lockfile::LockedPackage,
    operations::PackageInstallSpec,
    project::project_toml::LocalProjectTomlValidationError,
    remote_package_db::{RemotePackageDB, RemotePackageDBError},
    rockspec::Rockspec,
    tree::EntryType,
    workspace::{Workspace, WorkspaceError, WorkspaceTreeError},
};

use super::{
    build::Build, discover::FindPackageFromLuarocks,
    download_sources_and_hash::DownloadSourcesAndHash, emit_lockfile::LockfileHandle,
    resolve::ResolvePackageDependencies,
};

/// Installs all of a workspace's dependencies into its regular, build and test trees.
///
/// This resolves the workspace's dependency graph, downloads and hashes the sources,
/// writes the workspace lockfile, and builds the packages into the appropriate trees.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct InstallWorkspaceDependencies<'a> {
    #[builder(start_fn)]
    config: &'a Config,
    #[builder(start_fn)]
    workspace: &'a Workspace,
    /// Additional packages to install alongside the workspace's own dependencies.
    #[builder(field)]
    packages: Vec<PackageInstallSpec>,
    /// Also install the workspace's test dependencies.
    test: Option<bool>,
    /// Ignore the project's lockfile and don't create one.
    no_lock: Option<bool>,
}

impl<State> InstallWorkspaceDependenciesBuilder<'_, State>
where
    State: install_workspace_dependencies_builder::State,
{
    pub fn packages(mut self, packages: Vec<PackageInstallSpec>) -> Self {
        self.packages = packages;
        self
    }
}

impl<State> InstallWorkspaceDependenciesBuilder<'_, State>
where
    State: install_workspace_dependencies_builder::State
        + install_workspace_dependencies_builder::IsComplete,
{
    pub async fn install(self) -> Result<Vec<LockedPackage>, InstallWorkspaceDependenciesError> {
        let args = self._build();
        let config = args.config;
        let workspace = args.workspace;
        let no_lock = args.no_lock.unwrap_or(false);

        let package_db = RemotePackageDB::from_config(config).await?;
        let discover =
            FindPackageFromLuarocks::new(Arc::new(package_db), Arc::new(config.clone())).build();

        let mut regular = gather_dependencies(workspace, DependencyKind::Regular)?;
        regular.extend(args.packages);
        let build = gather_dependencies(workspace, DependencyKind::Build)?;

        let mut resolve = ResolvePackageDependencies::new(&discover, config)
            .packages(regular)
            .build_packages(build);
        if args.test.unwrap_or(false) {
            resolve = resolve.test_packages(gather_dependencies(workspace, DependencyKind::Test)?);
        }
        let resolved = resolve
            .resolve()
            .await
            .map_err(|err| InstallWorkspaceDependenciesError::Pipeline(Box::new(err)))?;

        let artifacts = DownloadSourcesAndHash::new(config)
            .resolved(resolved)
            .download_sources_and_hash()
            .await
            .map_err(|err| InstallWorkspaceDependenciesError::Pipeline(Box::new(err)))?;

        if !no_lock {
            let mut lockfile = workspace.lockfile()?.write_guard();
            LockfileHandle::from_artifacts(&artifacts).commit(&mut lockfile);
        }

        let regular_tree = workspace.tree(config)?;
        let build_tree = workspace.build_tree(config)?;
        let test_tree = workspace.test_tree(config)?;

        // TODO(vhyrro): Make parallel
        // make sure to build build dependencies first
        Build::new(config, &build_tree)
            .packages(artifacts.build.unwrap_or_default().into_values().collect())
            .build()
            .await
            .map_err(|err| InstallWorkspaceDependenciesError::Pipeline(Box::new(err)))?;
        let built = Build::new(config, &regular_tree)
            .packages(
                artifacts
                    .regular
                    .unwrap_or_default()
                    .into_values()
                    .collect(),
            )
            .build()
            .await
            .map_err(|err| InstallWorkspaceDependenciesError::Pipeline(Box::new(err)))?;
        Build::new(config, &test_tree)
            .packages(artifacts.test.unwrap_or_default().into_values().collect())
            .build()
            .await
            .map_err(|err| InstallWorkspaceDependenciesError::Pipeline(Box::new(err)))?;

        Ok(built)
    }
}

/// The errors that may occur while installing a workspace's dependencies.
#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum InstallWorkspaceDependenciesError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    WorkspaceTree(#[from] WorkspaceTreeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    RemotePackageDB(#[from] RemotePackageDBError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Project(#[from] LocalProjectTomlValidationError),
    #[error("failed to install workspace dependencies")]
    #[diagnostic(forward(0))]
    Pipeline(Box<dyn Diagnostic + Send + Sync + 'static>),
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
) -> Result<Vec<PackageInstallSpec>, InstallWorkspaceDependenciesError> {
    let mut packages = Vec::new();
    for project in workspace.members() {
        let toml = project.toml().into_local()?;
        let dependencies = match kind {
            DependencyKind::Regular => toml.dependencies().current_platform().clone(),
            DependencyKind::Build => toml.build_dependencies().current_platform().clone(),
            DependencyKind::Test => toml.test_dependencies().current_platform().clone(),
        };
        for dependency in dependencies {
            // From the perspective of a project, dependencies are entrypoints.
            packages.push(
                PackageInstallSpec::new(dependency.package_req().clone(), EntryType::Entrypoint)
                    .maybe_source(dependency.source().clone())
                    .build(),
            );
        }
    }
    Ok(packages)
}

#[cfg(all(test, feature = "impure_tests"))]
mod tests {
    use std::path::PathBuf;

    use assert_fs::prelude::PathCopy;

    use crate::{
        config::ConfigBuilder, lockfile::LockedPackageLockType, lua_version::LuaVersion,
        workspace::Workspace,
    };

    use super::InstallWorkspaceDependencies;

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

        InstallWorkspaceDependencies::new(&config, &workspace)
            .install()
            .await
            .unwrap();

        let lockfile = workspace.lockfile().unwrap();
        let rocks = lockfile.rocks(&LockedPackageLockType::Regular);
        assert!(
            rocks
                .values()
                .any(|pkg| pkg.name().to_string() == "lua-cjson")
        );
        assert!(
            rocks
                .values()
                .any(|pkg| pkg.name().to_string() == "plenary.nvim")
        );
    }
}
