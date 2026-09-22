use crate::{
    config::Config,
    lockfile::LockedPackage,
    remote_package_db::RemotePackageDB,
    tree::{InstallTree, Tree},
    workspace::{Workspace, WorkspaceTreeError},
};

pub use crate::operations::install::spec::PackageInstallSpec;

use bon::Builder;
use miette::Diagnostic;
use thiserror::Error;

use tracing::Instrument;

use super::pipeline::install_packages::{InstallPackages, InstallPackagesError};
pub mod spec;

/// A rocks package installer, providing fine-grained control
/// over how packages should be installed.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Install<'a, T>
where
    T: InstallTree + Clone + Send + Sync,
{
    #[builder(start_fn)]
    config: &'a Config,
    #[builder(field)]
    packages: Vec<PackageInstallSpec>,
    #[builder(setters(name = "_tree", vis = ""))]
    tree: T,
    package_db: Option<RemotePackageDB>,
}

impl<'a, State> InstallBuilder<'a, Tree, State>
where
    State: install_builder::State,
{
    pub fn workspace(
        self,
        workspace: &'a Workspace,
    ) -> Result<InstallBuilder<'a, Tree, install_builder::SetTree<State>>, WorkspaceTreeError>
    where
        State::Tree: install_builder::IsUnset,
    {
        let config = self.config;
        Ok(self._tree(workspace.tree(config)?))
    }
}

impl<'a, T, State> InstallBuilder<'a, T, State>
where
    State: install_builder::State,
    T: InstallTree + Clone + Send + Sync,
{
    pub fn tree(self, tree: T) -> InstallBuilder<'a, T, install_builder::SetTree<State>>
    where
        State::Tree: install_builder::IsUnset,
    {
        self._tree(tree)
    }

    pub fn packages(self, packages: Vec<PackageInstallSpec>) -> Self {
        Self { packages, ..self }
    }

    pub fn package(self, package: PackageInstallSpec) -> Self {
        Self {
            packages: self
                .packages
                .into_iter()
                .chain(std::iter::once(package))
                .collect(),
            ..self
        }
    }
}

impl<State, T> InstallBuilder<'_, T, State>
where
    State: install_builder::State + install_builder::IsComplete,
    T: InstallTree + Clone + Send + Sync + 'static,
{
    /// Install the packages.
    pub async fn install(self) -> Result<Vec<LockedPackage>, InstallError> {
        let install_built = self._build();
        if install_built.packages.is_empty() {
            return Ok(Vec::default());
        }
        let count = install_built.packages.len();
        let span = if count > 1 {
            tracing::info_span!("Installing", count)
        } else {
            let install_spec = &install_built.packages[0];
            tracing::info_span!("Installing", package = install_spec.package.to_string())
        };
        install_impl(install_built).instrument(span).await
    }
}

#[derive(Error, Debug, Diagnostic)]
pub enum InstallError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaVersionUnset(#[from] crate::lua_version::LuaVersionUnset),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaInstallation(#[from] crate::lua_installation::LuaInstallationError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    FlushLockfile(#[from] crate::lockfile::FlushLockfileError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] crate::tree::TreeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    WorkspaceTree(#[from] WorkspaceTreeError),
    #[error("error instantiating LuaRocks compatibility layer")]
    #[diagnostic(forward(0))]
    LuaRocks(#[from] crate::luarocks::luarocks_installation::LuaRocksError),
    #[error("error installing LuaRocks compatibility layer")]
    #[diagnostic(forward(0))]
    LuaRocksInstall(#[from] Box<crate::luarocks::luarocks_installation::LuaRocksInstallError>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Install(#[from] InstallPackagesError),
}

impl From<crate::luarocks::luarocks_installation::LuaRocksInstallError> for InstallError {
    fn from(
        source: crate::luarocks::luarocks_installation::LuaRocksInstallError,
    ) -> Self {
        Self::LuaRocksInstall(Box::new(source))
    }
}

async fn install_impl<T>(install: Install<'_, T>) -> Result<Vec<LockedPackage>, InstallError>
where
    T: InstallTree + Clone + Send + Sync + 'static,
{
    let packages = install.packages;
    if packages.is_empty() {
        return Ok(Vec::new());
    }

    InstallPackages::new(install.config, install.tree)
        .packages(packages)
        .maybe_package_db(install.package_db)
        .install()
        .await
        .map_err(InstallError::from)
}
