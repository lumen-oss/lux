use crate::package;
use crate::remote;
use crate::ROCKSPEC_FUEL_LIMIT;
use itertools::Itertools;
use miette::Diagnostic;
use ottavino::{Closure, Executor, Fuel, Lua};
use ottavino_util::serde::from_value;
use std::{cmp::Ordering, collections::HashMap};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Metadata {
    pub repository: HashMap<package::Name, HashMap<package::Version, Vec<remote::package::Kind>>>,
}

impl<'de> serde::Deserialize<'de> for Metadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let intermediate = IntermediateManifest::deserialize(deserializer)?;
        Ok(Self::from_intermediate(intermediate))
    }
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum LuaError {
    #[error("failed to parse Lua manifest")]
    ExecutionError(#[from] ottavino::ExternError),
    #[error("failed to deserialize Lua manifest")]
    DeserializationError(#[from] ottavino_util::serde::de::Error),
    #[error("manifest exceeds computational limit of {ROCKSPEC_FUEL_LIMIT} steps")]
    #[diagnostic(help(
        r#"the rockspec may be too complex or contain too many dependencies.
        Try reducing the number of dependencies or simplifying build instructions."#
    ))]
    FuelLimitExceeded,
}

impl Metadata {
    pub fn new(manifest: &String) -> Result<Self, LuaError> {
        let mut lua = Lua::core();

        let success = lua.try_enter(|ctx| {
            let closure = Closure::load(ctx, None, manifest.as_bytes())?;

            let executor = Executor::start(ctx, closure.into(), ());

            Ok(executor.step(ctx, &mut Fuel::with(ROCKSPEC_FUEL_LIMIT))?)
        })?;

        if !success {
            return Err(LuaError::FuelLimitExceeded);
        }

        let intermediate = IntermediateManifest {
            repository: lua.enter(|ctx| from_value(ctx.globals().get_value(ctx, "repository")))?,
        };

        let manifest = Self::from_intermediate(intermediate);

        Ok(manifest)
    }

    pub fn has_rock(&self, rock_name: &package::Name) -> bool {
        self.repository.contains_key(rock_name)
    }

    pub fn latest_match(
        &self,
        lua_package_req: &package::Req,
        filter: &remote::package::kind::FilterSpec,
    ) -> Option<(package::Spec, remote::package::Kind)> {
        if !self.has_rock(lua_package_req.name()) {
            return None;
        }

        let (version, rock_type) = self.repository[lua_package_req.name()]
            .iter()
            .filter(|(version, _)| lua_package_req.version_req().matches(version))
            .flat_map(|(version, rock_types)| {
                rock_types.iter().filter_map(move |rock_type| {
                    let include = match rock_type {
                        remote::package::Kind::Rockspec => filter.rockspec,
                        remote::package::Kind::Src => filter.src,
                        remote::package::Kind::Binary => filter.binary,
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
            package::Spec::new(lua_package_req.name().clone(), version.clone()),
            rock_type.clone(),
        ))
    }

    /// Construct a `Metadata` from an intermediate representation,
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
                            let version = package::Version::parse(version_str.as_str()).ok()?;
                            let entries = entries
                                .into_iter()
                                .filter_map(|entry| remote::package::Kind::try_from(entry).ok())
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

impl TryFrom<ManifestRockEntry> for remote::package::Kind {
    type Error = UnsupportedArchitectureError;
    fn try_from(
        ManifestRockEntry { arch }: ManifestRockEntry,
    ) -> Result<Self, UnsupportedArchitectureError> {
        match arch.as_str() {
            "rockspec" => Ok(remote::package::Kind::Rockspec),
            "src" => Ok(remote::package::Kind::Src),
            "all" => Ok(remote::package::Kind::Binary),
            arch if arch == crate::luarocks::current_platform_luarocks_identifier() => {
                Ok(remote::package::Kind::Binary)
            }
            _ => Err(UnsupportedArchitectureError),
        }
    }
}

/// Intermediate implementation for deserializing
#[derive(serde::Deserialize)]
struct IntermediateManifest {
    /// The key of each package's HashMap is the version string
    repository: HashMap<package::Name, HashMap<String, Vec<ManifestRockEntry>>>,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tokio::fs;

    use super::*;

    #[tokio::test]
    pub async fn parse_metadata_from_empty_manifest() {
        let manifest = "
            commands = {}\n
            modules = {}\n
            repository = {}\n
            "
        .to_string();
        Metadata::new(&manifest).unwrap();
    }

    #[tokio::test]
    pub async fn parse_metadata_from_test_manifest() {
        let test_manifest_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/manifest-5.1");
        let manifest = String::from_utf8(fs::read(&test_manifest_path).await.unwrap()).unwrap();
        Metadata::new(&manifest).unwrap();
    }

    #[tokio::test]
    pub async fn latest_match_regression() {
        let test_manifest_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/manifest-5.1");
        let manifest = String::from_utf8(fs::read(&test_manifest_path).await.unwrap()).unwrap();
        let metadata = Metadata::new(&manifest).unwrap();

        let package_req: package::Req = "30log > 1.3.0".parse().unwrap();
        assert!(metadata
            .latest_match(&package_req, &Default::default())
            .is_none());
    }
    #[tokio::test]
    pub async fn latest_match_respects_type_filter() {
        let test_manifest_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/manifest-5.1");
        let manifest = String::from_utf8(fs::read(&test_manifest_path).await.unwrap()).unwrap();
        let metadata = Metadata::new(&manifest).unwrap();
        let package_req: package::Req = "combine == 1.0-1".parse().unwrap();
        assert!(metadata
            .latest_match(&package_req, &Default::default())
            .is_some());
        let filter = remote::package::kind::FilterSpec {
            rockspec: false,
            src: true,
            binary: true,
        };
        assert!(metadata.latest_match(&package_req, &filter).is_none());
    }

    #[tokio::test]
    pub async fn latest_match_prefers_binary_then_rockspec_then_src() {
        let manifest = r#"
            commands = {}
            modules = {}
            repository = {
               ['testpkg'] = {
                  ['1.0-1'] = {
                     { arch = "rockspec" }, { arch = "src" }, { arch = "all" }
                  }
               }
            }
        "#
        .to_string();
        let metadata = Metadata::new(&manifest).unwrap();
        let package_req: package::Req = "testpkg == 1.0-1".parse().unwrap();

        let (_, rock_type) = metadata
            .latest_match(&package_req, &Default::default())
            .unwrap();
        assert_eq!(rock_type, remote::package::Kind::Binary);

        let no_binary = remote::package::kind::FilterSpec {
            rockspec: true,
            src: true,
            binary: false,
        };
        let (_, rock_type) = metadata.latest_match(&package_req, &no_binary).unwrap();
        assert_eq!(rock_type, remote::package::Kind::Rockspec);

        let src_only = remote::package::kind::FilterSpec {
            rockspec: false,
            src: true,
            binary: false,
        };
        let (_, rock_type) = metadata.latest_match(&package_req, &src_only).unwrap();
        assert_eq!(rock_type, remote::package::Kind::Src);
    }
}
