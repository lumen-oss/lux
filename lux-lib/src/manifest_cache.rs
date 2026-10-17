//! Client for Lux's manifest cache.
//!
//! The cache precomputes the full transitive dependency closure including evaluated rockspecs.
//! This lets Lux resolve a dependency graph without having to recursively fetch new rockspecs for
//! all dependencies.
//! Cache misses simply fall back to the regular discovery/building logic.

use serde::{Deserialize, Deserializer};
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;

use crate::{
    config::Config,
    lua_version::LuaVersion,
    luarocks,
    package::{
        PackageName, PackageReq, PackageVersion, RemotePackageType, RemotePackageTypeFilterSpec,
    },
};

// cache.luanox.org may change its format over time, so we track it.
const CACHE_FORMAT: u32 = 1;

#[derive(Debug, Error)]
pub(crate) enum ManifestCacheError {
    #[error("rock not found in the manifest cache")]
    NotFound,
    #[error("unsupported manifest cache format {found} (expected {expected})")]
    UnsupportedFormat { found: u32, expected: u32 },
    #[error("failed to request the manifest cache")]
    Request(#[source] reqwest::Error),
    #[error("failed to parse the manifest cache response")]
    Parse(#[source] serde_json::Error),
    #[error("invalid manifest cache URL")]
    Url(#[from] url::ParseError),
}

/// A precomputed dependency closure for one rock version.
#[derive(Debug, Clone, Deserialize)]
pub struct Closure {
    format: u32,
    specs: Vec<ClosureSpec>,
}

impl Closure {
    pub(crate) fn spec(
        &self,
        req: &PackageReq,
        filter: Option<&RemotePackageTypeFilterSpec>,
    ) -> Option<(&ClosureSpec, RemotePackageType)> {
        let default_filter = RemotePackageTypeFilterSpec::default();
        let filter = filter.unwrap_or(&default_filter);
        self.specs
            .iter()
            .filter(|spec| {
                spec.package() == req.name() && req.version_req().matches(spec.version())
            })
            .filter_map(|spec| {
                spec.remote_package_type(filter)
                    .map(|package_type| (spec, package_type))
            })
            .max_by(|(spec_a, type_a), (spec_b, type_b)| {
                spec_a
                    .version()
                    .cmp(spec_b.version())
                    .then_with(|| type_a.cmp(type_b))
            })
    }
}

/// One entry of a precomputed closure.
#[derive(Debug, Clone, Deserialize)]
pub struct ClosureSpec {
    package: PackageName,
    version: PackageVersion,
    #[serde(deserialize_with = "deserialize_available_types")]
    available_types: Vec<RemotePackageType>,
    rockspec_raw: String,
}

impl ClosureSpec {
    pub(crate) fn package(&self) -> &PackageName {
        &self.package
    }

    pub(crate) fn version(&self) -> &PackageVersion {
        &self.version
    }

    pub(crate) fn rockspec_raw(&self) -> &str {
        &self.rockspec_raw
    }

    /// The highest-priority package type available for this rock that is allowed
    /// by `filter`.
    pub(crate) fn remote_package_type(
        &self,
        filter: &RemotePackageTypeFilterSpec,
    ) -> Option<RemotePackageType> {
        self.available_types
            .iter()
            .filter(|&package_type| match package_type {
                RemotePackageType::Rockspec => filter.rockspec,
                RemotePackageType::Src => filter.src,
                RemotePackageType::Binary => filter.binary,
            })
            .cloned()
            .max()
    }
}

#[derive(Debug, Clone, Deserialize)]
struct VersionsIndex {
    format: u32,
    versions: Vec<PackageVersion>,
}

#[derive(Debug, Clone)]
pub struct ManifestCache {
    base_url: Url,
    // Passed in the URL to fetch the right manifest version
    lua_version: String,
    client: reqwest::Client,
}

impl ManifestCache {
    pub(crate) fn from_config(config: &Config) -> Option<Self> {
        let base_url = config.manifest_cache_url()?.clone();
        let lua_version = LuaVersion::from(config).ok()?.version_compatibility_str();
        #[cfg(not(test))]
        let client = crate::reqwest::https_client(config);
        #[cfg(test)]
        let client = crate::reqwest::http_client(config);
        Some(Self {
            base_url,
            lua_version,
            client: client.ok()?.clone(),
        })
    }

    pub(crate) async fn versions(
        &self,
        name: &PackageName,
    ) -> Result<Vec<PackageVersion>, ManifestCacheError> {
        let url = self.url(&format!("versions/{name}.json"))?;
        let index: VersionsIndex = self.get_json(&url).await?;
        self.check_format(index.format)?;
        Ok(index.versions)
    }

    pub(crate) async fn latest_closure(
        &self,
        name: &PackageName,
    ) -> Result<Closure, ManifestCacheError> {
        let url = self.url(&format!("latest/{name}.json"))?;
        let closure: Closure = self.get_json(&url).await?;
        self.check_format(closure.format)?;
        Ok(closure)
    }

    pub(crate) async fn closure(
        &self,
        name: &PackageName,
        version: &PackageVersion,
    ) -> Result<Closure, ManifestCacheError> {
        let hash = hex::encode(Sha256::digest(format!("{name}@{version}").as_bytes()));
        let url = self.url(&format!("{hash}.json"))?;
        let closure: Closure = self.get_json(&url).await?;
        self.check_format(closure.format)?;
        Ok(closure)
    }

    fn check_format(&self, format: u32) -> Result<(), ManifestCacheError> {
        if format == CACHE_FORMAT {
            Ok(())
        } else {
            Err(ManifestCacheError::UnsupportedFormat {
                found: format,
                expected: CACHE_FORMAT,
            })
        }
    }

    fn url(&self, path: &str) -> Result<Url, ManifestCacheError> {
        Ok(self
            .base_url
            .join(&format!("{}/{}", self.lua_version, path))?)
    }

    async fn get_json<T>(&self, url: &Url) -> Result<T, ManifestCacheError>
    where
        T: serde::de::DeserializeOwned,
    {
        let response = self
            .client
            .get(url.clone())
            .send()
            .await
            .map_err(ManifestCacheError::Request)?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(ManifestCacheError::NotFound);
        }

        let response = response
            .error_for_status()
            .map_err(ManifestCacheError::Request)?;

        let text = response.text().await.map_err(ManifestCacheError::Request)?;
        serde_json::from_str(&text).map_err(ManifestCacheError::Parse)
    }
}

fn deserialize_available_types<'de, D>(deserializer: D) -> Result<Vec<RemotePackageType>, D::Error>
where
    D: Deserializer<'de>,
{
    use RemotePackageType::{Binary, Rockspec, Src};

    let arches = Vec::<String>::deserialize(deserializer)?;

    Ok(arches
        .into_iter()
        .filter_map(|arch| match arch.as_str() {
            "rockspec" => Some(Rockspec),
            "src" => Some(Src),
            "all" => Some(Binary),
            arch if arch == luarocks::current_platform_luarocks_identifier().as_str() => {
                Some(Binary)
            }
            _ => None,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn closure_from_json(json: &str) -> Closure {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn selects_latest_matching_version() {
        let closure = closure_from_json(
            r#"{
                "format": 1,
                "specs": [
                    {"package":"say","version":"1.0.0-1","available_types":["rockspec"],"rockspec_raw":""},
                    {"package":"say","version":"2.0.0-1","available_types":["rockspec"],"rockspec_raw":""}
                ]
            }"#,
        );
        let req = PackageReq::from_str("say").unwrap();
        let (spec, _) = closure.spec(&req, None).unwrap();
        assert_eq!(
            spec.version(),
            &PackageVersion::from_str("2.0.0-1").unwrap()
        );
    }

    #[test]
    fn prefers_binary_over_rockspec_over_src() {
        let req = PackageReq::from_str("say").unwrap();

        let rockspec_or_src = closure_from_json(
            r#"{"format":1,"specs":[{"package":"say","version":"1.0.0-1","available_types":["rockspec","src"],"rockspec_raw":""}]}"#,
        );
        assert_eq!(
            rockspec_or_src.spec(&req, None).unwrap().1,
            RemotePackageType::Rockspec
        );

        let binary = closure_from_json(
            r#"{"format":1,"specs":[{"package":"say","version":"1.0.0-1","available_types":["all","rockspec"],"rockspec_raw":""}]}"#,
        );
        assert_eq!(
            binary.spec(&req, None).unwrap().1,
            RemotePackageType::Binary
        );
    }

    #[test]
    fn respects_type_filter() {
        let closure = closure_from_json(
            r#"{"format":1,"specs":[{"package":"say","version":"1.0.0-1","available_types":["all","rockspec"],"rockspec_raw":""}]}"#,
        );
        let req = PackageReq::from_str("say").unwrap();
        let filter: RemotePackageTypeFilterSpec =
            serde_json::from_str(r#"{"rockspec":true,"src":false,"binary":false}"#).unwrap();
        assert_eq!(
            closure.spec(&req, Some(&filter)).unwrap().1,
            RemotePackageType::Rockspec
        );
    }

    #[test]
    fn version_constraint_filters_specs() {
        let closure = closure_from_json(
            r#"{"format":1,"specs":[{"package":"say","version":"1.0.0-1","available_types":["rockspec"],"rockspec_raw":""},{"package":"say","version":"2.0.0-1","available_types":["rockspec"],"rockspec_raw":""}]}"#,
        );
        let req = PackageReq::from_str("say < 2.0.0").unwrap();
        let (spec, _) = closure.spec(&req, None).unwrap();
        assert_eq!(
            spec.version(),
            &PackageVersion::from_str("1.0.0-1").unwrap()
        );
    }
}
