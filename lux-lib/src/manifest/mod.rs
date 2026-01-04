use path_slash::PathExt;
use thiserror::Error;
use url::Url;

pub use crate::manifest::metadata::*;
use crate::manifest::metadata_from_server::*;
use crate::manifest::metadata_from_vendor_dir::manifest_from_vendor_dir;
use crate::package::{RemotePackageType, RemotePackageTypeFilterSpec};
use crate::progress::{Progress, ProgressBar};
use crate::{
    config::Config,
    package::{PackageReq, RemotePackage},
    remote_package_source::RemotePackageSource,
};

pub mod metadata;
mod metadata_from_server;
mod metadata_from_vendor_dir;

#[derive(Error, Debug)]
pub enum ManifestError {
    #[error(transparent)]
    Lua(#[from] ManifestLuaError),
    #[error("failed to fetch manifest from server:\n{0}")]
    Server(#[from] ManifestFromServerError),
    #[error("error parsing URL from `vendor-dir`: {0}:")]
    Vendor(String),
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
        if let Some(vendor_dir) = config.vendor_dir() {
            let server_url: Url = Url::from_file_path(vendor_dir)
                .map_err(|_err| ManifestError::Vendor(vendor_dir.to_slash_lossy().to_string()))?;
            return Ok(Self::new(server_url, manifest_from_vendor_dir(vendor_dir)));
        }
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
