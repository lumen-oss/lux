use std::sync::Arc;

use bon::Builder;
use itertools::Itertools;

use crate::{
    config::Config,
    lua_installation::LuaInstallation,
    luarocks::luarocks_installation::LuaRocksInstallation,
    operations::{Install, InstallError},
    progress::{MultiProgress, Progress},
    project::project_toml::LocalProjectToml,
    rockspec::Rockspec,
    tree::{self, InstallTree},
};

use super::PackageInstallSpec;

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(crate) struct InstallDependencies<'a, T>
where
    T: InstallTree,
{
    dependencies: Vec<PackageInstallSpec>,
    build_dependencies: Vec<PackageInstallSpec>,
    tree: &'a T,

    lua: &'a LuaInstallation,
    luarocks: &'a LuaRocksInstallation,
    config: &'a Config,
    progress: Arc<Progress<MultiProgress>>,
}

impl<
        T: InstallTree + Sync + Send + Clone + 'static,
        State: install_dependencies_builder::State + install_dependencies_builder::IsComplete,
    > InstallDependenciesBuilder<'_, T, State>
{
    pub(crate) async fn build(self) -> Result<(), InstallError> {
        let args = self._build();
        let config = args.config;
        let dependencies = args.dependencies;
        let build_dependencies = args.build_dependencies;
        let tree = args.tree;
        let build_tree = tree.build_tree(config)?;
        let lua = args.lua;
        let luarocks = args.luarocks;
        let progress_arc = args.progress;
        if !build_dependencies.is_empty() {
            let bar = progress_arc.map(|p| p.new_bar());
            luarocks.ensure_installed(lua, &bar).await?;
            Install::new(config)
                .packages(build_dependencies.into_iter().unique().collect_vec())
                .tree(build_tree.clone())
                .progress(progress_arc.clone())
                .install()
                .await?;
        }
        // for some reason, cargo can't infer the type
        Install::new(config)
            .packages(dependencies.into_iter().unique().collect_vec())
            .tree(tree.clone())
            .progress(progress_arc.clone())
            .install()
            .await?;
        Ok(())
    }
}

pub(crate) fn prepare_dependencies_for_build(
    project_toml: &LocalProjectToml,
    workspace_tree: &impl InstallTree,
    dependencies_to_install: &mut Vec<PackageInstallSpec>,
    build_dependencies_to_install: &mut Vec<PackageInstallSpec>,
) {
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
    dependencies
        .into_iter()
        .filter(|dep| {
            workspace_tree
                .match_rocks(dep.package_req())
                .is_ok_and(|rock_match| !rock_match.is_found())
        })
        .map(|dep| {
            PackageInstallSpec::new(dep.clone().into_package_req(), tree::EntryType::Entrypoint)
                .pin(*dep.pin())
                .opt(*dep.opt())
                .maybe_source(dep.source().clone())
                .build()
        })
        .for_each(|dep| dependencies_to_install.push(dep));

    build_dependencies
        .into_iter()
        .filter(|dep| {
            workspace_tree
                .match_rocks(dep.package_req())
                .is_ok_and(|rock_match| !rock_match.is_found())
        })
        .map(|dep| {
            PackageInstallSpec::new(dep.clone().into_package_req(), tree::EntryType::Entrypoint)
                .pin(*dep.pin())
                .opt(*dep.opt())
                .maybe_source(dep.source().clone())
                .build()
        })
        .for_each(|dep| build_dependencies_to_install.push(dep));
}
