use itertools::Itertools;

use miette::Diagnostic;
use serde::{de, Deserialize, Deserializer, Serialize};
use std::{cmp::Ordering, fmt::Display, str::FromStr};
use thiserror::Error;
mod outdated;
pub mod version;

pub use outdated::*;
pub use version::{Version, VersionReq};

use version::{HasModRev, ParseError, ReqError, ReqToVersionError};

pub(crate) use version::SpecRevIterator;

use crate::{
    lockfile::RemotePackageSourceUrl,
    lua_rockspec::{DisplayAsLuaKV, DisplayLuaKV, DisplayLuaValue},
    remote_package_source::RemotePackageSource,
    rockspec::lua_dependency::LuaDependencySpec,
    variables::{GetVariableError, HasVariables},
};
#[derive(Clone, Debug)]
pub(crate) struct RemotePackage {
    pub spec: Spec,
    pub source: RemotePackageSource,
    /// `Some` if present in a lockfile
    pub source_url: Option<RemotePackageSourceUrl>,
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

#[derive(Clone, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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

#[derive(Error, Debug, Diagnostic)]
pub enum ParseRemotePackageError {
    #[error("unable to parse package {0}. expected format: `name@version`")]
    InvalidInput(String),
    #[error("unable to parse package {package_str}:\n{error}")]
    InvalidPackageVersion {
        #[source]
        error: ParseError,
        package_str: String,
    },
}

/// A lua package requirement with a name and an optional version requirement.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "clap", derive(clap::Args))]
pub struct Req {
    /// The name of the package.
    pub(crate) name: Name,
    /// The version requirement, for example "1.0.0" or ">=1.0.0".
    pub(crate) version_req: VersionReq,
}

impl Display for Req {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.version_req.is_any() {
            self.name.fmt(f)
        } else {
            f.write_str(format!("{}{}", self.name, self.version_req).as_str())
        }
    }
}

impl From<Spec> for Req {
    fn from(value: Spec) -> Self {
        value.into_package_req()
    }
}

impl From<Name> for Req {
    fn from(name: Name) -> Self {
        Self {
            name,
            version_req: VersionReq::any(),
        }
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

#[derive(Error, Debug, Diagnostic)]
pub enum ReqParseError {
    #[error("could not parse dependency name from '{0}'")]
    InvalidDependencyName(String),
    #[error("could not parse version requirement in '{str}': {error}")]
    InvalidPackageVersionReq {
        #[source]
        error: ReqError,
        str: String,
    },
}

impl FromStr for Req {
    type Err = ReqParseError;

    fn from_str(str: &str) -> Result<Self, ReqParseError> {
        let rock_name_str = str
            .chars()
            .peeking_take_while(|t| t.is_alphanumeric() || matches!(t, '-' | '_' | '.'))
            .collect::<String>();

        if rock_name_str.is_empty() {
            return Err(ReqParseError::InvalidDependencyName(str.to_string()));
        }

        let constraints = str.trim_start_matches(&rock_name_str).trim();
        let version_req = match constraints {
            "" => VersionReq::any(),
            constraints => VersionReq::parse(constraints.trim_start()).map_err(|error| {
                ReqParseError::InvalidPackageVersionReq {
                    error,
                    str: str.to_string(),
                }
            })?,
        };
        Ok(Self {
            name: Name::new(rock_name_str),
            version_req,
        })
    }
}

impl<'de> Deserialize<'de> for Req {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).map_err(de::Error::custom)
    }
}

impl RemotePackage {
    pub fn new(
        spec: Spec,
        source: RemotePackageSource,
        source_url: Option<RemotePackageSourceUrl>,
    ) -> Self {
        Self {
            spec,
            source,
            source_url,
        }
    }
}

impl Req {
    pub fn new(name: String, version: Option<String>) -> Result<Self, ReqError> {
        let version_req = match version {
            Some(version_req_str) => VersionReq::parse(version_req_str.as_str())?,
            None => VersionReq::any(),
        };
        Ok(Self {
            name: Name::new(name),
            version_req,
        })
    }

    /// # Safety
    ///
    /// Call this only if you are absolutely sure that a `Req` can be parsed from the arguments.
    /// Calling this with invalid arguments is undefined behaviour.
    pub unsafe fn new_unchecked(name: String, version: Option<String>) -> Self {
        Self::new(name, version).unwrap_unchecked()
    }

    pub fn parse(pkg_constraints: &str) -> Result<Self, ReqParseError> {
        Self::from_str(pkg_constraints)
    }
    pub fn name(&self) -> &Name {
        &self.name
    }
    pub fn version_req(&self) -> &VersionReq {
        &self.version_req
    }
    /// Evaluate whether the given package satisfies the package requirement given by `self`.
    pub fn matches(&self, package: &Spec) -> bool {
        self.name() == package.name() && self.version_req.matches(package.version())
    }
}

/// A luarocks package name, which is always lowercase
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", serde(transparent))]
pub struct Name(String);

impl Name {
    pub fn new(name: String) -> Self {
        Self(name.to_lowercase())
    }
}

impl<'de> Deserialize<'de> for Name {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Name::new(String::deserialize(deserializer)?))
    }
}

impl Serialize for Name {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl From<&str> for Name {
    fn from(value: &str) -> Self {
        Self::new(value.into())
    }
}

impl Display for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

#[derive(Debug)]
pub struct List(Vec<Name>);

impl List {
    pub(crate) fn new(package_names: Vec<Name>) -> Self {
        Self(package_names)
    }
}

impl Display for List {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.iter().map(|pkg| pkg.to_string()).join(", ").as_str())
    }
}

#[derive(Debug, Error, Diagnostic)]
pub enum SpecFromReqError {
    #[error("invalid version for rock {rock}: {err}")]
    Version { rock: Name, err: ReqToVersionError },
}

/// Specification for a package with an exact name and version
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::Args))]
pub struct Spec {
    name: Name,
    version: Version,
}

impl Spec {
    pub fn new(name: Name, version: Version) -> Self {
        Self { name, version }
    }
    pub fn parse(name: String, version: String) -> Result<Self, ParseError> {
        Ok(Self::new(Name::new(name), Version::parse(&version)?))
    }
    pub fn name(&self) -> &Name {
        &self.name
    }
    pub fn version(&self) -> &Version {
        &self.version
    }
    /// Note that the version requirement loses the specrev information.
    pub fn into_package_req(self) -> Req {
        Req {
            name: self.name,
            version_req: self.version.into_version_req(),
        }
    }
}

impl TryFrom<Req> for Spec {
    type Error = SpecFromReqError;

    fn try_from(value: Req) -> Result<Self, Self::Error> {
        let name = value.name;
        let version = value
            .version_req
            .try_into()
            .map_err(|err| SpecFromReqError::Version {
                rock: name.clone(),
                err,
            })?;
        Ok(Self { name, version })
    }
}

impl<'de> Deserialize<'de> for Spec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let (name, version) = <_>::deserialize(deserializer)?;
        Self::parse(name, version).map_err(serde::de::Error::custom)
    }
}

impl HasVariables for Spec {
    fn get_variable(&self, input: &str) -> Result<Option<String>, GetVariableError> {
        Ok(match input {
            "PACKAGE" => Some(self.name.to_string()),
            "VERSION" => Some(self.version.to_modrev_string()),
            _ => None,
        })
    }
}

impl FromStr for Spec {
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::package::version::Version;

    #[test]
    fn parse_name() {
        let mut package_name: Name = "neorg".into();
        assert_eq!(package_name.to_string(), "neorg");
        package_name = "LuaFileSystem".into();
        assert_eq!(package_name.to_string(), "luafilesystem");
    }

    #[test]
    fn parse_lua_package() {
        let neorg = Spec::parse("neorg".into(), "1.0.0".into()).unwrap();
        let expected_version = Version::parse("1.0.0").unwrap();
        assert_eq!(neorg.name().to_string(), "neorg");
        assert!(matches!(
            neorg.version().cmp(&expected_version),
            std::cmp::Ordering::Equal
        ));
        let neorg = Spec::parse("neorg".into(), "1.0".into()).unwrap();
        assert!(matches!(
            neorg.version().cmp(&expected_version),
            std::cmp::Ordering::Equal
        ));
        let neorg = Spec::parse("neorg".into(), "1".into()).unwrap();
        assert!(matches!(
            neorg.version().cmp(&expected_version),
            std::cmp::Ordering::Equal
        ));
    }

    #[test]
    fn parse_lua_package_req() {
        let mut package_req = Req::new("foo".into(), Some("1.0.0".into())).unwrap();
        assert!(package_req.matches(&Spec::parse("foo".into(), "1.0.0".into()).unwrap()));
        assert!(!package_req.matches(&Spec::parse("bar".into(), "1.0.0".into()).unwrap()));
        assert!(!package_req.matches(&Spec::parse("foo".into(), "2.0.0".into()).unwrap()));
        package_req = Req::new("foo".into(), Some(">= 1.0.0".into())).unwrap();
        assert!(package_req.matches(&Spec::parse("foo".into(), "2.0.0".into()).unwrap()));
        let package_req: Req = "lua >= 5.1".parse().unwrap();
        assert_eq!(package_req.name.to_string(), "lua");
        let package_req: Req = "lua>=5.1".parse().unwrap();
        assert_eq!(package_req.name.to_string(), "lua");
        let package_req: Req = "toml-edit >= 0.1.0".parse().unwrap();
        assert_eq!(package_req.name.to_string(), "toml-edit");
        let package_req: Req = "plugin.nvim >= 0.1.0".parse().unwrap();
        assert_eq!(package_req.name.to_string(), "plugin.nvim");
        let package_req: Req = "lfs".parse().unwrap();
        assert_eq!(package_req.name.to_string(), "lfs");
        let package_req: Req = "neorg 1.0.0".parse().unwrap();
        assert_eq!(package_req.name.to_string(), "neorg");
        let neorg = Spec::parse("neorg".into(), "1.0.0".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "2.0.0".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let package_req: Req = "neorg 2.0.0".parse().unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: Req = "neorg = 2.0.0".parse().unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: Req = "neorg == 2.0.0".parse().unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: Req = "neorg &equals; 2.0.0".parse().unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: Req = "neorg >= 1.0, &lt; 2.0".parse().unwrap();
        let neorg = Spec::parse("neorg".into(), "1.5".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: Req = "neorg &gt; 1.0, &lt; 2.0".parse().unwrap();
        let neorg = Spec::parse("neorg".into(), "1.11.0".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "3.0.0".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "0.5".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let package_req: Req = "neorg ~> 1".parse().unwrap();
        assert!(!package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "3".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "1.5".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: Req = "neorg ~> 1.4".parse().unwrap();
        let neorg = Spec::parse("neorg".into(), "1.3".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "1.5".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "1.4.10".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "1.4".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let package_req: Req = "neorg ~> 1.0.5".parse().unwrap();
        let neorg = Spec::parse("neorg".into(), "1.0.4".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "1.0.5".into()).unwrap();
        assert!(package_req.matches(&neorg));
        let neorg = Spec::parse("neorg".into(), "1.0.6".into()).unwrap();
        assert!(!package_req.matches(&neorg));
        // Testing incomplete version constraints
        let package_req: Req = "lua-utils.nvim ~> 1.1-1".parse().unwrap();
        let lua_utils = Spec::parse("lua-utils.nvim".into(), "1.1.4".into()).unwrap();
        assert!(package_req.matches(&lua_utils));
        let lua_utils = Spec::parse("lua-utils.nvim".into(), "1.1.5".into()).unwrap();
        assert!(package_req.matches(&lua_utils));
        let lua_utils = Spec::parse("lua-utils.nvim".into(), "1.2-1".into()).unwrap();
        assert!(!package_req.matches(&lua_utils));
    }

    #[test]
    pub fn display_package_req() {
        let package_req = Req::new("foo".into(), Some("1.0.0".into())).unwrap();
        assert_eq!(package_req.to_string(), "foo==1.0.0");
        let package_req = Req::new("foo".into(), Some(">=1.0.0".into())).unwrap();
        assert_eq!(package_req.to_string(), "foo>=1.0.0");
        let package_req = Req::new("foo".into(), Some(">1.0.0".into())).unwrap();
        assert_eq!(package_req.to_string(), "foo>1.0.0");
        let package_req = Req::new("foo".into(), Some("<=1.0.0".into())).unwrap();
        assert_eq!(package_req.to_string(), "foo<=1.0.0");
        let package_req = Req::new("foo".into(), Some("<1.0.0".into())).unwrap();
        assert_eq!(package_req.to_string(), "foo<1.0.0");
        let package_req = Req::new("foo".into(), Some("~>1.0".into())).unwrap();
        assert_eq!(package_req.to_string(), "foo>=1.0.0, <1.1.0");
        let package_req = Req::new("foo".into(), Some(">=1.0.0, <1.1.0".into())).unwrap();
        assert_eq!(package_req.to_string(), "foo>=1.0.0, <1.1.0");
        let package_req = Req::new("foo".into(), None).unwrap();
        assert_eq!(package_req.to_string(), "foo");
        let package_req = Req::new("foo".into(), Some("*".into())).unwrap();
        assert_eq!(package_req.to_string(), "foo");
    }

    #[test]
    pub fn remote_package_type_priorities() {
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

    #[test]
    fn package_spec_req_roundtrips() {
        let spec = Spec::new("foo".into(), "1.0.0-1".parse().unwrap());
        let req: Req = spec.clone().into();
        assert_eq!(spec, req.try_into().unwrap());

        let spec = Spec::new("foo".into(), "1.0.0".parse().unwrap());
        let req: Req = spec.clone().into();
        assert_eq!(spec, req.try_into().unwrap());
    }

    #[test]
    fn package_req_spec_roundtrips() {
        let req: Req = "foo@1.0.0".parse().unwrap();
        let spec: Spec = req.clone().try_into().unwrap();
        assert_eq!(req, spec.into());

        let req: Req = "foo@1.0.0-1".parse().unwrap();
        let spec: Spec = req.clone().try_into().unwrap();
        assert_eq!(req, spec.into());
    }

    #[test]
    fn package_spec_parses_same_as_name_version() {
        let req: Req = "foo@1.0.0".parse().unwrap();
        let spec: Spec = req.clone().try_into().unwrap();
        let name: Name = "foo".into();
        let version: Version = "1.0.0".parse().unwrap();
        assert_eq!(spec, Spec::new(name, version));

        let req: Req = "foo@1.0.0-1".parse().unwrap();
        let spec: Spec = req.clone().try_into().unwrap();
        let name: Name = "foo".into();
        let version: Version = "1.0.0-1".parse().unwrap();
        assert_eq!(spec, Spec::new(name, version));
    }
}
