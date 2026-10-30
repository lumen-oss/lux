use crate::package;
use bon::Builder;
use miette::Diagnostic;
use thiserror::Error;

use crate::{
    build::{self, BuildBehaviour},
    config::Config,
    drivers::{InstallPackages, InstallPackagesError},
    lockfile::{LockedPackage, OptState, PinnedState},
    lua,
    luarocks::luarocks_installation::{LuaRocksError, LuaRocksInstallError},
    operations::PackageInstallSpec,
    package::db::PackageDB,
    pipeline::discover::FoundPackage,
    rockspec,
    rockspec::{LuaVersionCompatibility, Rockspec},
    tree::{self, InstallTree},
};

#[derive(Debug, Error, Diagnostic)]
#[error(transparent)]
#[non_exhaustive]
pub enum InstallRockspecError {
    #[diagnostic(transparent)]
    LuaInstallation(#[from] lua::installation::Error),
    #[diagnostic(transparent)]
    LuaVersion(#[from] rockspec::Error),
    #[diagnostic(transparent)]
    PackageDB(#[from] package::db::Error),
    #[diagnostic(transparent)]
    Tree(#[from] tree::Error),
    #[diagnostic(transparent)]
    Install(#[from] Box<InstallPackagesError>),
    #[diagnostic(transparent)]
    LuaRocks(#[from] LuaRocksError),
    #[diagnostic(transparent)]
    LuaRocksInstall(#[from] Box<LuaRocksInstallError>),
    #[diagnostic(transparent)]
    Build(#[from] Box<build::Error>),
    #[error("package '{0}' was not installed")]
    PackageNotInstalled(package::Name),
}

impl From<InstallPackagesError> for InstallRockspecError {
    fn from(source: InstallPackagesError) -> Self {
        Self::Install(Box::new(source))
    }
}

impl From<LuaRocksInstallError> for InstallRockspecError {
    fn from(source: LuaRocksInstallError) -> Self {
        Self::LuaRocksInstall(Box::new(source))
    }
}

impl From<build::Error> for InstallRockspecError {
    fn from(source: build::Error) -> Self {
        Self::Build(Box::new(source))
    }
}

/// Installs a Lua RockSpec into a [`Tree`].
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct InstallRockspec<'a, T>
where
    T: InstallTree,
{
    rockspec: rockspec::lua::Remote,

    pin: PinnedState,

    config: &'a Config,

    tree: &'a T,
}

impl<
        T: InstallTree + Sync + Send + Clone + 'static,
        State: install_rockspec_builder::State + install_rockspec_builder::IsComplete,
    > InstallRockspecBuilder<'_, T, State>
{
    pub async fn install(self) -> Result<LockedPackage, InstallRockspecError> {
        let args = self._build();
        let rockspec = args.rockspec;
        let pin = args.pin;
        let config = args.config;
        let tree = args.tree;

        rockspec.lua_version_matches(config)?;

        let name = rockspec.package().clone();
        let root = FoundPackage::from_rockspec(rockspec);
        let package_db = PackageDB::from_config(config).await?.with_local(vec![root]);

        let install_spec = PackageInstallSpec::new(
            package::Req::from(name.clone()),
            tree::EntryType::Entrypoint,
        )
        .build_behaviour(BuildBehaviour::Force)
        .pin(pin)
        .opt(OptState::Required)
        .build();

        InstallPackages::new(config, tree)
            .package_db(package_db)
            .package(install_spec)
            .install()
            .await?
            .0
            .into_iter()
            .find(|package| package.name() == &name)
            .ok_or(InstallRockspecError::PackageNotInstalled(name))
    }
}
