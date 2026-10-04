use bon::Builder;
use miette::Diagnostic;
use thiserror::Error;

use crate::{
    build::{BuildBehaviour, BuildError},
    config::Config,
    lockfile::{LockedPackage, OptState, PinnedState},
    lua_installation::LuaInstallationError,
    lua_rockspec::{LuaVersionError, RemoteLuaRockspec},
    luarocks::luarocks_installation::{LuaRocksError, LuaRocksInstallError},
    operations::{pipeline::discover::FoundPackage, Install, InstallError, PackageInstallSpec},
    package::{PackageName, PackageReq},
    remote_package_db::{RemotePackageDB, RemotePackageDBError},
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
    RemotePackageDB(#[from] RemotePackageDBError),
    #[diagnostic(transparent)]
    Tree(#[from] TreeError),
    #[diagnostic(transparent)]
    Install(#[from] Box<InstallError>),
    #[diagnostic(transparent)]
    LuaRocks(#[from] LuaRocksError),
    #[diagnostic(transparent)]
    LuaRocksInstall(#[from] Box<LuaRocksInstallError>),
    #[diagnostic(transparent)]
    Build(#[from] Box<BuildError>),
    #[error("package '{0}' was not installed")]
    PackageNotInstalled(PackageName),
}

impl From<InstallError> for InstallRockspecError {
    fn from(source: InstallError) -> Self {
        Self::Install(Box::new(source))
    }
}

impl From<LuaRocksInstallError> for InstallRockspecError {
    fn from(source: LuaRocksInstallError) -> Self {
        Self::LuaRocksInstall(Box::new(source))
    }
}

impl From<BuildError> for InstallRockspecError {
    fn from(source: BuildError) -> Self {
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
    pub async fn install(self) -> Result<LockedPackage, InstallRockspecError> {
        let args = self._build();
        let rockspec = args.rockspec;
        let pin = args.pin;
        let config = args.config;
        let tree = args.tree;

        rockspec.lua_version_matches(config)?;

        let name = rockspec.package().clone();
        let root = FoundPackage::from_rockspec(rockspec);
        let package_db = RemotePackageDB::from_config(config)
            .await?
            .with_local(vec![root]);

        let install_spec =
            PackageInstallSpec::new(PackageReq::from(name.clone()), tree::EntryType::Entrypoint)
                .build_behaviour(BuildBehaviour::Force)
                .pin(pin)
                .opt(OptState::Required)
                .build();

        Install::new(config)
            .package_db(package_db)
            .package(install_spec)
            .tree(tree)
            .install()
            .await?
            .into_iter()
            .find(|package| package.name() == &name)
            .ok_or(InstallRockspecError::PackageNotInstalled(name))
    }
}
