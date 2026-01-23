use itertools::Itertools;
use mlua::{Lua, LuaSerdeExt};
use std::{cmp::Ordering, collections::HashMap};
use thiserror::Error;

use crate::package::{PackageName, PackageReq, PackageSpec, PackageVersion};
use crate::package::{RemotePackageType, RemotePackageTypeFilterSpec};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ManifestMetadata {
    pub repository: HashMap<PackageName, HashMap<PackageVersion, Vec<RemotePackageType>>>,
}

impl<'de> serde::Deserialize<'de> for ManifestMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let intermediate = IntermediateManifest::deserialize(deserializer)?;
        Ok(Self::from_intermediate(intermediate))
    }
}

#[derive(Error, Debug)]
#[error("failed to parse Lua manifest:\n{0}")]
pub struct ManifestLuaError(#[from] mlua::Error);

impl ManifestMetadata {
    pub fn new(manifest: &String) -> Result<Self, ManifestLuaError> {
        let lua = Lua::new();

        #[cfg(feature = "luau")]
        lua.sandbox(true)?;

        lua.load(manifest).exec()?;

        let intermediate = IntermediateManifest {
            repository: lua.from_value(lua.globals().get("repository")?)?,
        };
        let manifest = Self::from_intermediate(intermediate);

        Ok(manifest)
    }

    pub fn has_rock(&self, rock_name: &PackageName) -> bool {
        self.repository.contains_key(rock_name)
    }

    pub fn latest_match(
        &self,
        lua_package_req: &PackageReq,
        filter: Option<RemotePackageTypeFilterSpec>,
    ) -> Option<(PackageSpec, RemotePackageType)> {
        let filter = filter.unwrap_or_default();
        if !self.has_rock(lua_package_req.name()) {
            return None;
        }

        let (version, rock_type) = self.repository[lua_package_req.name()]
            .iter()
            .filter(|(version, _)| lua_package_req.version_req().matches(version))
            .flat_map(|(version, rock_types)| {
                rock_types.iter().filter_map(move |rock_type| {
                    let include = match rock_type {
                        RemotePackageType::Rockspec => filter.rockspec,
                        RemotePackageType::Src => filter.src,
                        RemotePackageType::Binary => filter.binary,
                    };
                    if include {
                        Some((version, rock_type))
                    } else {
                        None
                    }
                })
            })
            .max_by(
                |(version_a, type_a), (version_b, type_b)| match version_a.cmp(version_b) {
                    Ordering::Equal => type_a.cmp(type_b),
                    ordering => ordering,
                },
            )?;

        Some((
            PackageSpec::new(lua_package_req.name().clone(), version.clone()),
            rock_type.clone(),
        ))
    }

    /// Construct a `ManifestMetadata` from an intermediate representation,
    /// silently skipping entries for versions we don't know how to parse.
    fn from_intermediate(intermediate: IntermediateManifest) -> Self {
        let repository = intermediate
            .repository
            .into_iter()
            .map(|(name, package_map)| {
                (
                    name,
                    package_map
                        .into_iter()
                        .filter_map(|(version_str, entries)| {
                            let version = PackageVersion::parse(version_str.as_str()).ok()?;
                            let entries = entries
                                .into_iter()
                                .filter_map(|entry| RemotePackageType::try_from(entry).ok())
                                .collect_vec();
                            Some((version, entries))
                        })
                        .collect(),
                )
            })
            .collect();
        Self { repository }
    }
}

struct UnsupportedArchitectureError;

#[derive(Clone, serde::Deserialize)]
struct ManifestRockEntry {
    /// e.g. "linux-x86_64", "rockspec", "src", ...
    pub arch: String,
}

impl TryFrom<ManifestRockEntry> for RemotePackageType {
    type Error = UnsupportedArchitectureError;
    fn try_from(
        ManifestRockEntry { arch }: ManifestRockEntry,
    ) -> Result<Self, UnsupportedArchitectureError> {
        match arch.as_str() {
            "rockspec" => Ok(RemotePackageType::Rockspec),
            "src" => Ok(RemotePackageType::Src),
            "all" => Ok(RemotePackageType::Binary),
            arch if arch == crate::luarocks::current_platform_luarocks_identifier() => {
                Ok(RemotePackageType::Binary)
            }
            _ => Err(UnsupportedArchitectureError),
        }
    }
}

/// Intermediate implementation for deserializing
#[derive(serde::Deserialize)]
struct IntermediateManifest {
    /// The key of each package's HashMap is the version string
    repository: HashMap<PackageName, HashMap<String, Vec<ManifestRockEntry>>>,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tokio::fs;

    use crate::package::PackageReq;

    use super::*;

    #[tokio::test]
    pub async fn parse_metadata_from_empty_manifest() {
        let manifest = "
            commands = {}\n
            modules = {}\n
            repository = {}\n
            "
        .to_string();
        ManifestMetadata::new(&manifest).unwrap();
    }

    #[tokio::test]
    pub async fn parse_metadata_from_test_manifest() {
        let mut test_manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        test_manifest_path.push("resources/test/manifest-5.1");
        let manifest = String::from_utf8(fs::read(&test_manifest_path).await.unwrap()).unwrap();
        ManifestMetadata::new(&manifest).unwrap();
    }

    #[tokio::test]
    pub async fn latest_match_regression() {
        let mut test_manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        test_manifest_path.push("resources/test/manifest-5.1");
        let manifest = String::from_utf8(fs::read(&test_manifest_path).await.unwrap()).unwrap();
        let metadata = ManifestMetadata::new(&manifest).unwrap();

        let package_req: PackageReq = "30log > 1.3.0".parse().unwrap();
        assert!(metadata.latest_match(&package_req, None).is_none());
    }
}
