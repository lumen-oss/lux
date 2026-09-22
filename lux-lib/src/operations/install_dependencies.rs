use bon::Builder;
use itertools::Itertools;

use crate::{
    config::Config,
    lockfile::LockedPackage,
    lua_installation::LuaInstallation,
    luarocks::luarocks_installation::LuaRocksInstallation,
    operations::{InstallError, pipeline::install_packages::InstallPackages},
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
}

impl<
        T: InstallTree + Sync + Send + Clone + 'static,
        State: install_dependencies_builder::State + install_dependencies_builder::IsComplete,
    > InstallDependenciesBuilder<'_, T, State>
{
    /// Installs the configured dependencies and build dependencies into [`Self::tree`],
    /// returning the installed regular dependencies.
    pub(crate) async fn build(self) -> Result<Vec<LockedPackage>, InstallError> {
        let args = self._build();
        let config = args.config;
        let dependencies = args.dependencies;
        let build_dependencies = args.build_dependencies;
        let tree = args.tree;
        let build_tree = tree.build_tree(config)?;
        let lua = args.lua;
        let luarocks = args.luarocks;
        if !build_dependencies.is_empty() {
            luarocks.ensure_installed(lua).await?;
            InstallPackages::new(config, build_tree.clone())
                .packages(build_dependencies.into_iter().unique().collect_vec())
                .install()
                .await
                .map_err(InstallError::from)?;
        }
        let dependencies = InstallPackages::new(config, (*tree).clone())
            .packages(dependencies.into_iter().unique().collect_vec())
            .install()
            .await
            .map_err(InstallError::from)?;
        Ok(dependencies)
    }
}

pub(crate) fn prepare_dependencies_for_build(
    project_toml: &LocalProjectToml,
    workspace_tree: &impl InstallTree,
    dependencies_to_install: &mut Vec<PackageInstallSpec>,
    build_dependencies_to_install: &mut Vec<PackageInstallSpec>,
    entry_type: tree::EntryType,
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
            PackageInstallSpec::new(dep.clone().into_package_req(), entry_type)
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
            PackageInstallSpec::new(dep.clone().into_package_req(), entry_type)
                .pin(*dep.pin())
                .opt(*dep.opt())
                .maybe_source(dep.source().clone())
                .build()
        })
        .for_each(|dep| build_dependencies_to_install.push(dep));
}
