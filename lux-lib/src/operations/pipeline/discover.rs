use miette::Diagnostic;
use thiserror::Error;

use crate::{
    lua_rockspec::RemoteLuaRockspec,
    operations,
    package::{PackageReq, RemotePackage},
    remote_package_db::SearchError,
};

use super::download_sources_and_hash::PackageSource;

#[derive(Clone, Debug)]
pub(crate) struct FoundPackage {
    pub(crate) package: RemotePackage,
    pub(crate) rockspec: RemoteLuaRockspec,
    pub(crate) artifact: Option<PackageSource>,
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum DiscoverError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Search(#[from] SearchError),
    #[error("failed to acquire metadata for '{0}'")]
    #[diagnostic(forward(1))]
    Download(PackageReq, Box<operations::SearchAndDownloadError>),
    #[error("failed to fetch vendored package for '{0}'")]
    #[diagnostic(forward(1))]
    FetchVendored(PackageReq, Box<operations::FetchVendoredError>),
}
