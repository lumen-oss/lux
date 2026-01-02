use itertools::Itertools;
use mlua::{ExternalResult, FromLua, IntoLua, LuaSerdeExt};
use serde::{de, Deserialize, Deserializer, Serialize};
use std::{cmp::Ordering, fmt::Display, str::FromStr};
use thiserror::Error;

mod outdated;
mod version;

pub use outdated::*;
pub use version::{
    PackageVersion, PackageVersionParseError, PackageVersionReq, PackageVersionReqError, SpecRev,
    VersionReqToVersionError,
};

pub(crate) use version::SpecRevIterator;

use crate::{
    lockfile::RemotePackageSourceUrl,
    lua_rockspec::{DisplayAsLuaKV, DisplayLuaKV, DisplayLuaValue},
    package::version::HasModRev,
    remote_package_source::RemotePackageSource,
    rockspec::lua_dependency::LuaDependencySpec,
    variables::{GetVariableError, HasVariables},
};

#[derive(Debug, Error)]
pub enum PackageSpecFromPackageReqError {
    #[error("invalid version for rock {rock}: {err}")]
    Version {
        rock: PackageName,
        err: VersionReqToVersionError,
    },
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "clap", derive(clap::Args))]
pub struct PackageSpec {
    name: PackageName,
    version: PackageVersion,
}

impl PackageSpec {
    pub fn new(name: PackageName, version: PackageVersion) -> Self {
        Self { name, version }
    }
    pub fn parse(name: String, version: String) -> Result<Self, PackageVersionParseError> {
        Ok(Self::new(
            PackageName::new(name),
            PackageVersion::parse(&version)?,
        ))
    }
    pub fn name(&self) -> &PackageName {
        &self.name
    }
    pub fn version(&self) -> &PackageVersion {
        &self.version
    }
    pub fn into_package_req(self) -> PackageReq {
        PackageReq {
            name: self.name,
            version_req: self.version.into_version_req(),
        }
    }
}

impl TryFrom<PackageReq> for PackageSpec {
    type Error = PackageSpecFromPackageReqError;

    fn try_from(value: PackageReq) -> Result<Self, Self::Error> {
        let name = value.name;
        let version = value.version_req.try_into().map_err(|err| {
            PackageSpecFromPackageReqError::Version {
                rock: name.clone(),
                err,
            }
        })?;
        Ok(Self { name, version })
    }
}

impl FromLua for PackageSpec {
    fn from_lua(
        value: mlua::prelude::LuaValue,
        lua: &mlua::prelude::Lua,
    ) -> mlua::prelude::LuaResult<Self> {
        let (name, version) = lua.from_value(value)?;

        Self::parse(name, version).into_lua_err()
    }
}

impl mlua::UserData for PackageSpec {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |_, this| Ok(this.name.to_string()));
        fields.add_field_method_get("version", |_, this| Ok(this.version.to_string()));
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("to_package_req", |_, this, ()| {
            Ok(this.clone().into_package_req())
        })
    }
}

impl HasVariables for PackageSpec {
    fn get_variable(&self, input: &str) -> Result<Option<String>, GetVariableError> {
        Ok(match input {
            "PACKAGE" => Some(self.name.to_string()),
            "VERSION" => Some(self.version.to_modrev_string()),
            _ => None,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RemotePackage {
    pub package: PackageSpec,
    pub source: RemotePackageSource,
    /// `Some` if present in a lockfile
    pub source_url: Option<RemotePackageSourceUrl>,
}

impl RemotePackage {
    pub fn new(
        package: PackageSpec,
        source: RemotePackageSource,
        source_url: Option<RemotePackageSourceUrl>,
    ) -> Self {
        Self {
            package,
            source,
            source_url,
        }
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub(crate) enum RemotePackageType {
    Rockspec,
    Src,
    Binary,
}

impl Ord for RemotePackageType {
    fn cmp(&self, other: &Self) -> Ordering {
        // Priority: binary > rockspec > src
        match (self, other) {
            (Self::Binary, Self::Binary)
            | (Self::Rockspec, Self::Rockspec)
            | (Self::Src, Self::Src) => Ordering::Equal,

            (Self::Binary, _) => Ordering::Greater,
            (_, Self::Binary) => Ordering::Less,
            (Self::Rockspec, Self::Src) => Ordering::Greater,
            (Self::Src, Self::Rockspec) => Ordering::Less,
        }
    }
}

impl PartialOrd for RemotePackageType {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone)]
pub struct RemotePackageTypeFilterSpec {
    /// Include Rockspec
    pub rockspec: bool,
    /// Include Src
    pub src: bool,
    /// Include Binary
    pub binary: bool,
}

impl Default for RemotePackageTypeFilterSpec {
    fn default() -> Self {
        Self {
            rockspec: true,
            src: true,
            binary: true,
        }
    }
}

#[derive(Error, Debug)]
pub enum ParseRemotePackageError {
    #[error("unable to parse package {0}. expected format: `name@version`")]
    InvalidInput(String),
    #[error("unable to parse package {package_str}: {error}")]
    InvalidPackageVersion {
        #[source]
        error: PackageVersionParseError,
        package_str: String,
    },
}

impl FromStr for PackageSpec {
    type Err = ParseRemotePackageError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let (name, version) = s
            .split_once('@')
            .ok_or_else(|| ParseRemotePackageError::InvalidInput(s.to_string()))?;

        Self::parse(name.to_string(), version.to_string()).map_err(|error| {
            ParseRemotePackageError::InvalidPackageVersion {
                error,
                package_str: s.to_string(),
            }
        })
    }
}

/// A lua package requirement with a name and an optional version requirement.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "clap", derive(clap::Args))]
pub struct PackageReq {
    /// The name of the package.
    pub(crate) name: PackageName,
    /// The version requirement, for example "1.0.0" or ">=1.0.0".
    pub(crate) version_req: PackageVersionReq,
}

impl PackageReq {
    pub fn new(name: String, version: Option<String>) -> Result<Self, PackageVersionReqError> {
        let version_req = match version {
            Some(version_req_str) => PackageVersionReq::parse(version_req_str.as_str())?,
            None => PackageVersionReq::any(),
        };
        Ok(Self {
            name: PackageName::new(name),
            version_req,
        })
    }

    /// # Safety
    ///
    /// Call this only if you are absolutely sure that a `PackageReq` can be parsed from the arguments.
    /// Calling this with invalid arguments is undefined behaviour.
    pub unsafe fn new_unchecked(name: String, version: Option<String>) -> Self {
        Self::new(name, version).unwrap_unchecked()
    }

    pub fn parse(pkg_constraints: &str) -> Result<Self, PackageReqParseError> {
        Self::from_str(pkg_constraints)
    }
    pub fn name(&self) -> &PackageName {
        &self.name
    }
    pub fn version_req(&self) -> &PackageVersionReq {
        &self.version_req
    }
    /// Evaluate whether the given package satisfies the package requirement
    /// given by `self`.
    pub fn matches(&self, package: &PackageSpec) -> bool {
        self.name == package.name && self.version_req.matches(&package.version)
    }
}

impl Display for PackageReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.version_req.is_any() {
            self.name.fmt(f)
        } else {
            f.write_str(format!("{}{}", self.name, self.version_req).as_str())
        }
    }
}

impl From<PackageSpec> for PackageReq {
    fn from(value: PackageSpec) -> Self {
        value.into_package_req()
    }
}

impl From<PackageName> for PackageReq {
    fn from(name: PackageName) -> Self {
        Self {
            name,
            version_req: PackageVersionReq::any(),
        }
    }
}

impl FromLua for PackageReq {
    fn from_lua(value: mlua::Value, lua: &mlua::Lua) -> mlua::Result<Self> {
        let str: String = lua.from_value(value)?;
        Self::parse(&str).into_lua_err()
    }
}

impl mlua::UserData for PackageReq {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("name", |_, this, ()| Ok(this.name.to_string()));
        methods.add_method("version_req", |_, this, ()| {
            Ok(this.version_req.to_string())
        });
        methods.add_method("matches", |_, this, package: PackageSpec| {
            Ok(this.matches(&package))
        });
    }
}

/// Wrapper structs for proper serialization of various dependency types.
pub(crate) struct Dependencies<'a>(pub(crate) &'a Vec<LuaDependencySpec>);
pub(crate) struct BuildDependencies<'a>(pub(crate) &'a Vec<LuaDependencySpec>);

impl DisplayAsLuaKV for Dependencies<'_> {
    fn display_lua(&self) -> DisplayLuaKV {
        DisplayLuaKV {
            key: "dependencies".to_string(),
            value: DisplayLuaValue::List(
                self.0
                    .iter()
                    .map(|req| DisplayLuaValue::String(req.to_string()))
                    .collect(),
            ),
        }
    }
}

impl DisplayAsLuaKV for BuildDependencies<'_> {
    fn display_lua(&self) -> DisplayLuaKV {
        DisplayLuaKV {
            key: "build_dependencies".to_string(),
            value: DisplayLuaValue::List(
                self.0
                    .iter()
                    .map(|req| DisplayLuaValue::String(req.to_string()))
                    .collect(),
            ),
        }
    }
}

#[derive(Error, Debug)]
pub enum PackageReqParseError {
    #[error("could not parse dependency name from {0}")]
    InvalidDependencyName(String),
    #[error("could not parse version requirement in '{str}': {error}")]
    InvalidPackageVersionReq {
        #[source]
        error: PackageVersionReqError,
        str: String,
    },
}

impl FromStr for PackageReq {
    type Err = PackageReqParseError;

    fn from_str(str: &str) -> Result<Self, PackageReqParseError> {
        let rock_name_str = str
            .chars()
            .peeking_take_while(|t| t.is_alphanumeric() || matches!(t, '-' | '_' | '.'))
            .collect::<String>();

        if rock_name_str.is_empty() {
            return Err(PackageReqParseError::InvalidDependencyName(str.to_string()));
        }

        let constraints = str.trim_start_matches(&rock_name_str).trim();
        let version_req = match constraints {
            "" => PackageVersionReq::any(),
            constraints => PackageVersionReq::parse(constraints.trim_start()).map_err(|error| {
                PackageReqParseError::InvalidPackageVersionReq {
                    error,
                    str: str.to_string(),
                }
            })?,
        };
        Ok(Self {
            name: PackageName::new(rock_name_str),
            version_req,
        })
    }
}

impl<'de> Deserialize<'de> for PackageReq {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).map_err(de::Error::custom)
    }
}

/// A luarocks package name, which is always lowercase
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
pub struct PackageName(String);

impl IntoLua for PackageName {
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        self.0.into_lua(lua)
    }
}

impl PackageName {
    pub fn new(name: String) -> Self {
        Self(name.to_lowercase())
    }
}

impl<'de> Deserialize<'de> for PackageName {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(PackageName::new(String::deserialize(deserializer)?))
    }
}

impl FromLua for PackageName {
    fn from_lua(
        value: mlua::prelude::LuaValue,
        lua: &mlua::prelude::Lua,
    ) -> mlua::prelude::LuaResult<Self> {
        Ok(Self::new(String::from_lua(value, lua)?))
    }
}

impl Serialize for PackageName {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl From<&str> for PackageName {
    fn from(value: &str) -> Self {
        Self::new(value.into())
    }
}

impl Display for PackageName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

#[derive(Debug)]
pub struct PackageNameList(Vec<PackageName>);

impl PackageNameList {
    pub(crate) fn new(package_names: Vec<PackageName>) -> Self {
        Self(package_names)
    }
}

impl Display for PackageNameList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.iter().map(|pkg| pkg.to_string()).join(", ").as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parse_name() {
        let mut package_name: PackageName = "neorg".into();
        assert_eq!(package_name.to_string(), "neorg");
        package_name = "LuaFileSystem".into();
        assert_eq!(package_name.to_string(), "luafilesystem");
    }

    #[tokio::test]
    async fn parse_lua_package() {
        let neorg = PackageSpec::parse("neorg".into(), "1.0.0".into()).unwrap();
        let expected_version = PackageVersion::parse("1.0.0").unwrap();
        assert_eq!(neorg.name().to_string(), "neorg");
        assert!(matches!(
            neorg.version().cmp(&expected_version),
            std::cmp::Ordering::Equal
        ));
        let neorg = PackageSpec::parse("neorg".into(), "1.0".into()).unwrap();
        assert!(matches!(
            neorg.version().cmp(&expected_version),
            std::cmp::Ordering::Equal
        ));
        let neorg = PackageSpec::parse("neorg".into(), "1".into()).unwrap();
        assert!(matches!(
            neorg.version().cmp(&expected_version),
            std::cmp::Ordering::Equal
        ));
    }

    #[tokio::test]
    async fn parse_lua_package_req() {
        let mut package_req = PackageReq::new("foo".into(), Some("1.0.0".into())).unwrap();
        assert!(package_req.matches(&PackageSpec::parse("foo".into(), "1.0.0".into()).unwrap()));
        assert!(!package_req.matches(&PackageSpec::parse("bar".into(), "1.0.0".into()).unwrap()));
        assert!(!package_req.matches(&PackageSpec::parse("foo".into(), "2.0.0".into()).unwrap()));
        package_req = PackageReq::new("foo".into(), Some(">= 1.0.0".into())).unwrap();
        assert!(package_req.matches(&PackageSpec::parse("foo".into(), "2.0.0".into()).unwrap()));
        let package_req: PackageReq = "lua >= 5.1".parse().unwrap();
        assert_eq!(package_req.name.to_string(), "lua");
        let package_req: PackageReq = "lua>=5.1".parse().unwrap();
        assert_eq!(package_req.name.to_string(), "lua");
        let package_req: PackageReq = "toml-edit >= 0.1.0".parse().unwrap();
        assert_eq!(package_req.name.to_string(), "toml-edit");
        let package_req: PackageReq = "plugin.nvim >= 0.1.0".parse().unwrap();
        assert_eq!(package_req.name.to_string(), "plugin.nvim");
        let package_req: PackageReq = "lfs".parse().unwrap();
        assert_eq!(package_req.name.to_string(), "lfs");
        let package_req: PackageReq = "neorg 1.0.0".parse().unwrap();
        assert_eq!(package_req.name.to_string(), "neorg");
        let neorg = PackageSpec::parse("neorg".into(), "1.0.0".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let neorg = PackageSpec::parse("neorg".into(), "2.0.0".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let package_req: PackageReq = "neorg 2.0.0".parse().unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: PackageReq = "neorg = 2.0.0".parse().unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: PackageReq = "neorg == 2.0.0".parse().unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: PackageReq = "neorg &equals; 2.0.0".parse().unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: PackageReq = "neorg >= 1.0, &lt; 2.0".parse().unwrap();
        let neorg = PackageSpec::parse("neorg".into(), "1.5".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: PackageReq = "neorg &gt; 1.0, &lt; 2.0".parse().unwrap();
        let neorg = PackageSpec::parse("neorg".into(), "1.11.0".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let neorg = PackageSpec::parse("neorg".into(), "3.0.0".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let neorg = PackageSpec::parse("neorg".into(), "0.5".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let package_req: PackageReq = "neorg ~> 1".parse().unwrap();
        assert!(!package_req.matches(&neorg));
        let neorg = PackageSpec::parse("neorg".into(), "3".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let neorg = PackageSpec::parse("neorg".into(), "1.5".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: PackageReq = "neorg ~> 1.4".parse().unwrap();
        let neorg = PackageSpec::parse("neorg".into(), "1.3".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let neorg = PackageSpec::parse("neorg".into(), "1.5".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let neorg = PackageSpec::parse("neorg".into(), "1.4.10".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let neorg = PackageSpec::parse("neorg".into(), "1.4".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: PackageReq = "neorg ~> 1.0.5".parse().unwrap();
        let neorg = PackageSpec::parse("neorg".into(), "1.0.4".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let neorg = PackageSpec::parse("neorg".into(), "1.0.5".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let neorg = PackageSpec::parse("neorg".into(), "1.0.6".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        // Testing incomplete version constraints
        let package_req: PackageReq = "lua-utils.nvim ~> 1.1-1".parse().unwrap();
        let lua_utils = PackageSpec::parse("lua-utils.nvim".into(), "1.1.4".into()).unwrap();
        assert!(package_req.matches(&lua_utils));
        let lua_utils = PackageSpec::parse("lua-utils.nvim".into(), "1.1.5".into()).unwrap();
        assert!(package_req.matches(&lua_utils));
        let lua_utils = PackageSpec::parse("lua-utils.nvim".into(), "1.2-1".into()).unwrap();
        assert!(!package_req.matches(&lua_utils));
    }

    #[tokio::test]
    pub async fn remote_package_type_priorities() {
        let rock_types = vec![
            RemotePackageType::Binary,
            RemotePackageType::Src,
            RemotePackageType::Rockspec,
        ];
        assert_eq!(
            rock_types.into_iter().sorted().collect_vec(),
            vec![
                RemotePackageType::Src,
                RemotePackageType::Rockspec,
                RemotePackageType::Binary,
            ]
        );
    }
}
