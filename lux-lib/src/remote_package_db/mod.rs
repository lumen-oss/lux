use std::collections::HashMap;

use crate::{
    config::{Config, ConfigError},
    lockfile::{LocalPackageLock, LockfileIntegrityError},
    manifest::{
        luarocks::{LuarocksManifest, ManifestError},
        Manifest, ManifestMetadata,
    },
    package::{
        PackageName, PackageReq, PackageSpec, PackageVersion, RemotePackage,
        RemotePackageTypeFilterSpec,
    },
    progress::{Progress, ProgressBar},
};
use futures::stream::{self, StreamExt};
use itertools::Itertools;
use mlua::{FromLua, UserData};
use thiserror::Error;

#[derive(Clone, FromLua)]
pub struct RemotePackageDB(Impl);

#[derive(Clone)]
enum Impl {
    Manifests(Vec<Manifest>),
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
        // NOTE: We assume that all custom servers provided via `extra_servers` are Luarocks
        // servers. We do not assume that any other server is Luanox-compatible.
        // If we ever support other server types, we will need to add a way to specify
        // the server type in the config.
        let mut manifests = Vec::new();
        for server in config.enabled_dev_servers()? {
            let manifest = LuarocksManifest::from_config(server, config, progress).await?;
            manifests.push(Manifest::from(manifest));
        }
        for server in config.extra_servers() {
            let manifest = LuarocksManifest::from_config(server.clone(), config, progress).await?;
            manifests.push(Manifest::from(manifest));
        }
        manifests.push(Manifest::from(
            LuarocksManifest::from_config(config.server().clone(), config, progress).await?,
        ));
        Ok(Self(Impl::Manifests(manifests)))
    }

    /// Find a remote package that matches the requirement, returning the latest match.
    pub(crate) async fn find(
        &self,
        package_req: &PackageReq,
        filter: Option<RemotePackageTypeFilterSpec>,
        progress: &Progress<ProgressBar>,
    ) -> Result<RemotePackage, SearchError> {
        match &self.0 {
            Impl::Manifests(manifests) => {
                let search = stream::iter(manifests).filter_map(async |manifest| {
                    progress
                        .map(|p| p.set_message(format!("🔎 Searching {}", &manifest.server_url())));
                    manifest.find(package_req, filter.clone()).await
                });

                tokio::pin!(search);

                match search.next().await {
                    Some(package) => Ok(package),
                    None => Err(SearchError::RockNotFound(package_req.clone())),
                }
            }
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
            Impl::Manifests(manifests) => manifests
                .iter()
                .flat_map(|manifest| match manifest {
                    Manifest::LuarocksManifest(m) => {
                        m.metadata()
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
                    }
                    Manifest::LuanoxManifest(_m) => todo!(),
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
        match self
            .find(package_req, filter, &Progress::no_progress())
            .await
        {
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
        methods.add_async_method("latest_match", |_, this, package_req| async move {
            Ok(this.latest_match(&package_req, None).await)
        });
    }
}

impl From<LuarocksManifest> for RemotePackageDB {
    fn from(manifest: LuarocksManifest) -> Self {
        Self(Impl::Manifests(vec![Manifest::LuarocksManifest(manifest)]))
    }
}

impl From<LocalPackageLock> for RemotePackageDB {
    fn from(lock: LocalPackageLock) -> Self {
        Self(Impl::Lock(lock))
    }
}
