use std::sync::Arc;

use bon::Builder;
use itertools::Itertools;
use thiserror::Error;

use crate::{
    build::{Build, BuildBehaviour, BuildError},
    config::Config,
    lockfile::LocalPackage,
    lua_installation::{LuaInstallation, LuaInstallationError},
    luarocks::luarocks_installation::{LuaRocksError, LuaRocksInstallError, LuaRocksInstallation},
    progress::{MultiProgress, Progress},
    project::{project_toml::LocalProjectTomlValidationError, Project, ProjectTreeError},
    rockspec::Rockspec,
    tree::{self, TreeError},
};

use super::{Install, InstallError, PackageInstallSpec, Sync, SyncError};

#[derive(Debug, Error)]
pub enum BuildProjectError {
    #[error(transparent)]
    LocalProjectTomlValidation(#[from] LocalProjectTomlValidationError),
    #[error(transparent)]
    ProjectTree(#[from] ProjectTreeError),
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
pub struct BuildProject<'a> {
    #[builder(start_fn)]
    project: &'a Project,

    #[builder(start_fn)]
    config: &'a Config,

    /// Ignore the project's lockfile and don't create one
    no_lock: bool,

    /// Build only the dependencies
    only_deps: bool,

    progress: Option<Arc<Progress<MultiProgress>>>,
}

impl<State: build_project_builder::State + build_project_builder::IsComplete>
    BuildProjectBuilder<'_, State>
{
    /// Returns `Some` if the `only_deps` option is set to `false`.
    pub async fn build(self) -> Result<Option<LocalPackage>, BuildProjectError> {
        let args = self._build();
        let project = args.project;
        let config = args.config;
        let progress_arc = args
            .progress
            .unwrap_or_else(|| MultiProgress::new_arc(config));
        let progress = Arc::clone(&progress_arc);

        let project_toml = project.toml().into_local()?;
        let project_tree = project.tree(config)?;

        let dependencies = project_toml
            .dependencies()
            .current_platform()
            .iter()
            .cloned()
            .collect_vec();

        let build_dependencies = project_toml
            .build_dependencies()
            .current_platform()
            .iter()
            .cloned()
            .collect_vec();

        let build_tree = project.build_tree(config)?;
        let lua =
            LuaInstallation::new_from_config(config, &progress.map(|progress| progress.new_bar()))
                .await?;
        let luarocks = LuaRocksInstallation::new(config, build_tree.clone())?;

        if args.no_lock {
            let dependencies_to_install = dependencies
                .into_iter()
                .filter(|dep| {
                    project_tree
                        .match_rocks(dep.package_req())
                        .is_ok_and(|rock_match| !rock_match.is_found())
                })
                .map(|dep| {
                    PackageInstallSpec::new(
                        dep.clone().into_package_req(),
                        tree::EntryType::Entrypoint,
                    )
                    .pin(*dep.pin())
                    .opt(*dep.opt())
                    .maybe_source(dep.source().clone())
                    .build()
                })
                .collect();

            Install::new(config)
                .packages(dependencies_to_install)
                .project(project)?
                .progress(progress.clone())
                .install()
                .await
                .map_err(BuildProjectError::InstallDependencies)?;

            let build_dependencies_to_install = build_dependencies
                .into_iter()
                .filter(|dep| {
                    project_tree
                        .match_rocks(dep.package_req())
                        .is_ok_and(|rock_match| !rock_match.is_found())
                })
                .map(|dep| {
                    PackageInstallSpec::new(
                        dep.clone().into_package_req(),
                        tree::EntryType::Entrypoint,
                    )
                    .pin(*dep.pin())
                    .opt(*dep.opt())
                    .maybe_source(dep.source().clone())
                    .build()
                })
                .collect_vec();

            if !build_dependencies_to_install.is_empty() {
                let bar = progress.map(|p| p.new_bar());
                luarocks.ensure_installed(&lua, &bar).await?;
                Install::new(config)
                    .packages(build_dependencies_to_install)
                    .tree(build_tree)
                    .progress(progress.clone())
                    .install()
                    .await
                    .map_err(BuildProjectError::InstallBuildDependencies)?;
            }
        } else {
            Sync::new(project, config)
                .progress(progress.clone())
                .sync_dependencies()
                .await
                .map_err(BuildProjectError::SyncDependencies)?;

            Sync::new(project, config)
                .progress(progress.clone())
                .sync_build_dependencies()
                .await
                .map_err(BuildProjectError::SyncBuildDependencies)?;
        }

        if !args.only_deps {
            let package = Build::new()
                .rockspec(&project_toml)
                .lua(&lua)
                .tree(&project_tree)
                .entry_type(tree::EntryType::Entrypoint)
                .config(config)
                .progress(&progress.map(|p| p.new_bar()))
                .behaviour(BuildBehaviour::Force)
                .build()
                .await?;

            let lockfile = project_tree.lockfile()?;
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
            Ok(Some(package))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{
        config::{ConfigBuilder, LuaVersion},
        lua_installation::detect_installed_lua_version,
        project::Project,
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
        let project = Project::from_exact(project_root).unwrap().unwrap();
        let tree = project.tree(&config).unwrap();
        let package = BuildProject::new(&project, &config)
            .no_lock(false)
            .only_deps(false)
            .build()
            .await
            .unwrap()
            .unwrap();
        let layout = tree.installed_rock_layout(&package).unwrap();
        let bin_dir = layout.bin;
        assert!(bin_dir.join("foo").is_file());
        assert!(bin_dir.join("bar").is_file());
    }
}
