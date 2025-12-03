use std::collections::HashMap;

use crate::{
    config::{Config, ConfigError},
    lockfile::{LocalPackageLock, LockfileIntegrityError},
    manifest::{Manifest, ManifestError},
    package::{
        PackageName, PackageReq, PackageSpec, PackageVersion, RemotePackage,
        RemotePackageTypeFilterSpec,
    },
    progress::{Progress, ProgressBar},
};
use itertools::Itertools;
use mlua::{FromLua, UserData};
use thiserror::Error;

#[derive(Clone, FromLua, Debug)]
pub struct RemotePackageDB(Impl);

#[derive(Clone, Debug)]
enum Impl {
    LuarocksManifests(Vec<Manifest>),
    Lock(LocalPackageLock),
}

#[derive(Error, Debug)]
pub enum RemotePackageDBError {
    #[error(transparent)]
    ManifestError(#[from] ManifestError),
    #[error(transparent)]
    ConfigError(#[from] ConfigError),
}

#[derive(Error, Debug)]
pub enum SearchError {
    #[error("no rock that matches '{0}' found")]
    RockNotFound(PackageReq),
    #[error("no rock that matches '{0}' found in the lockfile.")]
    RockNotFoundInLockfile(PackageReq),
    #[error("error when pulling manifest:\n{0}")]
    Manifest(#[from] ManifestError),
}

#[derive(Error, Debug)]
pub enum RemotePackageDbIntegrityError {
    #[error(transparent)]
    Lockfile(#[from] LockfileIntegrityError),
}

impl RemotePackageDB {
    pub async fn from_config(
        config: &Config,
        progress: &Progress<ProgressBar>,
    ) -> Result<Self, RemotePackageDBError> {
        let mut manifests = Vec::new();
        for server in config.enabled_dev_servers()? {
            let manifest = Manifest::from_config(server, config, progress).await?;
            manifests.push(manifest);
        }
        for server in config.extra_servers() {
            let manifest = Manifest::from_config(server.clone(), config, progress).await?;
            manifests.push(manifest);
        }
        manifests.push(Manifest::from_config(config.server().clone(), config, progress).await?);
        Ok(Self(Impl::LuarocksManifests(manifests)))
    }

    /// Find a remote package that matches the requirement, returning the latest match.
    pub(crate) fn find(
        &self,
        package_req: &PackageReq,
        filter: Option<RemotePackageTypeFilterSpec>,
        progress: &Progress<ProgressBar>,
    ) -> Result<RemotePackage, SearchError> {
        match &self.0 {
            Impl::LuarocksManifests(manifests) => match manifests.iter().find_map(|manifest| {
                progress.map(|p| p.set_message(format!("🔎 Searching {}", &manifest.server_url())));
                manifest.find(package_req, filter.clone())
            }) {
                Some(package) => Ok(package),
                None => Err(SearchError::RockNotFound(package_req.clone())),
            },
            Impl::Lock(lockfile) => {
                match lockfile.has_rock(package_req, filter).map(|local_package| {
                    RemotePackage::new(
                        PackageSpec::new(local_package.spec.name, local_package.spec.version),
                        local_package.source,
                        local_package.source_url,
                    )
                }) {
                    Some(package) => Ok(package),
                    None => Err(SearchError::RockNotFoundInLockfile(package_req.clone())),
                }
            }
        }
    }

    /// Search for all packages that match the requirement.
    pub fn search(&self, package_req: &PackageReq) -> Vec<(&PackageName, Vec<&PackageVersion>)> {
        match &self.0 {
            Impl::LuarocksManifests(manifests) => manifests
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
                .collect(),
            Impl::Lock(lockfile) => lockfile
                .rocks()
                .iter()
                .filter_map(|(_, package)| {
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
        }
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
        match self.find(package_req, filter, &Progress::no_progress()) {
            Ok(result) => Some(result.package),
            Err(_) => None,
        }
    }
}

impl UserData for RemotePackageDB {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("search", |_, this, package_req: PackageReq| {
            Ok(this
                .search(&package_req)
                .into_iter()
                .map(|(package_name, versions)| {
                    (
                        package_name.clone(),
                        versions.into_iter().cloned().collect_vec(),
                    )
                })
                .collect::<HashMap<_, _>>())
        });
        methods.add_method("latest_match", |_, this, package_req| {
            Ok(this.latest_match(&package_req, None))
        });
    }
}

impl From<Manifest> for RemotePackageDB {
    fn from(manifest: Manifest) -> Self {
        Self(Impl::LuarocksManifests(vec![manifest]))
    }
}

impl From<LocalPackageLock> for RemotePackageDB {
    fn from(lock: LocalPackageLock) -> Self {
        Self(Impl::Lock(lock))
    }
}
