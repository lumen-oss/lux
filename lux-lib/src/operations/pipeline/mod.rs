#![allow(dead_code)]

pub(crate) mod build;
pub(crate) mod discover;
pub(crate) mod download_sources_and_hash;
pub mod install_packages;
pub(crate) mod resolve;

use std::collections::HashMap;
use std::sync::Arc;

use bon::Builder;
use miette::Diagnostic;
use thiserror::Error;

use crate::{
    build::BuildBehaviour, config::Config, lockfile::{
        LockedPackage, LockedPackageId, LockedPackageLock, LockedPackageLockType, ReadWrite,
        WorkspaceLockfile,
    }, lua_installation::{LuaInstallation, LuaInstallationError}, operations::{GenLuaRc, GenLuaRcError, PackageInstallSpec, pipeline::download_sources_and_hash::{DownloadSourcesAndHashArtifacts, DownloadedPackage}}, package::PackageName, project::{project_toml::LocalProjectTomlValidationError, Project},     remote_package_db::{RemotePackageDB, RemotePackageDBError}, rockspec::Rockspec, tree::{EntryType, InstallTree, TreeError}, workspace::{Workspace, WorkspaceError, WorkspaceTreeError},
};

use self::{
    build::Build, discover::FindPackageFromLuarocks, download_sources_and_hash::DownloadSourcesAndHash,
    resolve::ResolvePackageDependencies,
};

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Pipeline<'a> {
    #[builder(start_fn)]
    pub(crate) config: &'a Config,
    #[builder(start_fn)]
    pub(crate) workspace: &'a Workspace,
    #[builder(field)]
    pub(crate) packages: Vec<PackageInstallSpec>,
    pub(crate) test: Option<bool>,
    /// Build the workspace's own project(s) after installing dependencies.
    pub(crate) build_projects: Option<bool>,
    /// Only build this workspace member.
    pub(crate) package: Option<PackageName>,
    /// Ignore the project's lockfile and don't create one.
    pub(crate) no_lock: Option<bool>,
}

impl<State> PipelineBuilder<'_, State>
where
    State: pipeline_builder::State,
{
    pub fn packages(mut self, packages: Vec<PackageInstallSpec>) -> Self {
        self.packages = packages;
        self
    }
}

impl<State> PipelineBuilder<'_, State>
where
    State: pipeline_builder::State + pipeline_builder::IsComplete,
{
    pub async fn run(self) -> miette::Result<Vec<LockedPackage>> {
        let args = self._build();
        let config = args.config;
        let workspace = args.workspace;
        let no_lock = args.no_lock.unwrap_or(false);

        let package_db = RemotePackageDB::from_config(config).await?;
        let discover = FindPackageFromLuarocks::new(Arc::new(package_db), Arc::new(config.clone())).build();

        let mut regular = gather_dependencies(workspace, DependencyKind::Regular)?;
        regular.extend(args.packages);
        let build = gather_dependencies(workspace, DependencyKind::Build)?;
        let test = if args.test.unwrap_or(false) {
            gather_dependencies(workspace, DependencyKind::Test)?
        } else {
            Vec::new()
        };

        let resolved = ResolvePackageDependencies::new(&discover, config)
            .packages(regular)
            .build_packages(build)
            .test_packages(test)
            .resolve()
            .await?;

        let artifacts = DownloadSourcesAndHash::new(config)
            .resolved(resolved)
            .download_sources_and_hash()
            .await?;

        if !no_lock {
            let mut lockfile = workspace.lockfile()?.write_guard();
            EmitLockfile::new(&mut lockfile)
                .artifacts(&artifacts)
                .emit()?;
        }

        let regular_tree = workspace.tree(config)?;
        let build_tree = workspace.build_tree(config)?;
        let test_tree = workspace.test_tree(config)?;

        // TODO(vhyrro): Make parallel
        // make sure to build build dependencies first
        Build::new(config, &build_tree)
            .packages(artifacts.build.into_values().collect())
            .build()
            .await?;
        Build::new(config, &regular_tree)
            .packages(artifacts.regular.into_values().collect())
            .build()
            .await?;
        Build::new(config, &test_tree)
            .packages(artifacts.test.into_values().collect())
            .build()
            .await?;

        let mut built = Vec::new();
        if args.build_projects.unwrap_or(false) {
            let lua = LuaInstallation::new_from_config(config).await?;
            match &args.package {
                Some(package) => {
                    let project = workspace.select_member(package)?;
                    built.push(build_project(project, workspace, &lua, config).await?);
                }
                None => {
                    for project in workspace.members() {
                        built.push(build_project(project, workspace, &lua, config).await?);
                    }
                }
            }
        }

        if !no_lock {
            GenLuaRc::new()
                .config(config)
                .workspace(workspace)
                .generate_luarc()
                .await?;
        }

        Ok(built)
    }
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
) -> Result<Vec<PackageInstallSpec>, PipelineError> {
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

async fn build_project(
    project: &Project,
    workspace: &Workspace,
    lua: &LuaInstallation,
    config: &Config,
) -> Result<LockedPackage, PipelineError> {
    let workspace_tree = workspace.tree(config)?;
    let project_toml = project.toml().into_local()?;

    let package = crate::build::Build::new()
        .rockspec(&project_toml)
        .lua(lua)
        .tree(&workspace_tree)
        .entry_type(EntryType::Entrypoint)
        .config(config)
        .behaviour(BuildBehaviour::Force)
        .build()
        .await?;

    let lockfile = workspace_tree.lockfile()?;
    let dependencies = lockfile
        .rocks()
        .iter()
        .filter_map(|(id, package)| lockfile.is_entrypoint(id).then_some(package))
        .cloned()
        .collect::<Vec<_>>();
    let build_lockfile = workspace.build_tree(config)?.lockfile()?;

    let mut lockfile = lockfile.write_guard();
    lockfile.add_entrypoint(&package);
    for dependency in dependencies {
        lockfile.add_dependency(&package, &dependency);
    }
    for dependency in build_lockfile.rocks().values() {
        lockfile.add_build_dependency(&package, dependency);
    }

    Ok(package)
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub(crate) enum PipelineError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Resolve(#[from] resolve::ResolveError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    DownloadSourcesAndHash(#[from] download_sources_and_hash::DownloadSourcesAndHashError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Build(#[from] build::BuildError),
    #[error("failed to build the project")]
    #[diagnostic(forward(0))]
    ProjectBuild(#[source] Box<crate::build::BuildError>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaInstallation(#[from] LuaInstallationError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Emit(#[from] EmitError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    GenLuaRc(#[from] GenLuaRcError),
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
}

// FIXME(vhyrro): Use the natural `#[from]` instead of manual implementation
impl From<crate::build::BuildError> for PipelineError {
    fn from(source: crate::build::BuildError) -> Self {
        Self::ProjectBuild(Box::new(source))
    }
}

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(crate) struct EmitLockfile<'a> {
    #[builder(start_fn)]
    pub(crate) lockfile: &'a mut WorkspaceLockfile<ReadWrite>,
    pub(crate) artifacts: &'a DownloadSourcesAndHashArtifacts,
}

impl<State> EmitLockfileBuilder<'_, State>
where
    State: emit_lockfile_builder::State + emit_lockfile_builder::IsComplete,
{
    pub(crate) fn emit(self) -> Result<(), EmitError> {
        let args = self._build();

        args.lockfile.sync(
            &lock_from(&args.artifacts.regular),
            &LockedPackageLockType::Regular,
        );
        args.lockfile.sync(
            &lock_from(&args.artifacts.build),
            &LockedPackageLockType::Build,
        );
        args.lockfile.sync(
            &lock_from(&args.artifacts.test),
            &LockedPackageLockType::Test,
        );

        Ok(())
    }
}

fn lock_from(
    packages: &HashMap<LockedPackageId, DownloadedPackage>,
) -> LockedPackageLock {
    // FIXME(vhyrro): Create a constructor here instead of mut overrides.
    let mut lock = LockedPackageLock::default();
    for package in packages.values() {
        lock.insert(package.package.clone(), package.entry_type.is_entrypoint());
    }
    lock
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub(crate) enum EmitError {
    #[error("failed to serialize the lockfile")]
    Serialize,
}

#[cfg(all(test, feature = "impure_tests"))]
mod tests {
    use std::path::PathBuf;

    use assert_fs::prelude::PathCopy;

    use crate::{config::ConfigBuilder, lua_version::LuaVersion, workspace::Workspace};

    use super::*;

    #[tokio::test]
    async fn pipeline_installs_workspace_dependencies() {
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

        Pipeline::new(&config, &workspace).run().await.unwrap();

        let lockfile = workspace.lockfile().unwrap();
        let rocks = lockfile.rocks(&LockedPackageLockType::Regular);
        assert!(rocks
            .values()
            .any(|pkg| pkg.name().to_string() == "lua-cjson"));
        assert!(rocks
            .values()
            .any(|pkg| pkg.name().to_string() == "plenary.nvim"));
    }
}
