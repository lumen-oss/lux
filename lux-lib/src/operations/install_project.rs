use crate::{
    build::{Build, BuildBehaviour, BuildError},
    config::Config,
    lockfile::LocalPackage,
    lua_installation::{LuaInstallation, LuaInstallationError},
    luarocks::luarocks_installation::{LuaRocksError, LuaRocksInstallError, LuaRocksInstallation},
    operations::{install_dependencies::prepare_dependencies_for_build, InstallDependencies},
    project::{project_toml::LocalProjectTomlValidationError, Project, ProjectError},
    tree::{self, InstallTree, TreeError},
};
use bon::Builder;
use itertools::Itertools;
use miette::Diagnostic;
use thiserror::Error;

use super::InstallError;

#[derive(Debug, Error, Diagnostic)]
pub enum InstallProjectError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    LocalProjectTomlValidation(#[from] LocalProjectTomlValidationError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Project(#[from] ProjectError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaInstallation(#[from] LuaInstallationError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaRocks(#[from] LuaRocksError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaRocksInstall(#[from] LuaRocksInstallError),
    #[error("error installind dependencies:\n{0}")]
    #[diagnostic(forward(0))]
    InstallDependencies(InstallError),
    #[error("error installind build dependencies:\n{0}")]
    #[diagnostic(forward(0))]
    InstallBuildDependencies(InstallError),
    #[error("error building project:\n{0}")]
    #[diagnostic(forward(0))]
    Build(#[from] BuildError),
}

/// Installs a project into a [`Tree`].
/// Typically, you will want to use [`crate::operations::BuildWorkspace`].
/// Useful for installing a project and its dependencies outside of a workspace tree.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct InstallProject<'a, T>
where
    T: InstallTree,
{
    project: &'a Project,

    config: &'a Config,

    tree: &'a T,
}

impl<
        T: InstallTree + Sync + Send + Clone + 'static,
        State: install_project_builder::State + install_project_builder::IsComplete,
    > InstallProjectBuilder<'_, T, State>
{
    /// Returns `Some` if the `only_deps` option is set to `false`.
    pub async fn build(self) -> Result<LocalPackage, InstallProjectError> {
        let args = self._build();
        let config = args.config;
        let project = args.project;
        let tree = args.tree;
        let build_tree = tree.build_tree(config)?;
        let lua = LuaInstallation::new_from_config(config).await?;
        let luarocks = LuaRocksInstallation::new(config, build_tree.clone())?;
        let mut dependencies_to_install = Vec::new();
        let mut build_dependencies_to_install = Vec::new();
        let project_toml = project.toml().into_local()?;
        prepare_dependencies_for_build(
            &project_toml,
            tree,
            &mut dependencies_to_install,
            &mut build_dependencies_to_install,
        );

        InstallDependencies::new()
            .dependencies(dependencies_to_install.into_iter().unique().collect_vec())
            .build_dependencies(
                build_dependencies_to_install
                    .into_iter()
                    .unique()
                    .collect_vec(),
            )
            .tree(tree)
            .lua(&lua)
            .luarocks(&luarocks)
            .config(config)
            .build()
            .await
            .map_err(InstallProjectError::InstallBuildDependencies)?;

        let package = Build::new()
            .rockspec(&project_toml)
            .lua(&lua)
            .tree(tree)
            .entry_type(tree::EntryType::Entrypoint)
            .config(config)
            .behaviour(BuildBehaviour::Force)
            .build()
            .await?;

        let lockfile = tree.lockfile()?;
        let dependencies = lockfile
            .rocks()
            .iter()
            .filter_map(|(pkg_id, value)| {
                if lockfile.is_entrypoint(pkg_id) {
                    Some(value)
                } else {
                    None
                }
            })
            .cloned()
            .collect_vec();
        let mut lockfile = lockfile.write_guard();
        lockfile.add_entrypoint(&package);
        for dep in dependencies {
            lockfile.add_dependency(&package, &dep);
            lockfile.remove_entrypoint(&dep);
        }
        Ok(package)
    }
}
