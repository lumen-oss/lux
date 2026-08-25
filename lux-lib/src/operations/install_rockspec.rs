use bon::Builder;
use miette::Diagnostic;
use thiserror::Error;

use crate::{
    build::{self, BuildBehaviour, BuildError},
    config::Config,
    lockfile::{LocalPackage, OptState, PinnedState},
    lua_installation::{LuaInstallation, LuaInstallationError},
    lua_rockspec::{BuildBackendSpec, LuaVersionError, RemoteLuaRockspec},
    luarocks::luarocks_installation::{LuaRocksError, LuaRocksInstallError, LuaRocksInstallation},
    operations::{Install, InstallError, PackageInstallSpec},
    rockspec::{LuaVersionCompatibility, Rockspec},
    tree::{self, InstallTree, TreeError},
};

#[derive(Debug, Error, Diagnostic)]
#[error(transparent)]
#[non_exhaustive]
pub enum InstallRockspecError {
    #[diagnostic(transparent)]
    LuaInstallation(#[from] LuaInstallationError),
    #[diagnostic(transparent)]
    LuaVersion(#[from] LuaVersionError),
    #[diagnostic(transparent)]
    Tree(#[from] TreeError),
    #[diagnostic(transparent)]
    Install(#[from] InstallError),
    #[diagnostic(transparent)]
    LuaRocks(#[from] LuaRocksError),
    #[diagnostic(transparent)]
    LuaRocksInstall(#[from] LuaRocksInstallError),
    #[diagnostic(transparent)]
    Build(#[from] BuildError),
}

/// Installs a Lua RockSpec into a [`Tree`].
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct InstallRockspec<'a, T>
where
    T: InstallTree,
{
    rockspec: RemoteLuaRockspec,

    pin: PinnedState,

    config: &'a Config,

    tree: &'a T,
}

impl<
        T: InstallTree + Sync + Send + Clone + 'static,
        State: install_rockspec_builder::State + install_rockspec_builder::IsComplete,
    > InstallRockspecBuilder<'_, T, State>
{
    pub async fn install(self) -> Result<LocalPackage, InstallRockspecError> {
        let args = self._build();
        let rockspec = args.rockspec;
        let pin = args.pin;
        let config = args.config;
        let tree = args.tree;

        let lua_version = rockspec.lua_version_matches(config)?;
        let lua = LuaInstallation::new(&lua_version, config).await?;

        // Ensure all dependencies and build dependencies are installed first

        let build_dependencies = rockspec.build_dependencies().current_platform();

        let build_dependencies_to_install = build_dependencies
            .iter()
            .filter(|dep| {
                // Exclude luarocks build backends that we have implemented in lux
                !matches!(
                    dep.name().to_string().as_str(),
                    "luarocks-build-rust-mlua" | "luarocks-build-treesitter-parser"
                )
            })
            .filter(|dep| {
                tree.match_rocks(dep.package_req())
                    .is_ok_and(|rock_match| rock_match.is_found())
            })
            .map(|dep| {
                PackageInstallSpec::new(dep.package_req().clone(), tree::EntryType::Entrypoint)
                    .build_behaviour(BuildBehaviour::NoForce)
                    .pin(pin)
                    .opt(OptState::Required)
                    .maybe_source(dep.source().clone())
                    .build()
            })
            .collect();

        Install::new(config)
            .packages(build_dependencies_to_install)
            .tree(tree.build_tree(config)?)
            .install()
            .await?;

        let dependencies = rockspec.dependencies().current_platform();

        let mut dependencies_to_install = Vec::new();
        for dep in dependencies {
            let rock_match = tree.match_rocks(dep.package_req())?;
            if !rock_match.is_found() {
                let dep = PackageInstallSpec::new(
                    dep.package_req().clone(),
                    tree::EntryType::DependencyOnly,
                )
                .build_behaviour(BuildBehaviour::NoForce)
                .pin(pin)
                .opt(OptState::Required)
                .maybe_source(dep.source().clone())
                .build();
                dependencies_to_install.push(dep);
            }
        }

        let dependencies = Install::new(config)
            .packages(dependencies_to_install)
            .tree(tree.clone())
            .install()
            .await?;

        if let Some(BuildBackendSpec::LuaRock(_)) =
            &rockspec.build().current_platform().build_backend
        {
            let build_tree = tree.build_tree(config)?;
            let luarocks = LuaRocksInstallation::new(config, build_tree)?;
            luarocks.ensure_installed(&lua).await?;
        }

        let package = build::Build::new()
            .rockspec(&rockspec)
            .tree(tree)
            .lua(&lua)
            .entry_type(tree::EntryType::Entrypoint)
            .config(config)
            .pin(pin)
            .behaviour(BuildBehaviour::Force)
            .build()
            .await?;

        let lockfile = tree.lockfile()?;
        let mut lockfile = lockfile.write_guard();
        lockfile.add_entrypoint(&package);
        for dep in dependencies {
            lockfile.add_dependency(&package, &dep);
        }
        Ok(package)
    }
}
