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
    package::PackageName,
    progress::{MultiProgress, Progress},
    project::{project_toml::LocalProjectTomlValidationError, Project},
    tree::{self, TreeError},
    workspace::{Workspace, WorkspaceError, WorkspaceTreeError},
};

use super::{InstallError, Sync, SyncError};

#[derive(Debug, Error)]
pub enum BuildWorkspaceError {
    #[error(transparent)]
    LocalProjectTomlValidation(#[from] LocalProjectTomlValidationError),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    WorkspaceTree(#[from] WorkspaceTreeError),
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
    #[error("syncing dependencies with the project lockfile failed.\nUse --no-lock to force a new build.\n\n{0}")]
    SyncDependencies(SyncError),
    #[error("syncing build dependencies with the project lockfile failed.\nUse --no-lock to force a new build.\n\n{0}")]
    SyncBuildDependencies(SyncError),
    #[error("error building project:\n{0}")]
    Build(#[from] BuildError),
}

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct BuildWorkspace<'a> {
    #[builder(start_fn)]
    workspace: &'a Workspace,

    #[builder(start_fn)]
    config: &'a Config,

    /// Package to build
    package: Option<PackageName>,

    /// Ignore the project's lockfile and don't create one
    no_lock: bool,

    /// Build only the dependencies
    only_deps: bool,

    progress: Option<Arc<Progress<MultiProgress>>>,
}

impl<State: build_workspace_builder::State + build_workspace_builder::IsComplete>
    BuildWorkspaceBuilder<'_, State>
{
    /// Returns `Some` if the `only_deps` option is set to `false`.
    pub async fn build(self) -> Result<Vec<LocalPackage>, BuildWorkspaceError> {
        let args = self._build();
        let config = args.config;
        let workspace = args.workspace;
        let workspace_tree = workspace.tree(config)?;
        let build_tree = workspace.build_tree(config)?;
        let progress_arc = args
            .progress
            .clone()
            .unwrap_or_else(|| MultiProgress::new_arc(args.config));
        let lua = LuaInstallation::new_from_config(
            config,
            &progress_arc.map(|progress| progress.new_bar()),
        )
        .await?;
        if !args.no_lock {
            Sync::new(workspace, config)
                .progress(progress_arc.clone())
                .sync_dependencies()
                .await
                .map_err(BuildWorkspaceError::SyncDependencies)?;

            Sync::new(workspace, config)
                .progress(progress_arc.clone())
                .sync_build_dependencies()
                .await
                .map_err(BuildWorkspaceError::SyncBuildDependencies)?;
        } else {
            let luarocks = LuaRocksInstallation::new(config, build_tree.clone())?;
            let mut dependencies_to_install = Vec::new();
            let mut build_dependencies_to_install = Vec::new();
            if let Some(package) = &args.package {
                let project = workspace.select_member(package)?;
                let project_toml = project.toml().into_local()?;
                prepare_dependencies_for_build(
                    &project_toml,
                    &workspace_tree,
                    &mut dependencies_to_install,
                    &mut build_dependencies_to_install,
                );
            } else {
                for project in workspace.members() {
                    let project_toml = project.toml().into_local()?;
                    prepare_dependencies_for_build(
                        &project_toml,
                        &workspace_tree,
                        &mut dependencies_to_install,
                        &mut build_dependencies_to_install,
                    );
                }
            }

            let tree = workspace.tree(config)?;

            InstallDependencies::new()
                .dependencies(dependencies_to_install.into_iter().unique().collect_vec())
                .build_dependencies(
                    build_dependencies_to_install
                        .into_iter()
                        .unique()
                        .collect_vec(),
                )
                .tree(&tree)
                .lua(&lua)
                .luarocks(&luarocks)
                .config(config)
                .progress(progress_arc.clone())
                .build()
                .await
                .map_err(BuildWorkspaceError::InstallBuildDependencies)?;
        }

        let mut packages = Vec::new();
        if !args.only_deps {
            if let Some(package) = &args.package {
                let project = workspace.select_member(package)?;
                let pkg =
                    build_project(project, workspace, &lua, config, progress_arc.clone()).await?;
                packages.push(pkg);
            } else {
                for project in workspace.members() {
                    let pkg = build_project(project, workspace, &lua, config, progress_arc.clone())
                        .await?;
                    packages.push(pkg);
                }
            }
        }
        Ok(packages)
    }
}

async fn build_project(
    project: &Project,
    workspace: &Workspace,
    lua: &LuaInstallation,
    config: &Config,
    progress: Arc<Progress<MultiProgress>>,
) -> Result<LocalPackage, BuildWorkspaceError> {
    let workspace_tree = workspace.tree(config)?;
    let project_toml = project.toml().into_local()?;

    let package = Build::new()
        .rockspec(&project_toml)
        .lua(lua)
        .tree(&workspace_tree)
        .entry_type(tree::EntryType::Entrypoint)
        .config(config)
        .progress(&progress.map(|p| p.new_bar()))
        .behaviour(BuildBehaviour::Force)
        .build()
        .await?;

    let lockfile = workspace_tree.lockfile()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{
        config::ConfigBuilder, lua_installation::detect_installed_lua_version,
        lua_version::LuaVersion,
    };
    use assert_fs::prelude::PathCopy;
    use std::path::PathBuf;

    #[tokio::test]
    /// Non-regression for #980
    async fn builtin_build_autodetect_bin_scripts() {
        let project_root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-projects/init/");
        let data_dir: PathBuf = assert_fs::TempDir::new().unwrap().path().into();
        let temp_dir = assert_fs::TempDir::new().unwrap();
        temp_dir.copy_from(&project_root, &["**"]).unwrap();
        let project_root = temp_dir.path();
        let foo_bin_dir = project_root.join("src").join("bin");
        tokio::fs::create_dir_all(&foo_bin_dir).await.unwrap();
        let foo_bin_file = foo_bin_dir.join("foo");
        tokio::fs::write(&foo_bin_file, "print('hello')")
            .await
            .unwrap();
        let bar_bin_dir = project_root.join("bin");
        tokio::fs::create_dir_all(&bar_bin_dir).await.unwrap();
        let bar_bin_file = bar_bin_dir.join("bar");
        tokio::fs::write(&bar_bin_file, "print('hello')")
            .await
            .unwrap();
        let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));
        let config = ConfigBuilder::new()
            .unwrap()
            .data_dir(Some(data_dir))
            .lua_version(lua_version)
            .build()
            .unwrap();
        let workspace = Workspace::from_exact(project_root).unwrap().unwrap();
        let tree = workspace.tree(&config).unwrap();
        let package = BuildWorkspace::new(&workspace, &config)
            .no_lock(false)
            .only_deps(false)
            .build()
            .await
            .unwrap();
        let package = package.first().unwrap();
        let layout = tree.installed_rock_layout(package).unwrap();
        let bin_dir = layout.bin;
        assert!(bin_dir.join("foo").is_file());
        assert!(bin_dir.join("bar").is_file());
    }

    #[tokio::test]
    /// Non-regression for #1563
    async fn builtin_build_support_src_init_lua() {
        let data_dir: PathBuf = assert_fs::TempDir::new().unwrap().path().into();
        let project_root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-projects/init/");
        let temp_dir = assert_fs::TempDir::new().unwrap();
        temp_dir.copy_from(&project_root, &["**"]).unwrap();
        let project_root = temp_dir.path();
        let src_dir = project_root.join("src");
        tokio::fs::create_dir_all(&src_dir).await.unwrap();
        let init_lua_file = src_dir.join("init.lua");
        tokio::fs::write(&init_lua_file, "print('hello')")
            .await
            .unwrap();
        let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));
        let config = ConfigBuilder::new()
            .unwrap()
            .data_dir(Some(data_dir))
            .lua_version(lua_version)
            .build()
            .unwrap();
        let workspace = Workspace::from_exact(project_root).unwrap().unwrap();
        let package = BuildWorkspace::new(&workspace, &config)
            .no_lock(false)
            .only_deps(false)
            .build()
            .await
            .unwrap();
        let package = package.first().unwrap();
        let tree = workspace.tree(&config).unwrap();
        let layout = tree.installed_rock_layout(package).unwrap();
        let src_dir = layout.src;
        assert!(src_dir.join("init.lua").is_file());
    }
}
