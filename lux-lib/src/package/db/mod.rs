use crate::package;

pub mod search;
use std::sync::Arc;

use crate::{
    config::{self, Config},
    lockfile::PackageLock,
    manifest::{self, Manifest},
    operations::install,
    pipeline::discover::{self, FoundPackage},
    remote,
    workspace::{self, Workspace},
};
use itertools::Itertools;
use tokio::sync::OnceCell;
use url::Url;

use miette::Diagnostic;
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct PackageDB {
    providers: Vec<Provider>,
}

#[derive(Error, Debug, Diagnostic)]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Manifest(#[from] manifest::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] config::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Workspace(Box<workspace::Error>),
}

impl From<workspace::Error> for Error {
    fn from(err: workspace::Error) -> Self {
        Self::Workspace(Box::new(err))
    }
}

/// A source of package metadata consulted by a [`PackageDB`].
#[derive(Clone, Debug)]
pub(crate) enum Provider {
    /// Locally available packages, e.g. workspace members.
    Local(Vec<FoundPackage>),
    /// Packages pinned in the lockfile.
    Lockfile(Vec<PackageLock>),
    /// A luarocks manifest.
    Manifest(ManifestProvider),
}

/// A luarocks manifest that is only fetched when it is first needed.
#[derive(Clone, Debug)]
pub(crate) struct ManifestProvider {
    server_url: Url,
    config: Box<Config>,
    manifest: Arc<OnceCell<Manifest>>,
}

impl ManifestProvider {
    fn new(server_url: Url, config: &Config) -> Self {
        Self {
            server_url,
            config: Box::new(config.clone()),
            manifest: Arc::new(OnceCell::new()),
        }
    }

    #[cfg(test)]
    fn from_manifest(manifest: Manifest, config: &Config) -> Self {
        Self {
            server_url: manifest.server_url().clone(),
            config: Box::new(config.clone()),
            manifest: Arc::new(OnceCell::from(manifest)),
        }
    }

    async fn get(&self) -> Result<&Manifest, manifest::Error> {
        let manifest = self
            .manifest
            .get_or_try_init(|| Manifest::from_config(self.server_url.clone(), &self.config));
        Box::pin(manifest).await
    }
}

impl Provider {
    async fn find(
        &self,
        package_req: &package::Req,
        filter: Option<remote::package::kind::FilterSpec>,
    ) -> Result<Option<remote::Package>, search::Error> {
        match self {
            Self::Local(packages) => Ok(packages
                .iter()
                .find(|found| found.remote.spec.name() == package_req.name())
                .map(|found| found.remote.clone())),
            Self::Lockfile(locks) => Ok(locks
                .iter()
                .filter_map(|lock| lock.has_rock(package_req, filter.clone()))
                .map(|locked_package| {
                    remote::Package::new(
                        locked_package.to_package(),
                        locked_package.source().clone(),
                        locked_package.source_url().cloned(),
                    )
                })
                .next()),
            Self::Manifest(manifest) => Ok(manifest.get().await?.find(package_req, filter)),
        }
    }

    async fn resolve(&self, spec: &install::Spec) -> Result<Option<FoundPackage>, discover::Error> {
        match self {
            Self::Local(packages) => Ok(packages
                .iter()
                .find(|package| package.remote.spec.name() == spec.package.name())
                .cloned()),
            Self::Lockfile(_) | Self::Manifest(_) => Ok(None),
        }
    }

    async fn search(
        &self,
        package_req: &package::Req,
    ) -> Result<Vec<(package::Name, Vec<package::Version>)>, search::Error> {
        match self {
            Self::Local(packages) => Ok(packages
                .iter()
                .filter_map(|package| {
                    let name = package.remote.spec.name();
                    if name.to_string().contains(&package_req.name().to_string())
                        && package_req
                            .version_req()
                            .matches(package.remote.spec.version())
                    {
                        Some((name.clone(), vec![package.remote.spec.version().clone()]))
                    } else {
                        None
                    }
                })
                .collect()),
            // NOTE: This doesn't group packages by name, but we don't care for now,
            // as we shouldn't need to use this function with a lockfile.
            Self::Lockfile(locks) => Ok(locks
                .iter()
                .flat_map(|lock| lock.rocks().values())
                .filter_map(|package| {
                    let name = package.name();
                    if name.to_string().contains(&package_req.name().to_string()) {
                        Some((name.clone(), vec![package.version().clone()]))
                    } else {
                        None
                    }
                })
                .collect()),
            Self::Manifest(manifest) => Ok(manifest
                .get()
                .await?
                .metadata()
                .repository
                .iter()
                .filter_map(|(name, elements)| {
                    if name.to_string().contains(&package_req.name().to_string()) {
                        Some((
                            name.clone(),
                            elements
                                .keys()
                                .filter(|version| package_req.version_req().matches(version))
                                .sorted_by(|a, b| Ord::cmp(b, a))
                                .cloned()
                                .collect_vec(),
                        ))
                    } else {
                        None
                    }
                })
                .collect()),
        }
    }
}

impl PackageDB {
    #[tracing::instrument(level = "trace", skip_all)]
    pub async fn from_config(config: &Config) -> Result<Self, Error> {
        let mut providers = Vec::new();
        for server in config.enabled_dev_servers()? {
            providers.push(Provider::Manifest(ManifestProvider::new(server, config)));
        }
        for server in config.extra_servers() {
            providers.push(Provider::Manifest(ManifestProvider::new(
                server.clone(),
                config,
            )));
        }
        providers.push(Provider::Manifest(ManifestProvider::new(
            config.server().clone(),
            config,
        )));
        Ok(Self { providers })
    }

    /// Builds a package database from a single, already loaded manifest.
    #[cfg(test)]
    pub(crate) fn from_manifest(manifest: Manifest, config: &Config) -> Self {
        Self {
            providers: vec![Provider::Manifest(ManifestProvider::from_manifest(
                manifest, config,
            ))],
        }
    }

    /// Builds a package database that prefers the workspace lockfile, falling
    /// back to the manifests for requirements the lockfile does not satisfy.
    pub(crate) async fn open(config: &Config, workspace: &Workspace) -> Result<Self, Error> {
        Ok(Self::from_config(config)
            .await?
            .with_locks(workspace.lockfile()?.local_pkg_locks()))
    }

    #[cfg(test)]
    pub(crate) fn from_local(packages: Vec<FoundPackage>) -> Self {
        Self {
            providers: vec![Provider::Local(packages)],
        }
    }

    /// Sets the local packages to look up first.
    pub(crate) fn with_local(mut self, packages: Vec<FoundPackage>) -> Self {
        self.providers
            .retain(|provider| !matches!(provider, Provider::Local(_)));
        self.providers.insert(0, Provider::Local(packages));
        self
    }

    /// Sets the locked packages to look up after local packages, ahead of manifests.
    pub(crate) fn with_locks(mut self, locks: Vec<PackageLock>) -> Self {
        self.providers
            .retain(|provider| !matches!(provider, Provider::Lockfile(_)));
        let index = self
            .providers
            .iter()
            .position(|provider| matches!(provider, Provider::Manifest(_)))
            .unwrap_or(self.providers.len());
        self.providers.insert(index, Provider::Lockfile(locks));
        self
    }

    pub(crate) fn has_remote_sources(&self) -> bool {
        self.providers
            .iter()
            .any(|provider| matches!(provider, Provider::Lockfile(_) | Provider::Manifest(_)))
    }
}

impl From<Vec<PackageLock>> for PackageDB {
    fn from(locks: Vec<PackageLock>) -> Self {
        Self {
            providers: vec![Provider::Lockfile(locks)],
        }
    }
}
