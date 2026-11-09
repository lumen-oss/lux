use crate::package;
use bon::Builder;
use miette::Diagnostic;
use thiserror::Error;

use crate::{
    build::{self, BuildBehaviour},
    config::Config,
    drivers::{InstallBuildDependencies, InstallPackages, InstallPackagesError},
    lockfile::{LockedPackage, OptState, PinnedState},
    lua,
    luarocks::installation,
    operations::install,
    package::db::PackageDB,
    pipeline::discover::FoundPackage,
    rockspec,
    rockspec::{LuaVersionCompatibility, Rockspec},
    tree::{self, InstallSession},
};

#[derive(Debug, Error, Diagnostic)]
#[error(transparent)]
#[non_exhaustive]
pub enum Error {
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
    LuaRocks(#[from] Box<installation::Error>),
    #[diagnostic(transparent)]
    Build(#[from] Box<build::Error>),
    #[error("package '{0}' was not installed")]
    PackageNotInstalled(package::Name),
}

impl From<InstallPackagesError> for Error {
    fn from(source: InstallPackagesError) -> Self {
        Self::Install(Box::new(source))
    }
}

impl From<installation::Error> for Error {
    fn from(source: installation::Error) -> Self {
        Self::LuaRocks(Box::new(source))
    }
}

impl From<build::Error> for Error {
    fn from(source: build::Error) -> Self {
        Self::Build(Box::new(source))
    }
}

/// Installs a Lua RockSpec into a [`Slot`](tree::Slot).
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct InstallRockspec<'a, T>
where
    T: InstallSession,
{
    rockspec: rockspec::lua::Remote,

    pin: PinnedState,

    config: &'a Config,

    target: &'a T,
}

impl<
        T: InstallSession + Sync + Send,
        State: install_rockspec_builder::State + install_rockspec_builder::IsComplete,
    > InstallRockspecBuilder<'_, T, State>
{
    pub async fn install(self) -> Result<LockedPackage, Error> {
        let args = self._build();
        let rockspec = args.rockspec;
        let pin = args.pin;
        let config = args.config;
        let destination = args.target;

        rockspec.lua_version_matches(config)?;

        let name = rockspec.package().clone();
        let root = FoundPackage::from_rockspec(rockspec);
        let package_db = PackageDB::from_config(config).await?.with_local(vec![root]);

        let install_spec = install::Spec::new(
            package::Req::from(name.clone()),
            tree::EntryType::Entrypoint,
        )
        .build_behaviour(BuildBehaviour::Force)
        .pin(pin)
        .opt(OptState::Required)
        .build();

        let build_transaction = destination.build_transaction()?;
        let build_dependencies = InstallBuildDependencies::new(config, &build_transaction)
            .package(install_spec.clone())
            .package_db(package_db.clone())
            .install()
            .await?;
        build_transaction.commit()?;

        InstallPackages::new(config, destination)
            .package(install_spec)
            .package_db(package_db)
            .install(&build_dependencies.token)
            .await?
            .0
            .into_iter()
            .find(|package| package.name() == &name)
            .ok_or(Error::PackageNotInstalled(name))
    }
}
