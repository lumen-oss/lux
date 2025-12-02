use std::{
    cmp::{self, Ordering},
    fmt::Display,
    str::FromStr,
};

use html_escape::decode_html_entities;
use itertools::Itertools;
use mlua::{ExternalResult, FromLua, IntoLua};
use nonempty::NonEmpty;
use semver::{Comparator, Error, Op, Version, VersionReq};
use serde::{de, Deserialize, Deserializer, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VersionReqToVersionError {
    #[error("cannot parse version from non-exact version requirement '{0}'")]
    NonExactVersionReq(VersionReq),
    #[error("cannot parse version from version requirement '*' (any version)")]
    Any,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum PackageVersion {
    /// **SemVer version** as defined by <https://semver.org>,
    /// but a bit more lenient for compatibility with luarocks
    SemVer(SemVer),
    /// A known **Dev** version
    DevVer(DevVer),
    /// An arbitrary string version.
    /// Yes, luarocks-site allows arbitrary string versions in the root manifest
    /// ┻━┻ ︵ヽ(`Д´)ﾉ︵ ┻━┻
    StringVer(StringVer),
}

impl HasModRev for PackageVersion {
    fn to_modrev_string(&self) -> String {
        match self {
            Self::SemVer(ver) => ver.to_modrev_string(),
            Self::DevVer(ver) => ver.to_modrev_string(),
            Self::StringVer(ver) => ver.to_modrev_string(),
        }
    }
}

impl PackageVersion {
    pub fn parse(text: &str) -> Result<Self, PackageVersionParseError> {
        PackageVersion::from_str(text)
    }
    /// Note that this loses the specrev information.
    pub fn into_version_req(&self) -> PackageVersionReq {
        match self {
            PackageVersion::DevVer(DevVer { modrev, .. }) => {
                PackageVersionReq::DevVer(modrev.to_owned())
            }
            PackageVersion::StringVer(StringVer { modrev, .. }) => {
                PackageVersionReq::StringVer(modrev.to_owned())
            }
            PackageVersion::SemVer(SemVer { version, .. }) => {
                let version = version.to_owned();
                PackageVersionReq::SemVer(VersionReq {
                    comparators: vec![Comparator {
                        op: Op::Exact,
                        major: version.major,
                        minor: Some(version.minor),
                        patch: Some(version.patch),
                        pre: version.pre,
                    }],
                })
            }
        }
    }

    pub(crate) fn is_semver(&self) -> bool {
        matches!(self, PackageVersion::SemVer(_))
    }

    pub(crate) fn default_dev_version() -> Self {
        Self::DevVer(DevVer::default())
    }

    pub(crate) fn default_dev_version_with_specrev(specrev: SpecRev) -> Self {
        Self::DevVer(DevVer::default_with_specrev(specrev))
    }
}

impl TryFrom<PackageVersionReq> for PackageVersion {
    type Error = VersionReqToVersionError;

    fn try_from(req: PackageVersionReq) -> Result<Self, Self::Error> {
        match req {
            PackageVersionReq::SemVer(version_req) => {
                match NonEmpty::try_from(version_req.comparators.clone()) {
                    Ok(comparators)
                        if comparators
                            .iter()
                            .any(|comparator| comparator.op != semver::Op::Exact) =>
                    {
                        Err(VersionReqToVersionError::NonExactVersionReq(version_req))
                    }
                    Ok(comparators) => {
                        let comparator = comparators.first();
                        let version = semver::Version {
                            major: comparator.major,
                            minor: comparator.minor.unwrap_or(0),
                            patch: comparator.patch.unwrap_or(0),
                            pre: comparator.pre.clone(),
                            build: semver::BuildMetadata::EMPTY,
                        };
                        let component_count = if comparator.patch.is_some() {
                            3
                        } else if comparator.minor.is_some() {
                            2
                        } else {
                            1
                        };
                        Ok(PackageVersion::SemVer(SemVer {
                            version,
                            component_count,
                            specrev: 1.into(),
                        }))
                    }
                    Err(_) => Err(VersionReqToVersionError::NonExactVersionReq(version_req)),
                }
            }
            PackageVersionReq::DevVer(modrev) => Ok(PackageVersion::DevVer(DevVer {
                modrev,
                specrev: 1.into(),
            })),
            PackageVersionReq::StringVer(modrev) => Ok(PackageVersion::StringVer(StringVer {
                modrev,
                specrev: 1.into(),
            })),
            PackageVersionReq::Any => Err(VersionReqToVersionError::Any),
        }
    }
}

impl IntoLua for PackageVersion {
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        self.to_string().into_lua(lua)
    }
}

#[derive(Error, Debug)]
pub enum PackageVersionParseError {
    #[error(transparent)]
    Specrev(#[from] SpecrevParseError),
    #[error("failed to parse version: {0}")]
    Version(#[from] Error),
}

impl Serialize for PackageVersion {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            PackageVersion::SemVer(version) => version.serialize(serializer),
            PackageVersion::DevVer(version) => version.serialize(serializer),
            PackageVersion::StringVer(version) => version.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for PackageVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).map_err(de::Error::custom)
    }
}

impl FromLua for PackageVersion {
    fn from_lua(
        value: mlua::prelude::LuaValue,
        lua: &mlua::prelude::Lua,
    ) -> mlua::prelude::LuaResult<Self> {
        let s = String::from_lua(value, lua)?;
        Self::from_str(&s).map_err(|err| mlua::Error::DeserializeError(err.to_string()))
    }
}

impl Display for PackageVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackageVersion::SemVer(version) => version.fmt(f),
            PackageVersion::DevVer(version) => version.fmt(f),
            PackageVersion::StringVer(version) => version.fmt(f),
        }
    }
}

impl PartialOrd for PackageVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PackageVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (PackageVersion::SemVer(a), PackageVersion::SemVer(b)) => a.cmp(b),
            (PackageVersion::SemVer(..), PackageVersion::DevVer(..)) => Ordering::Less,
            (PackageVersion::SemVer(..), PackageVersion::StringVer(..)) => Ordering::Greater,
            (PackageVersion::DevVer(..), PackageVersion::SemVer(..)) => Ordering::Greater,
            (PackageVersion::DevVer(a), PackageVersion::DevVer(b)) => a.cmp(b),
            (PackageVersion::DevVer(..), PackageVersion::StringVer(..)) => Ordering::Greater,
            (PackageVersion::StringVer(a), PackageVersion::StringVer(b)) => a.cmp(b),
            (PackageVersion::StringVer(..), PackageVersion::SemVer(..)) => Ordering::Less,
            (PackageVersion::StringVer(..), PackageVersion::DevVer(..)) => Ordering::Less,
        }
    }
}

impl FromStr for PackageVersion {
    type Err = PackageVersionParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (modrev, specrev) = split_specrev(text)?;
        match modrev {
            "scm" => Ok(PackageVersion::DevVer(DevVer {
                modrev: DevVersion::Scm,
                specrev,
            })),
            "dev" => Ok(PackageVersion::DevVer(DevVer {
                modrev: DevVersion::Dev,
                specrev,
            })),
            modrev => match parse_version(modrev) {
                Ok(version) => Ok(PackageVersion::SemVer(SemVer {
                    component_count: cmp::min(text.chars().filter(|c| *c == '.').count() + 1, 3),
                    version,
                    specrev,
                })),
                Err(_) => Ok(PackageVersion::StringVer(StringVer {
                    modrev: modrev.into(),
                    specrev,
                })),
            },
        }
    }
}

/// The revision of a rov
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct SpecRev(u16);

impl From<u16> for SpecRev {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

impl Display for SpecRev {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Default for SpecRev {
    fn default() -> Self {
        Self(1)
    }
}

impl FromLua for SpecRev {
    fn from_lua(value: mlua::Value, _lua: &mlua::Lua) -> mlua::Result<Self> {
        match value {
            mlua::Value::Integer(value) => {
                let value = u16::try_from(value).map_err(|err| {
                    mlua::Error::DeserializeError(format!(
                        "Error deserializing specrev {value}:\n{err}"
                    ))
                })?;
                Ok(Self(value))
            }
            value => Err(mlua::Error::DeserializeError(format!(
                "Expected specrev to be an integer, but got {}",
                value.type_name()
            ))),
        }
    }
}

/// Iterates `SpecRev`s upwards, starting from `1`.
pub(crate) struct SpecRevIterator {
    current: u16,
}

impl SpecRevIterator {
    pub fn new() -> Self {
        SpecRevIterator {
            current: SpecRev::default().0,
        }
    }
}

impl Iterator for SpecRevIterator {
    type Item = SpecRev;

    fn next(&mut self) -> Option<Self::Item> {
        self.current += 1;
        if self.current == 0 {
            None // overflow
        } else {
            Some(SpecRev(self.current))
        }
    }
}

// TODO: Stop deriving Eq here
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct SemVer {
    version: Version,
    component_count: usize,
    specrev: SpecRev,
}

impl HasModRev for SemVer {
    fn to_modrev_string(&self) -> String {
        self.version.to_string()
    }
}

impl Display for SemVer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (version_str, remainder) = split_semver_version(&self.version.to_string());
        let mut luarocks_version_str = version_str.split('.').take(self.component_count).join(".");
        if let Some(remainder) = remainder {
            // luarocks allows and arbitrary number of '.' separators
            // We treat anything after the third '.' as a semver prerelease/build version,
            // so we have to convert it back for luarocks.
            luarocks_version_str.push_str(&format!(".{remainder}"));
        }
        let str = format!("{}-{}", luarocks_version_str, self.specrev);
        str.fmt(f)
    }
}

fn split_semver_version(version_str: &str) -> (String, Option<String>) {
    if let Some(pos) = version_str.rfind('-') {
        if let Some(pre_build_str) = version_str.get(pos + 1..) {
            (version_str[..pos].into(), Some(pre_build_str.into()))
        } else {
            (version_str[..pos].into(), None)
        }
    } else {
        (version_str.into(), None)
    }
}

impl Serialize for SemVer {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_string().serialize(serializer)
    }
}

impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> Ordering {
        let result = self.version.cmp(&other.version);
        if result == Ordering::Equal {
            return self.specrev.cmp(&other.specrev);
        }
        result
    }
}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Eq, PartialEq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DevVersion {
    #[default]
    Dev,
    Scm,
}

impl Display for DevVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dev => "dev".fmt(f),
            Self::Scm => "scm".fmt(f),
        }
    }
}

impl IntoLua for DevVersion {
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        self.to_string().into_lua(lua)
    }
}

#[derive(Clone, Eq, PartialEq, Hash, Debug, Default)]
pub struct DevVer {
    modrev: DevVersion,
    specrev: SpecRev,
}

impl DevVer {
    fn default_with_specrev(specrev: SpecRev) -> Self {
        Self {
            modrev: DevVersion::default(),
            specrev,
        }
    }
}

impl HasModRev for DevVer {
    fn to_modrev_string(&self) -> String {
        self.modrev.to_string().to_lowercase()
    }
}

impl Display for DevVer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let str = format!("{}-{}", self.modrev, self.specrev);
        str.fmt(f)
    }
}

impl Serialize for DevVer {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_string().serialize(serializer)
    }
}

impl Ord for DevVer {
    fn cmp(&self, other: &Self) -> Ordering {
        // NOTE: We compare specrevs first for dev versions
        let result = self.specrev.cmp(&other.specrev);
        if result == Ordering::Equal {
            return self.modrev.cmp(&other.modrev);
        }
        result
    }
}

impl PartialOrd for DevVer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct StringVer {
    modrev: String,
    specrev: SpecRev,
}

impl HasModRev for StringVer {
    fn to_modrev_string(&self) -> String {
        self.modrev.to_string()
    }
}

impl Display for StringVer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let str = format!("{}-{}", self.modrev, self.specrev);
        str.fmt(f)
    }
}

impl Serialize for StringVer {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_string().serialize(serializer)
    }
}

impl Ord for StringVer {
    fn cmp(&self, other: &Self) -> Ordering {
        // NOTE: We compare specrevs first for dev versions
        let result = self.specrev.cmp(&other.specrev);
        if result == Ordering::Equal {
            return self.modrev.cmp(&other.modrev);
        }
        result
    }
}

impl PartialOrd for StringVer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub(crate) trait HasModRev {
    /// If a version has a modrev (and possibly a specrev),
    /// this is equivalent to `to_string()`, but includes only the modrev.
    fn to_modrev_string(&self) -> String;
}

#[derive(Error, Debug)]
#[error(transparent)]
pub struct PackageVersionReqError(#[from] Error);

/// **SemVer version** requirement as defined by <https://semver.org>.
/// or a **Dev** version requirement, which can be one of "dev", "scm", or "git"
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum PackageVersionReq {
    /// A PackageVersionReq that matches a SemVer version.
    SemVer(VersionReq),
    /// A PackageVersionReq that matches only known dev versions.
    DevVer(DevVersion),
    /// A PackageVersionReq that matches a arbitrary string version.
    StringVer(String),
    /// A PackageVersionReq that has no version constraint.
    Any,
}

impl FromLua for PackageVersionReq {
    fn from_lua(value: mlua::Value, lua: &mlua::Lua) -> mlua::Result<Self> {
        PackageVersionReq::parse(&String::from_lua(value, lua)?).into_lua_err()
    }
}

impl IntoLua for PackageVersionReq {
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        let table = lua.create_table()?;

        match self {
            PackageVersionReq::SemVer(version_req) => {
                table.set("semver", version_req.to_string())?
            }
            PackageVersionReq::DevVer(dev) => table.set("dev", dev)?,
            PackageVersionReq::StringVer(dev) => table.set("stringver", dev)?,
            PackageVersionReq::Any => table.set("any", true)?,
        }

        Ok(mlua::Value::Table(table))
    }
}

impl PackageVersionReq {
    /// Returns a `PackageVersionReq` that matches any version.
    pub fn any() -> Self {
        PackageVersionReq::Any
    }

    pub fn parse(text: &str) -> Result<Self, PackageVersionReqError> {
        PackageVersionReq::from_str(text)
    }

    pub fn matches(&self, version: &PackageVersion) -> bool {
        match (self, version) {
            (PackageVersionReq::SemVer(req), PackageVersion::SemVer(ver)) => {
                req.matches(&ver.version)
            }
            (PackageVersionReq::DevVer(req), PackageVersion::DevVer(ver)) => req == &ver.modrev,
            (PackageVersionReq::StringVer(req), PackageVersion::StringVer(ver)) => {
                req == &ver.modrev
            }
            (PackageVersionReq::Any, _) => true,
            _ => false,
        }
    }

    pub fn is_any(&self) -> bool {
        matches!(self, PackageVersionReq::Any)
    }
}

impl Display for PackageVersionReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackageVersionReq::SemVer(version_req) => {
                let mut str = version_req.to_string();
                if str.starts_with("=") {
                    str = str.replacen("=", "==", 1);
                } else if str.starts_with("^") {
                    str = str.replacen("^", "~>", 1);
                }
                str.fmt(f)
            }
            PackageVersionReq::DevVer(name_req) => write!(f, "=={}", &name_req),
            PackageVersionReq::StringVer(name_req) => write!(f, "=={}", &name_req),
            PackageVersionReq::Any => f.write_str("any"),
        }
    }
}

impl<'de> Deserialize<'de> for PackageVersionReq {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl FromStr for PackageVersionReq {
    type Err = PackageVersionReqError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = correct_version_req_str(text);

        let trimmed = text.trim_start_matches('=').trim_start_matches('@').trim();

        match parse_version_req(&text) {
            Ok(_) => Ok(PackageVersionReq::SemVer(parse_version_req(&text)?)),
            Err(_) => match trimmed {
                "scm" => Ok(PackageVersionReq::DevVer(DevVersion::Scm)),
                "dev" => Ok(PackageVersionReq::DevVer(DevVersion::Dev)),
                ver => Ok(PackageVersionReq::StringVer(ver.to_string())),
            },
        }
    }
}

fn correct_version_req_str(text: &str) -> String {
    text.chars()
        .chunk_by(|t| t.is_alphanumeric() || matches!(t, '-' | '_' | '.'))
        .into_iter()
        .map(|(is_version_str, chars)| (is_version_str, chars.collect::<String>()))
        .map(|(is_version_str, chunk)| {
            if is_version_str && !is_known_dev_version_str(&chunk) {
                let version_str = trim_specrev(&chunk);
                correct_prerelease_version_string(version_str)
            } else {
                chunk
            }
        })
        .collect::<String>()
}

fn trim_specrev(version_str: &str) -> &str {
    if let Some(pos) = version_str.rfind('-') {
        &version_str[..pos]
    } else {
        version_str
    }
}

#[derive(Error, Debug)]
pub enum SpecrevParseError {
    #[error("specrev {specrev} in version {full_version} contains non-numeric characters")]
    InvalidSpecrev {
        specrev: String,
        full_version: String,
    },
    #[error("could not parse specrev in version {0}")]
    InvalidVersion(String),
}

fn split_specrev(version_str: &str) -> Result<(&str, SpecRev), SpecrevParseError> {
    if let Some(pos) = version_str.rfind('-') {
        if let Some(specrev_str) = version_str.get(pos + 1..) {
            if specrev_str.chars().all(|c| c.is_ascii_digit()) {
                let specrev =
                    specrev_str
                        .parse::<u16>()
                        .map_err(|_| SpecrevParseError::InvalidSpecrev {
                            specrev: specrev_str.into(),
                            full_version: version_str.into(),
                        })?;
                Ok((&version_str[..pos], specrev.into()))
            } else {
                Err(SpecrevParseError::InvalidSpecrev {
                    specrev: specrev_str.into(),
                    full_version: version_str.into(),
                })
            }
        } else {
            Err(SpecrevParseError::InvalidVersion(version_str.into()))
        }
    } else {
        // We assume a specrev of 1 if none can be found.
        Ok((version_str, 1.into()))
    }
}

fn is_known_dev_version_str(text: &str) -> bool {
    matches!(text, "dev" | "scm")
}

/// Parses a Version from a string, automatically supplying any missing details (i.e. missing
/// minor/patch sections).
fn parse_version(s: &str) -> Result<Version, Error> {
    let version_str = correct_version_string(s);
    Version::parse(&version_str)
}

/// Transform LuaRocks constraints into constraints that can be parsed by the semver crate.
fn parse_version_req(version_constraints: &str) -> Result<VersionReq, Error> {
    let unescaped = decode_html_entities(version_constraints)
        .to_string()
        .as_str()
        .to_owned();
    let transformed = match unescaped {
        s if s.starts_with("~>") => parse_pessimistic_version_constraint(s)?,
        s if s.starts_with("@") => format!("={}", &s[1..]),
        // The semver crate only understands "= version", unlike luarocks which understands "== version".
        s if s.starts_with("==") => s[1..].to_string(),
        s if s // semver parses no constraint prefix as ^ (equivalent to ~>)
            .find(|c: char| c.is_alphanumeric())
            .is_some_and(|idx| idx == 0) =>
        {
            format!("={}", &s)
        }
        s => s,
    };

    let version_req = VersionReq::parse(&transformed)?;
    Ok(version_req)
}

fn parse_pessimistic_version_constraint(version_constraint: String) -> Result<String, Error> {
    // pessimistic operator
    let min_version_str = &version_constraint[2..].trim();
    let min_version = Version::parse(&correct_version_string(min_version_str))?;

    let max_version = match min_version_str.matches('.').count() {
        0 => Version {
            major: &min_version.major + 1,
            ..min_version.clone()
        },
        1 => Version {
            minor: &min_version.minor + 1,
            ..min_version.clone()
        },
        _ => Version {
            patch: &min_version.patch + 1,
            ..min_version.clone()
        },
    };

    Ok(format!(">= {min_version}, < {max_version}"))
}

/// ┻━┻ ︵╰(°□°╰) Luarocks allows for an arbitrary number of version digits
/// This function attempts to correct a non-semver compliant version string,
/// by swapping the third '.' out with a '-', converting the non-semver
/// compliant digits to a pre-release identifier.
fn correct_version_string(version: &str) -> String {
    let version = append_minor_patch_if_missing(version);
    correct_prerelease_version_string(&version)
}

fn correct_prerelease_version_string(version: &str) -> String {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() > 3 {
        let corrected_version = format!(
            "{}.{}.{}-{}",
            parts[0],
            parts[1],
            parts[2],
            parts[3..].join(".")
        );
        corrected_version
    } else {
        version.to_string()
    }
}

/// Recursively append .0 until the version string has a minor or patch version
fn append_minor_patch_if_missing(version: &str) -> String {
    if version.matches('.').count() < 2 {
        append_minor_patch_if_missing(&format!("{version}.0"))
    } else {
        version.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parse_semver_version() {
        assert_eq!(
            PackageVersion::parse("1-1").unwrap(),
            PackageVersion::SemVer(SemVer {
                version: "1.0.0".parse().unwrap(),
                component_count: 1,
                specrev: 1.into(),
            })
        );
        assert_eq!(
            PackageVersion::parse("1.0-1").unwrap(),
            PackageVersion::SemVer(SemVer {
                version: "1.0.0".parse().unwrap(),
                component_count: 2,
                specrev: 1.into(),
            })
        );
        assert_eq!(
            PackageVersion::parse("1.0.0-1").unwrap(),
            PackageVersion::SemVer(SemVer {
                version: "1.0.0".parse().unwrap(),
                component_count: 3,
                specrev: 1.into()
            })
        );
        assert_eq!(
            PackageVersion::parse("1.0.0-1").unwrap(),
            PackageVersion::SemVer(SemVer {
                version: "1.0.0".parse().unwrap(),
                component_count: 3,
                specrev: 1.into()
            })
        );
        assert_eq!(
            PackageVersion::parse("1.0.0-10-1").unwrap(),
            PackageVersion::SemVer(SemVer {
                version: "1.0.0-10".parse().unwrap(),
                component_count: 3,
                specrev: 1.into()
            })
        );
        assert_eq!(
            PackageVersion::parse("1.0.0.10-1").unwrap(),
            PackageVersion::SemVer(SemVer {
                version: "1.0.0-10".parse().unwrap(),
                component_count: 3,
                specrev: 1.into()
            })
        );
        assert_eq!(
            PackageVersion::parse("1.0.0.10.0-1").unwrap(),
            PackageVersion::SemVer(SemVer {
                version: "1.0.0-10.0".parse().unwrap(),
                component_count: 3,
                specrev: 1.into()
            })
        );
    }

    #[tokio::test]
    async fn parse_dev_version() {
        assert_eq!(
            PackageVersion::parse("dev-1").unwrap(),
            PackageVersion::DevVer(DevVer {
                modrev: DevVersion::Dev,
                specrev: 1.into()
            })
        );
        assert_eq!(
            PackageVersion::parse("scm-1").unwrap(),
            PackageVersion::DevVer(DevVer {
                modrev: DevVersion::Scm,
                specrev: 1.into()
            })
        );
        assert_eq!(
            PackageVersion::parse("git-1").unwrap(),
            PackageVersion::StringVer(StringVer {
                modrev: "git".into(),
                specrev: 1.into()
            })
        );
        assert_eq!(
            PackageVersion::parse("scm-1").unwrap(),
            PackageVersion::DevVer(DevVer {
                modrev: DevVersion::Scm,
                specrev: 1.into()
            })
        );
    }

    #[tokio::test]
    async fn parse_dev_version_req() {
        assert_eq!(
            PackageVersionReq::parse("dev").unwrap(),
            PackageVersionReq::DevVer(DevVersion::Dev)
        );
        assert_eq!(
            PackageVersionReq::parse("scm").unwrap(),
            PackageVersionReq::DevVer(DevVersion::Scm)
        );
        assert_eq!(
            PackageVersionReq::parse("git").unwrap(),
            PackageVersionReq::StringVer("git".into())
        );
        assert_eq!(
            PackageVersionReq::parse("==dev").unwrap(),
            PackageVersionReq::DevVer(DevVersion::Dev)
        );
        assert_eq!(
            PackageVersionReq::parse("==git").unwrap(),
            PackageVersionReq::StringVer("git".into())
        );
        assert_eq!(
            PackageVersionReq::parse("== dev").unwrap(),
            PackageVersionReq::DevVer(DevVersion::Dev)
        );
        assert_eq!(
            PackageVersionReq::parse("== scm").unwrap(),
            PackageVersionReq::DevVer(DevVersion::Scm)
        );
        assert_eq!(
            PackageVersionReq::parse("@dev").unwrap(),
            PackageVersionReq::DevVer(DevVersion::Dev)
        );
        assert_eq!(
            PackageVersionReq::parse("@git").unwrap(),
            PackageVersionReq::StringVer("git".into())
        );
        assert_eq!(
            PackageVersionReq::parse("@ dev").unwrap(),
            PackageVersionReq::DevVer(DevVersion::Dev)
        );
        assert_eq!(
            PackageVersionReq::parse("@ scm").unwrap(),
            PackageVersionReq::DevVer(DevVersion::Scm)
        );
        assert_eq!(
            PackageVersionReq::parse(">1-1,<1.2-2").unwrap(),
            PackageVersionReq::SemVer(">1,<1.2".parse().unwrap())
        );
        assert_eq!(
            PackageVersionReq::parse("> 1-1, < 1.2-2").unwrap(),
            PackageVersionReq::SemVer("> 1, < 1.2".parse().unwrap())
        );
        assert_eq!(
            PackageVersionReq::parse("> 2.1.0.10, < 2.1.1").unwrap(),
            PackageVersionReq::SemVer("> 2.1.0-10, < 2.1.1".parse().unwrap())
        );
    }

    #[tokio::test]
    async fn package_version_req_semver_roundtrips() {
        let req = PackageVersionReq::parse("==0.7.1").unwrap();
        assert_eq!(req.to_string(), "==0.7.1");

        let req = PackageVersionReq::parse("0.7.1").unwrap();
        assert_eq!(req.to_string(), "==0.7.1");

        let req = PackageVersionReq::parse(">=0.7.1").unwrap();
        assert_eq!(req.to_string(), ">=0.7.1");

        let req = PackageVersionReq::parse(">0.7.1").unwrap();
        assert_eq!(req.to_string(), ">0.7.1");

        let req = PackageVersionReq::parse("<0.7.1").unwrap();
        assert_eq!(req.to_string(), "<0.7.1");

        let req = PackageVersionReq::parse("~> 0.7.1").unwrap();
        assert_eq!(req.to_string(), ">=0.7.1, <0.7.2");
    }

    #[tokio::test]
    async fn package_version_req_devver_roundtrips() {
        let req = PackageVersionReq::parse("==scm").unwrap();
        assert_eq!(req.to_string(), "==scm");

        let req = PackageVersionReq::parse("@scm").unwrap();
        assert_eq!(req.to_string(), "==scm");

        let req = PackageVersionReq::parse("scm").unwrap();
        assert_eq!(req.to_string(), "==scm");

        let req = PackageVersionReq::parse("==a144124839f027a2d0a95791936c478d047126fc").unwrap();
        assert_eq!(
            req.to_string(),
            "==a144124839f027a2d0a95791936c478d047126fc"
        );
    }
}
