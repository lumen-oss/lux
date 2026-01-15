use thiserror::Error;
use url::Url;

pub use crate::manifest::metadata::*;
use crate::manifest::metadata_from_server::*;
use crate::package::{RemotePackageType, RemotePackageTypeFilterSpec};
use crate::progress::{Progress, ProgressBar};
use crate::{
    config::Config,
    package::{PackageReq, RemotePackage},
    remote_package_source::RemotePackageSource,
};

pub mod metadata;
mod metadata_from_server;

#[derive(Error, Debug)]
pub enum ManifestError {
    #[error(transparent)]
    Lua(#[from] ManifestLuaError),
    #[error("failed to fetch manifest from server:\n{0}")]
    Server(#[from] ManifestFromServerError),
}

#[derive(Clone, Debug)]
pub(crate) struct Manifest {
    server_url: Url,
    metadata: ManifestMetadata,
}

impl Manifest {
    pub fn new(server_url: Url, metadata: ManifestMetadata) -> Self {
        Self {
            server_url,
            metadata,
        }
    }

    pub async fn from_config(
        server_url: Url,
        config: &Config,
        progress: &Progress<ProgressBar>,
    ) -> Result<Self, ManifestError> {
        let content = manifest_from_cache_or_server(&server_url, config, progress).await?;
        match ManifestMetadata::new(&content) {
            Ok(metadata) => Ok(Self::new(server_url, metadata)),
            Err(_) => {
                let manifest = manifest_from_server_only(&server_url, config, progress).await?;
                Ok(Self::new(server_url, ManifestMetadata::new(&manifest)?))
            }
        }
    }

    pub fn server_url(&self) -> &Url {
        &self.server_url
    }

    pub fn metadata(&self) -> &ManifestMetadata {
        &self.metadata
    }

    /// Find a package that matches the requirement, returning the latest match
    pub fn find(
        &self,
        package_req: &PackageReq,
        filter: Option<RemotePackageTypeFilterSpec>,
    ) -> Option<RemotePackage> {
        match self.metadata().latest_match(package_req, filter) {
            None => None,
            Some((package, package_type)) => {
                let remote_source = match package_type {
                    RemotePackageType::Rockspec => {
                        RemotePackageSource::LuarocksRockspec(self.server_url().clone())
                    }
                    RemotePackageType::Src => {
                        RemotePackageSource::LuarocksSrcRock(self.server_url().clone())
                    }
                    RemotePackageType::Binary => {
                        RemotePackageSource::LuarocksBinaryRock(self.server_url().clone())
                    }
                };
                Some(RemotePackage::new(package, remote_source, None))
            }
        }
    }
}
