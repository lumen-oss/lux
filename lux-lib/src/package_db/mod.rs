use std::{collections::HashMap, sync::Arc};

use crate::{
    config::{Config, ConfigError},
    lockfile::{LockfileIntegrityError, PackageLock},
    lua_rockspec::RemoteLuaRockspec,
    manifest::{Manifest, ManifestError},
    manifest_cache::{Closure, ClosureSpec, ManifestCache},
    operations::{Download, FetchVendored, PackageInstallSpec, RemoteRockDownload},
    package::{
        PackageName, PackageReq, PackageSpec, PackageVersion, RemotePackage, RemotePackageType,
        RemotePackageTypeFilterSpec,
    },
    pipeline::{
        discover::{DiscoverError, FoundPackage},
        materialise::PackageSource,
    },
    remote_package_source::RemotePackageSource,
    workspace::{Workspace, WorkspaceError},
};
use itertools::Itertools;
use tokio::sync::Mutex;
use url::Url;

use miette::Diagnostic;
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct PackageDB {
    providers: Vec<PackageProvider>,
}

#[derive(Error, Debug, Diagnostic)]
pub enum PackageDBError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    ManifestError(#[from] ManifestError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    ConfigError(#[from] ConfigError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Workspace(Box<WorkspaceError>),
}

impl From<WorkspaceError> for PackageDBError {
    fn from(err: WorkspaceError) -> Self {
        Self::Workspace(Box::new(err))
    }
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum SearchError {
    #[error("no rock that matches '{0}' found in the lockfile or in the manifests")]
    RockNotFound(PackageReq),
    #[error("error when pulling manifest")]
    #[diagnostic(forward(0))]
    Manifest(#[from] ManifestError),
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum RemotePackageDbIntegrityError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Lockfile(#[from] LockfileIntegrityError),
}

/// A source of package metadata consulted by a [`PackageDB`].
#[derive(Clone, Debug)]
pub(crate) enum PackageProvider {
    /// Locally available packages, e.g. workspace members.
    Local(Vec<FoundPackage>),
    /// Packages pinned in the lockfile.
    Lockfile(Vec<PackageLock>),
    /// A luarocks manifest.
    Manifest(Manifest),
    /// The manifest cache.
    Closure(ClosureProvider),
}

impl PackageProvider {
    async fn find(
        &self,
        package_req: &PackageReq,
        filter: Option<RemotePackageTypeFilterSpec>,
    ) -> Option<RemotePackage> {
        match self {
            Self::Local(packages) => packages
                .iter()
                .find(|found| found.remote.spec.name() == package_req.name())
                .map(|found| found.remote.clone()),
            Self::Lockfile(locks) => locks
                .iter()
                .filter_map(|lock| lock.has_rock(package_req, filter.clone()))
                .map(|locked_package| {
                    RemotePackage::new(
                        locked_package.to_package(),
                        locked_package.source().clone(),
                        locked_package.source_url().cloned(),
                    )
                })
                .next(),
            Self::Manifest(manifest) => manifest.find(package_req, filter),
            Self::Closure(closure) => closure.find(package_req, filter).await,
        }
    }

    async fn resolve(
        &self,
        spec: &PackageInstallSpec,
        config: &Config,
    ) -> Result<Option<FoundPackage>, DiscoverError> {
        match self {
            Self::Local(packages) => Ok(packages
                .iter()
                .find(|package| package.remote.spec.name() == spec.package.name())
                .cloned()),
            Self::Closure(closure) => closure.resolve(spec, config).await,
            Self::Lockfile(_) | Self::Manifest(_) => Ok(None),
        }
    }

    async fn search(&self, package_req: &PackageReq) -> Vec<(PackageName, Vec<PackageVersion>)> {
        match self {
            Self::Local(packages) => packages
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
                .collect(),
            // NOTE: This doesn't group packages by name, but we don't care for now,
            // as we shouldn't need to use this function with a lockfile.
            Self::Lockfile(locks) => locks
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
                .collect(),
            Self::Manifest(manifest) => manifest
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
                .collect(),
            // The cache only supports lookups by exact name, not substring search.
            Self::Closure(_) => Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ClosureProvider {
    cache: ManifestCache,
    rock_server: Url,
    loaded: Arc<Mutex<HashMap<PackageName, Arc<Closure>>>>,
}

impl ClosureProvider {
    fn new(cache: ManifestCache, config: &Config) -> Self {
        Self {
            cache,
            rock_server: config.server().clone(),
            loaded: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn load_closure_for(&self, req: &PackageReq) -> Option<Arc<Closure>> {
        if let Some(closure) = self.loaded.lock().await.get(req.name()) {
            return Some(closure.clone());
        }

        let versions = self.cache.versions(req.name()).await.ok()?;
        let chosen = versions
            .iter()
            .find(|version| req.version_req().matches(version))?;

        let is_latest = versions.first().is_some_and(|latest| latest == chosen);
        let closure = if is_latest {
            self.cache.latest_closure(req.name()).await.ok()?
        } else {
            self.cache.closure(req.name(), chosen).await.ok()?
        };
        let closure = Arc::new(closure);

        self.loaded
            .lock()
            .await
            .insert(req.name().clone(), closure.clone());
        Some(closure)
    }

    fn remote_package(&self, spec: &ClosureSpec, package_type: RemotePackageType) -> RemotePackage {
        let source = match package_type {
            RemotePackageType::Rockspec => {
                RemotePackageSource::LuarocksRockspec(self.rock_server.clone())
            }
            RemotePackageType::Src => {
                RemotePackageSource::LuarocksSrcRock(self.rock_server.clone())
            }
            RemotePackageType::Binary => {
                RemotePackageSource::LuarocksBinaryRock(self.rock_server.clone())
            }
        };
        RemotePackage::new(
            PackageSpec::new(spec.package().clone(), spec.version().clone()),
            source,
            None,
        )
    }

    async fn find(
        &self,
        req: &PackageReq,
        filter: Option<RemotePackageTypeFilterSpec>,
    ) -> Option<RemotePackage> {
        let closure = self.load_closure_for(req).await?;
        let (spec, package_type) = closure.spec(req, filter.as_ref())?;
        Some(self.remote_package(spec, package_type))
    }

    async fn resolve(
        &self,
        spec: &PackageInstallSpec,
        _config: &Config,
    ) -> Result<Option<FoundPackage>, DiscoverError> {
        let Some(closure) = self.load_closure_for(&spec.package).await else {
            return Ok(None);
        };
        let Some((closure_spec, package_type)) = closure.spec(&spec.package, None) else {
            return Ok(None);
        };
        // Only rockspec-type packages can be resolved fully offline, as they have no source archive
        // to download and hash. For src/binary rocks we fall through to the generic download step,
        // which fetches the archive.
        if package_type != RemotePackageType::Rockspec {
            return Ok(None);
        }
        let rockspec = RemoteLuaRockspec::new(closure_spec.rockspec_raw()).map_err(|err| {
            DiscoverError::Download(
                spec.package.clone(),
                Box::new(crate::operations::SearchAndDownloadError::Rockspec(
                    Box::new(err),
                )),
            )
        })?;
        let remote = self.remote_package(closure_spec, package_type);
        Ok(Some(FoundPackage {
            remote,
            rockspec,
            artifact: None,
        }))
    }
}

impl PackageDB {
    #[tracing::instrument(level = "trace", skip_all)]
    pub async fn from_config(config: &Config) -> Result<Self, PackageDBError> {
        let mut providers = Vec::new();
        if let Some(cache) = ManifestCache::from_config(config) {
            providers.push(PackageProvider::Closure(ClosureProvider::new(
                cache, config,
            )));
        }
        for server in config.enabled_dev_servers()? {
            let manifest = Manifest::from_config(server, config).await?;
            providers.push(PackageProvider::Manifest(manifest));
        }
        for server in config.extra_servers() {
            let manifest = Manifest::from_config(server.clone(), config).await?;
            providers.push(PackageProvider::Manifest(manifest));
        }
        providers.push(PackageProvider::Manifest(
            Manifest::from_config(config.server().clone(), config).await?,
        ));
        Ok(Self { providers })
    }

    /// Builds a package database that prefers the workspace lockfile, falling
    /// back to the manifests for requirements the lockfile does not satisfy.
    pub(crate) async fn open(
        config: &Config,
        workspace: &Workspace,
    ) -> Result<Self, PackageDBError> {
        Ok(Self::from_config(config)
            .await?
            .with_locks(workspace.lockfile()?.local_pkg_locks()))
    }

    #[cfg(test)]
    pub(crate) fn from_local(packages: Vec<FoundPackage>) -> Self {
        Self {
            providers: vec![PackageProvider::Local(packages)],
        }
    }

    /// Sets the local packages to look up first.
    pub(crate) fn with_local(mut self, packages: Vec<FoundPackage>) -> Self {
        self.providers
            .retain(|provider| !matches!(provider, PackageProvider::Local(_)));
        self.providers.insert(0, PackageProvider::Local(packages));
        self
    }

    /// Sets the locked packages to look up after local packages, ahead of manifests.
    pub(crate) fn with_locks(mut self, locks: Vec<PackageLock>) -> Self {
        self.providers
            .retain(|provider| !matches!(provider, PackageProvider::Lockfile(_)));
        let index = self
            .providers
            .iter()
            .position(|provider| {
                matches!(
                    provider,
                    PackageProvider::Closure(_) | PackageProvider::Manifest(_)
                )
            })
            .unwrap_or(self.providers.len());
        self.providers
            .insert(index, PackageProvider::Lockfile(locks));
        self
    }

    fn has_remote_sources(&self) -> bool {
        self.providers.iter().any(|provider| {
            matches!(
                provider,
                PackageProvider::Lockfile(_)
                    | PackageProvider::Manifest(_)
                    | PackageProvider::Closure(_)
            )
        })
    }

    /// Find a remote package that matches the requirement, returning the latest match.
    #[tracing::instrument(
          name = "Searching",
          level = "info",
          skip_all,
          fields(package = package_req.to_string()),
    )]
    pub(crate) async fn find(
        &self,
        package_req: &PackageReq,
        filter: Option<RemotePackageTypeFilterSpec>,
    ) -> Result<RemotePackage, SearchError> {
        for provider in &self.providers {
            if let Some(package) = provider.find(package_req, filter.clone()).await {
                return Ok(package);
            }
        }

        Err(SearchError::RockNotFound(package_req.clone()))
    }

    /// Resolves a package install spec into a fully discovered package.
    pub(crate) async fn resolve(
        &self,
        spec: &PackageInstallSpec,
        config: &Config,
    ) -> Result<FoundPackage, DiscoverError> {
        if let Some(source) = &spec.source {
            let download = RemoteRockDownload::from_package_req_and_source_spec(
                spec.package.clone(),
                source.clone(),
            )
            .map_err(|err| DiscoverError::Download(spec.package.clone(), Box::new(err)))?;
            let rockspec = download.rockspec().clone();
            let package = RemotePackage::new(
                spec.package.clone().try_into().map_err(|err| {
                    DiscoverError::Download(
                        spec.package.clone(),
                        Box::new(crate::operations::SearchAndDownloadError::from(err)),
                    )
                })?,
                download.rockspec_download().source.clone(),
                download.rockspec_download().source_url.clone(),
            );
            return Ok(FoundPackage {
                remote: package,
                rockspec,
                artifact: None,
            });
        }

        for provider in &self.providers {
            // Providers that can serve a package without downloading it (local packages and the
            // manifest cache) resolve early.
            if let Some(found) = provider.resolve(spec, config).await? {
                return Ok(found);
            }

            // Otherwise break on the first provider that has the package. The `break` prevents a
            // further invocation of `provider.resolve` on a provider we don't want.
            if provider.find(&spec.package, None).await.is_some() {
                break;
            }
        }

        if self.has_remote_sources() {
            let mut package = self.find(&spec.package, None).await?;
            let download = if let Some(vendor_dir) = config.vendor_dir() {
                FetchVendored::new()
                    .vendor_dir(vendor_dir)
                    .package(&spec.package)
                    .package_db(self)
                    .fetch_vendored_rock()
                    .await
                    .map_err(|err| {
                        DiscoverError::FetchVendored(spec.package.clone(), Box::new(err))
                    })?
            } else {
                Download::new(&spec.package, config)
                    .package_db(self)
                    .download_remote_rock()
                    .await
                    .map_err(|err| DiscoverError::Download(spec.package.clone(), Box::new(err)))?
            };
            let rockspec = download.rockspec().clone();
            let artifact = match &download {
                RemoteRockDownload::SrcRock {
                    src_rock,
                    source_url,
                    ..
                } => {
                    package.source_url = Some(source_url.clone());
                    Some(PackageSource::Archive(src_rock.clone()))
                }
                RemoteRockDownload::BinaryRock { packed_rock, .. } => {
                    Some(PackageSource::Archive(packed_rock.clone()))
                }
                RemoteRockDownload::RockspecOnly { .. } => None,
            };
            return Ok(FoundPackage {
                remote: package,
                rockspec,
                artifact,
            });
        }

        Err(DiscoverError::Search(SearchError::RockNotFound(
            spec.package.clone(),
        )))
    }

    /// Search for all packages that match the requirement.
    pub async fn search(
        &self,
        package_req: &PackageReq,
    ) -> Vec<(PackageName, Vec<PackageVersion>)> {
        let mut results = Vec::new();
        for provider in &self.providers {
            results.extend(provider.search(package_req).await);
        }
        results
    }

    /// Find the latest version for a package by name.
    pub(crate) async fn latest_version(&self, rock_name: &PackageName) -> Option<PackageVersion> {
        self.latest_match(&rock_name.clone().into(), None)
            .await
            .map(|result| result.version().clone())
    }

    /// Find the latest package that matches the requirement.
    pub async fn latest_match(
        &self,
        package_req: &PackageReq,
        filter: Option<RemotePackageTypeFilterSpec>,
    ) -> Option<PackageSpec> {
        match self.find(package_req, filter).await {
            Ok(result) => Some(result.spec),
            Err(_) => None,
        }
    }
}

impl From<Manifest> for PackageDB {
    fn from(manifest: Manifest) -> Self {
        Self {
            providers: vec![PackageProvider::Manifest(manifest)],
        }
    }
}

impl From<Vec<PackageLock>> for PackageDB {
    fn from(locks: Vec<PackageLock>) -> Self {
        Self {
            providers: vec![PackageProvider::Lockfile(locks)],
        }
    }
}
