use crate::package::{PackageReq, RemotePackage, RemotePackageTypeFilterSpec};
use enum_dispatch::enum_dispatch;
use url::Url;

pub mod luanox;
pub mod luarocks;

#[enum_dispatch]
pub(crate) trait ManifestMetadata {
    /// Find a package that matches the requirement, returning the latest match
    fn find(
        &self,
        package_req: &PackageReq,
        filter: Option<RemotePackageTypeFilterSpec>,
    ) -> Option<RemotePackage>;

    fn server_url(&self) -> &Url;
}

use luarocks::LuarocksManifest;
// use luanox::LuanoxManifest;

#[enum_dispatch(ManifestMetadata)]
#[derive(Debug, Clone)]
pub(crate) enum Manifest {
    LuarocksManifest,
    // LuanoxManifest,
}
