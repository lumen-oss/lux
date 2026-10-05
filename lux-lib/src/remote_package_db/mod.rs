use crate::{
    config::{Config, ConfigError},
    lockfile::{LockfileIntegrityError, PackageLock},
    manifest::{Manifest, ManifestError},
    operations::{Download, FetchVendored, PackageInstallSpec, RemoteRockDownload},
    package::{
        PackageName, PackageReq, PackageSpec, PackageVersion, RemotePackage,
        RemotePackageTypeFilterSpec,
    },
    pipeline::{
        discover::{DiscoverError, FoundPackage},
        download_sources_and_hash::PackageSource,
    },
};
use itertools::Itertools;

use miette::Diagnostic;
use thiserror::Error;

/// Package database, used to look up remote rocks
#[derive(Clone, Debug)]
pub struct RemotePackageDB(pub(crate) Vec<RemoteSource>);

#[derive(Clone, Debug)]
pub(crate) enum RemoteSource {
    LuarocksManifests(Vec<Manifest>),
    LockedPackageLocks(Vec<PackageLock>),
    Local(Vec<FoundPackage>),
}

#[derive(Error, Debug, Diagnostic)]
pub enum RemotePackageDBError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    ManifestError(#[from] ManifestError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    ConfigError(#[from] ConfigError),
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum SearchError {
    #[error("no rock that matches '{0}' found")]
    RockNotFound(PackageReq),
    #[error("no rock that matches '{0}' found in the lockfile.")]
    RockNotFoundInLockfile(PackageReq),
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

impl RemotePackageDB {
    #[tracing::instrument(level = "trace", skip_all)]
    pub async fn from_config(config: &Config) -> Result<Self, RemotePackageDBError> {
        let mut manifests = Vec::new();
        for server in config.enabled_dev_servers()? {
            let manifest = Manifest::from_config(server, config).await?;
            manifests.push(manifest);
        }
        for server in config.extra_servers() {
            let manifest = Manifest::from_config(server.clone(), config).await?;
            manifests.push(manifest);
        }
        manifests.push(Manifest::from_config(config.server().clone(), config).await?);
        Ok(Self(vec![RemoteSource::LuarocksManifests(manifests)]))
    }

    /// Prepends local packages to the lookup order.
    pub(crate) fn with_local(mut self, packages: Vec<FoundPackage>) -> Self {
        self.0.insert(0, RemoteSource::Local(packages));
        self
    }

    /// Prepends locked packages to the lookup order, ahead of manifests.
    pub(crate) fn with_locks(mut self, locks: Vec<PackageLock>) -> Self {
        self.0.insert(0, RemoteSource::LockedPackageLocks(locks));
        self
    }

    fn find_in_source(
        source: &RemoteSource,
        package_req: &PackageReq,
        filter: Option<RemotePackageTypeFilterSpec>,
    ) -> Result<Option<RemotePackage>, SearchError> {
        match source {
            RemoteSource::LuarocksManifests(manifests) => Ok(manifests
                .iter()
                .find_map(|manifest| manifest.find(package_req, filter.clone()))),
            RemoteSource::LockedPackageLocks(locks) => Ok(locks
                .iter()
                .filter_map(|lock| lock.has_rock(package_req, filter.clone()))
                .map(|local_package| {
                    RemotePackage::new(
                        local_package.to_package(),
                        local_package.source().clone(),
                        local_package.source_url().cloned(),
                    )
                })
                .next()),
            RemoteSource::Local(packages) => Ok(packages
                .iter()
                .find(|package| package.package.package.name() == package_req.name())
                .map(|package| package.package.clone())),
        }
    }

    /// Find a remote package that matches the requirement, returning the latest match.
    #[tracing::instrument(
          name = "Searching tree-sitter parser",
          level = "info",
          skip_all,
          fields(package = package_req.to_string()),
    )]
    pub(crate) fn find(
        &self,
        package_req: &PackageReq,
        filter: Option<RemotePackageTypeFilterSpec>,
    ) -> Result<RemotePackage, SearchError> {
        for source in &self.0 {
            if let Some(package) = Self::find_in_source(source, package_req, filter.clone())? {
                return Ok(package);
            }
        }
        if self
            .0
            .iter()
            .any(|source| matches!(source, RemoteSource::LuarocksManifests(_)))
        {
            Err(SearchError::RockNotFound(package_req.clone()))
        } else if self
            .0
            .iter()
            .any(|source| matches!(source, RemoteSource::LockedPackageLocks(_)))
        {
            Err(SearchError::RockNotFoundInLockfile(package_req.clone()))
        } else {
            Err(SearchError::RockNotFound(package_req.clone()))
        }
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
                package,
                rockspec,
                artifact: None,
            });
        }

        for source in &self.0 {
            match source {
                RemoteSource::Local(packages) => {
                    if let Some(package) = packages
                        .iter()
                        .find(|package| package.package.package.name() == spec.package.name())
                    {
                        return Ok(package.clone());
                    }
                }
                RemoteSource::LuarocksManifests(_) | RemoteSource::LockedPackageLocks(_) => {
                    let mut package = self.find(&spec.package, None)?;
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
                            .map_err(|err| {
                                DiscoverError::Download(spec.package.clone(), Box::new(err))
                            })?
                    };
                    let rockspec = download.rockspec().clone();
                    let artifact = match &download {
                        RemoteRockDownload::SrcRock {
                            src_rock,
                            source_url,
                            ..
                        } => {
                            package.source_url = Some(source_url.clone());
                            Some(PackageSource::PackedRock(src_rock.clone()))
                        }
                        RemoteRockDownload::BinaryRock { packed_rock, .. } => {
                            Some(PackageSource::PackedRock(packed_rock.clone()))
                        }
                        RemoteRockDownload::RockspecOnly { .. } => None,
                    };
                    return Ok(FoundPackage {
                        package,
                        rockspec,
                        artifact,
                    });
                }
            }
        }

        Err(DiscoverError::Search(SearchError::RockNotFound(
            spec.package.clone(),
        )))
    }

    /// Search for all packages that match the requirement.
    pub fn search(&self, package_req: &PackageReq) -> Vec<(&PackageName, Vec<&PackageVersion>)> {
        self.0
            .iter()
            .flat_map(|source| match source {
                RemoteSource::LuarocksManifests(manifests) => manifests
                    .iter()
                    .flat_map(|manifest| {
                        manifest
                            .metadata()
                            .repository
                            .iter()
                            .filter_map(|(name, elements)| {
                                if name.to_string().contains(&package_req.name().to_string()) {
                                    Some((
                                        name,
                                        elements
                                            .keys()
                                            .filter(|version| {
                                                package_req.version_req().matches(version)
                                            })
                                            .sorted_by(|a, b| Ord::cmp(b, a))
                                            .collect_vec(),
                                    ))
                                } else {
                                    None
                                }
                            })
                    })
                    .collect_vec(),
                RemoteSource::LockedPackageLocks(locks) => locks
                    .iter()
                    .flat_map(|lock| lock.rocks().values())
                    .filter_map(|package| {
                        // NOTE: This doesn't group packages by name, but we don't care for now,
                        // as we shouldn't need to use this function with a lockfile.
                        let name = package.name();
                        if name.to_string().contains(&package_req.name().to_string()) {
                            Some((name, vec![package.version()]))
                        } else {
                            None
                        }
                    })
                    .collect_vec(),
                RemoteSource::Local(packages) => packages
                    .iter()
                    .filter_map(|package| {
                        let name = package.package.package.name();
                        if name.to_string().contains(&package_req.name().to_string())
                            && package_req
                                .version_req()
                                .matches(package.package.package.version())
                        {
                            Some((name, vec![package.package.package.version()]))
                        } else {
                            None
                        }
                    })
                    .collect_vec(),
            })
            .collect()
    }

    /// Find the latest version for a package by name.
    pub(crate) fn latest_version(&self, rock_name: &PackageName) -> Option<PackageVersion> {
        self.latest_match(&rock_name.clone().into(), None)
            .map(|result| result.version().clone())
    }

    /// Find the latest package that matches the requirement.
    pub fn latest_match(
        &self,
        package_req: &PackageReq,
        filter: Option<RemotePackageTypeFilterSpec>,
    ) -> Option<PackageSpec> {
        match self.find(package_req, filter) {
            Ok(result) => Some(result.package),
            Err(_) => None,
        }
    }
}

impl From<Manifest> for RemotePackageDB {
    fn from(manifest: Manifest) -> Self {
        Self(vec![RemoteSource::LuarocksManifests(vec![manifest])])
    }
}

impl From<Vec<PackageLock>> for RemotePackageDB {
    fn from(locks: Vec<PackageLock>) -> Self {
        Self(vec![RemoteSource::LockedPackageLocks(locks)])
    }
}
