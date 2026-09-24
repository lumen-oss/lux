use bon::Builder;
use miette::Diagnostic;
use thiserror::Error;

use crate::{
    build::BuildBehaviour,
    config::Config,
    lockfile::LockedPackage,
    lua_installation::{LuaInstallation, LuaInstallationError},
    project::project_toml::LocalProjectTomlValidationError,
    tree::{EntryType, InstallTree, TreeError},
    workspace::{Workspace, WorkspaceTreeError},
};

/// Builds a single workspace member into the workspace's install tree, recording
/// it as an entrypoint in the tree's lockfile.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(crate) struct BuildProject<'a> {
    #[builder(start_fn)]
    project: &'a crate::project::Project,
    #[builder(start_fn)]
    workspace: &'a Workspace,
    #[builder(start_fn)]
    config: &'a Config,
    #[builder(start_fn)]
    lua: &'a LuaInstallation,
}

impl<State> BuildProjectBuilder<'_, State>
where
    State: build_project_builder::State + build_project_builder::IsComplete,
{
    pub(crate) async fn build(self) -> Result<LockedPackage, BuildProjectError> {
        let args = self._build();

        let workspace_tree = args.workspace.tree(args.config)?;
        let project_toml = args.project.toml().into_local()?;

        let package = crate::build::Build::new()
            .rockspec(&project_toml)
            .lua(args.lua)
            .tree(&workspace_tree)
            .entry_type(EntryType::Entrypoint)
            .config(args.config)
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
        let build_lockfile = args.workspace.build_tree(args.config)?.lockfile()?;

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
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub(crate) enum BuildProjectError {
    #[error("failed to build the project")]
    #[diagnostic(forward(0))]
    Build(#[source] Box<crate::build::BuildError>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaInstallation(#[from] LuaInstallationError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    WorkspaceTree(#[from] WorkspaceTreeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Project(#[from] LocalProjectTomlValidationError),
}

impl From<crate::build::BuildError> for BuildProjectError {
    fn from(source: crate::build::BuildError) -> Self {
        Self::Build(Box::new(source))
    }
}
