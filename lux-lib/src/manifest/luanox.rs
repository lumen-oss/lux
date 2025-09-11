use itertools::Itertools;
use thiserror::Error;
use url::Url;

use crate::{
    manifest::ManifestMetadata,
    package::{
        PackageReq, PackageSpec, PackageVersion, PackageVersionParseError, RemotePackage,
        RemotePackageTypeFilterSpec,
    },
    remote_package_source::RemotePackageSource,
};

#[derive(Debug, Clone)]
pub struct LuanoxManifest(Url);

#[derive(serde::Deserialize)]
struct LuanoxPackageResponse {
    #[serde(flatten)]
    data: LuanoxPackageData,
}

#[derive(serde::Deserialize)]
struct LuanoxPackageData {
    releases: Vec<LuanoxPackageRelease>,
}

#[derive(serde::Deserialize)]
struct LuanoxPackageRelease {
    version: String,
}

#[derive(Debug, Error)]
#[error(transparent)]
pub enum LuanoxManifestError {
    ReqwestError(#[from] reqwest::Error),
    UrlParseError(#[from] url::ParseError),
    PackageVersionParseError(#[from] PackageVersionParseError),
}

impl ManifestMetadata for LuanoxManifest {
    fn server_url(&self) -> &Url {
        &self.0
    }

    async fn find(
        &self,
        package_req: &PackageReq,
        // TODO(vhyrro): Implement filtering
        _filter: Option<RemotePackageTypeFilterSpec>,
    ) -> Option<RemotePackage> {
        let package: LuanoxPackageResponse =
            reqwest::get(self.0.join(&format!("api/{}", package_req.name())).ok()?)
                .await
                .ok()?
                .json()
                .await
                .ok()?;
        package
            .data
            .releases
            .into_iter()
            .filter_map(|release| release.version.parse::<PackageVersion>().ok())
            .sorted_by(|a, b| b.cmp(a))
            .find(|version| package_req.version_req().matches(version))
            .and_then(|release| {
                Some(RemotePackage {
                    source: RemotePackageSource::LuarocksRockspec(
                        Url::parse(&format!(
                            "{}/download/{}/{}",
                            self.0,
                            package_req.name(),
                            release
                        ))
                        .ok()?,
                    ),
                    package: PackageSpec::new(package_req.name().clone(), release),
                    source_url: None,
                })
            })
    }
}
