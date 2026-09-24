use std::sync::Arc;

use bon::Builder;
use miette::Diagnostic;
use thiserror::Error;

use crate::{
    config::Config,
    lua_rockspec::RemoteLuaRockspec,
    operations::{self, Download, FetchVendored, FetchVendoredError, RemoteRockDownload},
    package::{PackageReq, RemotePackage},
    remote_package_db::{RemotePackageDB, SearchError},
};

use super::download_sources_and_hash::PackageSource;

pub(crate) struct FoundPackage {
    pub(crate) package: RemotePackage,
    pub(crate) rockspec: RemoteLuaRockspec,
    pub(crate) artifact: Option<PackageSource>,
}

pub(crate) enum FoundPackageType {
    Package(FoundPackage),
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub(crate) enum DiscoverError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Search(#[from] SearchError),
    #[error("failed to acquire metadata for '{0}'")]
    #[diagnostic(forward(1))]
    Download(PackageReq, Box<operations::SearchAndDownloadError>),
    #[error("failed to fetch vendored package for '{0}'")]
    #[diagnostic(forward(1))]
    FetchVendored(PackageReq, Box<FetchVendoredError>),
}

pub(crate) trait FindPackageFromProvider: Clone + Send + Sync + 'static {
    fn find(
        &self,
        req: &PackageReq,
    ) -> impl std::future::Future<Output = Result<FoundPackageType, DiscoverError>> + Send;
}

#[derive(Builder, Clone)]
#[builder(start_fn = new)]
pub(crate) struct FindPackageFromLuarocks {
    #[builder(start_fn)]
    pub(crate) package_db: Arc<RemotePackageDB>,
    #[builder(start_fn)]
    pub(crate) config: Arc<Config>,
}

impl FindPackageFromProvider for FindPackageFromLuarocks {
    async fn find(&self, req: &PackageReq) -> Result<FoundPackageType, DiscoverError> {
        let mut package = self.package_db.find(req, None)?;
        // TODO(vhyrro): Don't force us to call download here, extrapolate stuff from
        // `operations` into this pipeline instead.
        let download = if let Some(vendor_dir) = self.config.vendor_dir() {
            FetchVendored::new()
                .vendor_dir(vendor_dir)
                .package(req)
                .package_db(&self.package_db)
                .fetch_vendored_rock()
                .await
                .map_err(|err| DiscoverError::FetchVendored(req.clone(), Box::new(err)))?
        } else {
            Download::new(req, &self.config)
                .package_db(&self.package_db)
                .download_remote_rock()
                .await
                .map_err(|err| DiscoverError::Download(req.clone(), Box::new(err)))?
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

        Ok(FoundPackageType::Package(FoundPackage {
            package,
            rockspec,
            artifact,
        }))
    }
}
