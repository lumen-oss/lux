use bon::Builder;
use itertools::Itertools;
use std::sync::Arc;
use thiserror::Error;

use crate::{
    build::{Build, BuildBehaviour, BuildError},
    config::Config,
    lockfile::LocalPackage,
    lua_installation::{LuaInstallation, LuaInstallationError},
    luarocks::luarocks_installation::{LuaRocksError, LuaRocksInstallError, LuaRocksInstallation},
    operations::{install_dependencies::prepare_dependencies_for_build, InstallDependencies},
    progress::{MultiProgress, Progress},
    project::{project_toml::LocalProjectTomlValidationError, Project, ProjectError},
    tree::{self, InstallTree, TreeError},
};

use super::InstallError;

#[derive(Debug, Error)]
pub enum InstallProjectError {
    #[error(transparent)]
    LocalProjectTomlValidation(#[from] LocalProjectTomlValidationError),
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error(transparent)]
    LuaInstallation(#[from] LuaInstallationError),
    #[error(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    LuaRocks(#[from] LuaRocksError),
    #[error(transparent)]
    LuaRocksInstall(#[from] LuaRocksInstallError),
    #[error("error installind dependencies:\n{0}")]
    InstallDependencies(InstallError),
    #[error("error installind build dependencies:\n{0}")]
    InstallBuildDependencies(InstallError),
    #[error("error building project:\n{0}")]
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

    progress: Option<Arc<Progress<MultiProgress>>>,
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
        let progress_arc = args
            .progress
            .clone()
            .unwrap_or_else(|| MultiProgress::new_arc(args.config));
        let lua = LuaInstallation::new_from_config(
            config,
            &progress_arc.map(|progress| progress.new_bar()),
        )
        .await?;
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
            .progress(progress_arc.clone())
            .build()
            .await
            .map_err(InstallProjectError::InstallBuildDependencies)?;

        let package = Build::new()
            .rockspec(&project_toml)
            .lua(&lua)
            .tree(tree)
            .entry_type(tree::EntryType::Entrypoint)
            .config(config)
            .progress(&progress_arc.map(|p| p.new_bar()))
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
