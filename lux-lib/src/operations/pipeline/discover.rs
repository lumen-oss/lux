use miette::Diagnostic;
use thiserror::Error;

use crate::{
    lua_rockspec::RemoteLuaRockspec,
    operations,
    package::{PackageReq, PackageSpec, RemotePackage},
    remote_package_db::SearchError,
    remote_package_source::RemotePackageSource,
    rockspec::Rockspec,
};

use super::download_sources_and_hash::PackageSource;

#[derive(Clone, Debug)]
pub(crate) struct FoundPackage {
    pub(crate) package: RemotePackage,
    pub(crate) rockspec: RemoteLuaRockspec,
    pub(crate) artifact: Option<PackageSource>,
}

impl FoundPackage {
    /// Builds a discovered package from an already-parsed rockspec, so it can
    /// be resolved as a root without contacting a remote package database.
    pub(crate) fn from_rockspec(rockspec: RemoteLuaRockspec) -> Self {
        let source = rockspec
            .to_lua_remote_rockspec_string()
            .map(RemotePackageSource::RockspecContent)
            .unwrap_or(RemotePackageSource::Local);
        let package = RemotePackage::new(
            PackageSpec::new(rockspec.package().clone(), rockspec.version().clone()),
            source,
            None,
        );
        Self {
            package,
            rockspec,
            artifact: None,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_rockspec_builds_local_found_package() {
        let content = "rockspec_format = '1.0'\npackage = 'foo'\nversion = '1.0.0-1'\nsource = { url = 'https://example.com/foo.zip' }";
        let Ok(rockspec) = RemoteLuaRockspec::new(content) else {
            panic!("invalid test rockspec");
        };

        let found = FoundPackage::from_rockspec(rockspec);

        assert_eq!(found.package.package.name().to_string(), "foo");
        assert_eq!(found.package.package.version().to_string(), "1.0.0-1");
        assert!(matches!(
            found.package.source,
            RemotePackageSource::RockspecContent(_)
        ));
        assert!(found.artifact.is_none());
    }
}
