use crate::lua_rockspec::LuaRockspecError;
use crate::operations::{DownloadSrcRockError, DownloadedPackedRockBytes, RemoteRockDownload};
use crate::package::{
    PackageName, PackageReq, PackageVersion, RemotePackage, RemotePackageTypeFilterSpec,
};
use crate::progress::{Progress, ProgressBar};
use bytes::Bytes;
use enum_dispatch::enum_dispatch;
use std::string::FromUtf8Error;
use thiserror::Error;
use url::Url;

pub mod luanox;
pub mod luarocks;

#[derive(Error, Debug)]
pub enum ManifestDownloadError {
    #[error("failed to download: {0}")]
    Request(#[from] reqwest::Error),
    #[error("failed to parse URL: {0}")]
    UrlParse(#[from] url::ParseError),
    #[error("package not found: {0}")]
    PackageNotFound(String),
    #[error("invalid UTF-8 in response: {0}")]
    Utf8(#[from] FromUtf8Error),
    #[error(transparent)]
    LuaRockspec(#[from] LuaRockspecError),
    #[error(transparent)]
    DownloadSrcRock(#[from] DownloadSrcRockError),
}

#[derive(Debug, Clone)]
pub struct DownloadedRock {
    pub name: PackageName,
    pub version: PackageVersion,
    pub bytes: Bytes,
    pub url: Url,
}

#[enum_dispatch]
pub(crate) trait RemotePackageDB {
    /// Find a package that matches the requirement, returning the latest match
    async fn find(
        &self,
        package_req: &PackageReq,
        filter: Option<RemotePackageTypeFilterSpec>,
    ) -> Option<RemotePackage>;

    fn url(&self) -> &Url;

    /// Search for all packages that match the requirement
    fn search(&self, package_req: &PackageReq) -> Vec<(&PackageName, Vec<&PackageVersion>)>;

    /// Download a rockspec for the given package
    async fn download_rockspec(
        &self,
        package: RemotePackage,
        progress: &Progress<ProgressBar>,
    ) -> Result<RemoteRockDownload, ManifestDownloadError>;

    /// Download a source rock for the given package
    async fn download_src_rock(
        &self,
        package: RemotePackage,
        progress: &Progress<ProgressBar>,
    ) -> Result<DownloadedPackedRockBytes, ManifestDownloadError>;

    /// Download a binary rock for the given package
    async fn download_binary_rock(
        &self,
        package: RemotePackage,
        progress: &Progress<ProgressBar>,
    ) -> Result<DownloadedPackedRockBytes, ManifestDownloadError>;
}

use luanox::LuanoxRemoteDB;
use luarocks::LuarocksManifest;

#[enum_dispatch(RemotePackageDB)]
#[derive(Debug, Clone)]
pub(crate) enum RemotePackageDBImpl {
    LuarocksManifest,
    LuanoxRemoteDB,
}
