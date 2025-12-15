use itertools::Itertools;
use thiserror::Error;
use url::Url;

use crate::{
    manifest::{DownloadedRock, ManifestDownloadError, RemotePackageDB},
    package::{
        PackageName, PackageReq, PackageSpec, PackageVersion, PackageVersionParseError,
        RemotePackage, RemotePackageTypeFilterSpec,
    },
    progress::{Progress, ProgressBar},
    remote_package_source::RemotePackageSource,
};

#[derive(Debug, Clone)]
pub struct LuanoxRemoteDB(Url);

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

impl RemotePackageDB for LuanoxRemoteDB {
    fn url(&self) -> &Url {
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

    fn search(&self, _package_req: &PackageReq) -> Vec<(&PackageName, Vec<&PackageVersion>)> {
        // TODO(vhyrro): Implement search for Luanox
        Vec::new()
    }

    async fn download_rockspec(
        &self,
        package: &PackageSpec,
        progress: &Progress<ProgressBar>,
    ) -> Result<String, ManifestDownloadError> {
        progress.map(|p| p.set_message(format!("📥 Downloading rockspec for {}", package)));
        let url = Url::parse(&format!(
            "{}/download/{}/{}",
            self.0,
            package.name(),
            package.version()
        ))?;
        let bytes = reqwest::Client::new()
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        Ok(String::from_utf8(bytes.to_vec())?)
    }

    async fn download_src_rock(
        &self,
        package: &PackageSpec,
        progress: &Progress<ProgressBar>,
    ) -> Result<DownloadedRock, ManifestDownloadError> {
        progress.map(|p| {
            p.set_message(format!(
                "📥 Downloading {}-{}.src.rock",
                package.name(),
                package.version()
            ))
        });
        // Luanox typically serves rockspecs, not src rocks
        // For now, we'll return an error suggesting this isn't supported
        Err(ManifestDownloadError::PackageNotFound(
            "Luanox does not support src.rock downloads directly".to_string(),
        ))
    }

    async fn download_binary_rock(
        &self,
        package: &PackageSpec,
        progress: &Progress<ProgressBar>,
    ) -> Result<DownloadedRock, ManifestDownloadError> {
        progress.map(|p| {
            p.set_message(format!(
                "📥 Downloading {}-{} binary rock",
                package.name(),
                package.version()
            ))
        });
        // Luanox typically serves rockspecs, not binary rocks
        // For now, we'll return an error suggesting this isn't supported
        Err(ManifestDownloadError::PackageNotFound(
            "Luanox does not support binary rock downloads directly".to_string(),
        ))
    }

    async fn has_update(
        &self,
        package: &PackageSpec,
        constraint: &PackageReq,
    ) -> Option<PackageVersion> {
        let package_response: LuanoxPackageResponse =
            reqwest::get(self.0.join(&format!("api/{}", package.name())).ok()?)
                .await
                .ok()?
                .json()
                .await
                .ok()?;

        package_response
            .data
            .releases
            .into_iter()
            .filter_map(|release| release.version.parse::<PackageVersion>().ok())
            .filter(|version| constraint.version_req().matches(version))
            .filter(|version| version > package.version())
            .max()
    }
}
