use std::{
    cmp::{self, Ordering},
    fmt::Display,
    str::FromStr,
};

use itertools::Itertools;
use miette::Diagnostic;
use nonempty::NonEmpty;
use semver::{Comparator, Error as SemverError, Op, Prerelease};
use serde::{de, Deserialize, Deserializer, Serialize};
use thiserror::Error;

mod req;
mod spec_rev;
mod upgrade;

use spec_rev::split_specrev;

pub use req::{FromReqError, ReqError, VersionReq};
pub(crate) use spec_rev::SpecRevIterator;
pub use spec_rev::{SpecRev, SpecRevParseError};

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum Version {
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

impl HasModRev for Version {
    fn to_modrev_string(&self) -> String {
        match self {
            Self::SemVer(ver) => ver.to_modrev_string(),
            Self::DevVer(ver) => ver.to_modrev_string(),
            Self::StringVer(ver) => ver.to_modrev_string(),
        }
    }
}

impl Version {
    pub fn parse(text: &str) -> Result<Self, Error> {
        Version::from_str(text)
    }
    /// The package version without its spec revision.
    pub fn to_version_string(&self) -> String {
        self.to_modrev_string()
    }
    /// Note that this loses the specrev information.
    pub fn into_version_req(&self) -> VersionReq {
        match self {
            Version::DevVer(DevVer { modrev, .. }) => VersionReq::DevVer(modrev.to_owned()),
            Version::StringVer(StringVer { modrev, .. }) => {
                VersionReq::StringVer(modrev.to_owned())
            }
            Version::SemVer(SemVer { version, .. }) => {
                let version = version.to_owned();
                VersionReq::SemVer(semver::VersionReq {
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

    /// A `~>` version requirement that allows non-breaking updates, or `None` if this is
    /// not a SemVer version.
    /// Following the SemVer conventions around zero versions, the most significant non-zero
    /// component is kept, so that only the components to its right may change.
    pub(crate) fn to_pessimistic_version_req(&self) -> Option<String> {
        let Version::SemVer(sem_ver) = self else {
            return None;
        };
        let version = &sem_ver.version;
        Some(match (version.major, version.minor) {
            (0, 0) => format!("~> 0.0.{}", version.patch),
            (0, minor) => format!("~> 0.{minor}"),
            (major, _) => format!("~> {major}"),
        })
    }

    pub(crate) fn is_semver(&self) -> bool {
        matches!(self, Version::SemVer(_))
    }

    pub(crate) fn default_dev_version() -> Self {
        Self::DevVer(DevVer::default())
    }

    pub(crate) fn default_dev_version_with_specrev(specrev: SpecRev) -> Self {
        Self::DevVer(DevVer::default_with_specrev(specrev))
    }

    pub(crate) fn with_specrev(mut self, specrev: SpecRev) -> Self {
        match &mut self {
            Self::SemVer(version) => version.specrev = specrev,
            Self::DevVer(version) => version.specrev = specrev,
            Self::StringVer(version) => version.specrev = specrev,
        }
        self
    }
}

impl TryFrom<VersionReq> for Version {
    type Error = FromReqError;

    fn try_from(req: VersionReq) -> Result<Self, Self::Error> {
        match req {
            VersionReq::SemVer(version_req) => {
                match NonEmpty::try_from(version_req.comparators.clone()) {
                    Ok(comparators)
                        if comparators
                            .iter()
                            .any(|comparator| comparator.op != semver::Op::Exact) =>
                    {
                        Err(FromReqError::NonExactVersionReq(version_req))
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
                        Ok(Version::SemVer(SemVer {
                            version,
                            component_count: comparator_component_count(comparator),
                            specrev: 1.into(),
                        }))
                    }
                    Err(_) => Err(FromReqError::NonExactVersionReq(version_req)),
                }
            }
            VersionReq::DevVer(modrev) => Ok(Version::DevVer(DevVer {
                modrev,
                specrev: 1.into(),
            })),
            VersionReq::StringVer(modrev) => Ok(Version::StringVer(StringVer {
                modrev,
                specrev: 1.into(),
            })),
            VersionReq::Any => Err(FromReqError::Any),
        }
    }
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Specrev(#[from] SpecRevParseError),
    #[error("failed to parse version")]
    Version(#[from] SemverError),
}

impl Serialize for Version {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Version::SemVer(version) => version.serialize(serializer),
            Version::DevVer(version) => version.serialize(serializer),
            Version::StringVer(version) => version.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).map_err(de::Error::custom)
    }
}

impl Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Version::SemVer(version) => version.fmt(f),
            Version::DevVer(version) => version.fmt(f),
            Version::StringVer(version) => version.fmt(f),
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Version::SemVer(a), Version::SemVer(b)) => a.cmp(b),
            (Version::SemVer(..), Version::DevVer(..)) => Ordering::Less,
            (Version::SemVer(..), Version::StringVer(..)) => Ordering::Greater,
            (Version::DevVer(..), Version::SemVer(..)) => Ordering::Greater,
            (Version::DevVer(a), Version::DevVer(b)) => a.cmp(b),
            (Version::DevVer(..), Version::StringVer(..)) => Ordering::Greater,
            (Version::StringVer(a), Version::StringVer(b)) => a.cmp(b),
            (Version::StringVer(..), Version::SemVer(..)) => Ordering::Less,
            (Version::StringVer(..), Version::DevVer(..)) => Ordering::Less,
        }
    }
}

impl FromStr for Version {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (modrev, specrev) = split_specrev(text)?;
        match modrev {
            "scm" => Ok(Version::DevVer(DevVer {
                modrev: DevVersion::Scm,
                specrev,
            })),
            "dev" => Ok(Version::DevVer(DevVer {
                modrev: DevVersion::Dev,
                specrev,
            })),
            modrev => match parse_version(modrev) {
                Ok(version) => Ok(Version::SemVer(SemVer {
                    component_count: cmp::min(text.chars().filter(|c| *c == '.').count() + 1, 3),
                    version,
                    specrev,
                })),
                Err(_) => Ok(Version::StringVer(StringVer {
                    modrev: modrev.into(),
                    specrev,
                })),
            },
        }
    }
}

// TODO: Stop deriving Eq here

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct SemVer {
    pub(crate) version: semver::Version,
    pub(crate) component_count: usize,
    pub(crate) specrev: SpecRev,
}

impl HasModRev for SemVer {
    fn to_modrev_string(&self) -> String {
        self.version.to_string()
    }
}

impl Display for SemVer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self
            .version
            .pre
            .as_str()
            .chars()
            .any(|c| c.is_ascii_alphabetic())
        {
            // pre-release instead of specrev
            return self.version.fmt(f);
        }
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
        match cmp_versions(&self.version, &other.version) {
            Ordering::Equal => self.specrev.cmp(&other.specrev),
            ord => ord,
        }
    }
}

/// Compare two LuaRocks versions.
/// ┻━┻ ︵╰(°□°╰) Luarocks allows for an arbitrary number of version digits (e.g. `2.1.0.10`),
/// which Lux treats as a SemVer prerelease.
fn cmp_versions(a: &semver::Version, b: &semver::Version) -> Ordering {
    match (a.major, a.minor, a.patch).cmp(&(b.major, b.minor, b.patch)) {
        Ordering::Equal => cmp_prerelease(&a.pre, &b.pre),
        ord => ord,
    }
}

pub(super) fn comparator_component_count(comparator: &Comparator) -> usize {
    if comparator.patch.is_some() {
        3
    } else if comparator.minor.is_some() {
        2
    } else {
        1
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

#[derive(Clone, Eq, PartialEq, Hash, Debug, Default)]
pub struct DevVer {
    pub(crate) modrev: DevVersion,
    pub(crate) specrev: SpecRev,
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
    pub(crate) modrev: String,
    pub(crate) specrev: SpecRev,
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

/// Parses a Version from a string, automatically supplying any missing details (i.e. missing
/// minor/patch sections).
fn parse_version(s: &str) -> Result<semver::Version, SemverError> {
    let version_str = correct_version_string(s);
    semver::Version::parse(&version_str)
}

fn cmp_prerelease(a: &Prerelease, b: &Prerelease) -> Ordering {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => Ordering::Equal,
        (false, true) => Ordering::Greater,
        (true, false) => Ordering::Less,
        (false, false) => a.cmp(b),
    }
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
    if version.contains('-') {
        return version.to_string();
    }
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() > 3 {
        format!(
            "{}.{}.{}-{}",
            parts[0],
            parts[1],
            parts[2],
            parts[3..].join(".")
        )
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

    #[test]
    fn parse_semver_version() {
        assert_eq!(
            Version::parse("1-1").unwrap(),
            Version::SemVer(SemVer {
                version: "1.0.0".parse().unwrap(),
                component_count: 1,
                specrev: 1.into(),
            })
        );
        assert_eq!(
            Version::parse("1.0-1").unwrap(),
            Version::SemVer(SemVer {
                version: "1.0.0".parse().unwrap(),
                component_count: 2,
                specrev: 1.into(),
            })
        );
        assert_eq!(
            Version::parse("1.0.0-1").unwrap(),
            Version::SemVer(SemVer {
                version: "1.0.0".parse().unwrap(),
                component_count: 3,
                specrev: 1.into()
            })
        );
        assert_eq!(
            Version::parse("1.0.0-1").unwrap(),
            Version::SemVer(SemVer {
                version: "1.0.0".parse().unwrap(),
                component_count: 3,
                specrev: 1.into()
            })
        );
        assert_eq!(
            Version::parse("1.0.0-10-1").unwrap(),
            Version::SemVer(SemVer {
                version: "1.0.0-10".parse().unwrap(),
                component_count: 3,
                specrev: 1.into()
            })
        );
        assert_eq!(
            Version::parse("1.0.0.10-1").unwrap(),
            Version::SemVer(SemVer {
                version: "1.0.0-10".parse().unwrap(),
                component_count: 3,
                specrev: 1.into()
            })
        );
        assert_eq!(
            Version::parse("1.0.0.10.0-1").unwrap(),
            Version::SemVer(SemVer {
                version: "1.0.0-10.0".parse().unwrap(),
                component_count: 3,
                specrev: 1.into()
            })
        );
    }

    #[test]
    fn to_pessimistic_version_req() {
        for (version, req, expected) in [
            ("1.5.6-2", "~> 1", ">=1.0.0, <2.0.0"),
            ("0.5.6-2", "~> 0.5", ">=0.5.0, <0.6.0"),
            ("0.0.6-2", "~> 0.0.6", ">=0.0.6, <0.0.7"),
        ] {
            let pessimistic = Version::parse(version)
                .unwrap()
                .to_pessimistic_version_req()
                .unwrap();
            assert_eq!(pessimistic, req);
            let req = VersionReq::parse(&pessimistic).unwrap();
            assert_eq!(req.to_string(), expected);
        }

        let req = VersionReq::parse("~> 1").unwrap();
        assert!(req.matches(&Version::parse("1.9.0-2").unwrap()));
        assert!(!req.matches(&Version::parse("2.0.0-1").unwrap()));

        let req = VersionReq::parse("~> 0.5").unwrap();
        assert!(req.matches(&Version::parse("0.5.9-1").unwrap()));
        assert!(!req.matches(&Version::parse("0.6.0-1").unwrap()));

        assert_eq!(
            Version::parse("scm").unwrap().to_pessimistic_version_req(),
            None
        );
    }

    #[test]
    fn four_component_versions_sort_after_base_version() {
        let base = Version::parse("2.1.0-1").unwrap();
        let extended = Version::parse("2.1.0.10-1").unwrap();
        assert!(extended > base);
        assert!(base < extended);
        assert_eq!(
            Version::parse("2.1.0.10-1").unwrap(),
            Version::parse("2.1.0.10-1").unwrap()
        );
    }

    #[test]
    fn four_component_version_satisfies_greater_eq_req() {
        let req = VersionReq::parse(">= 2.1.0").unwrap();
        assert!(req.matches(&Version::parse("2.1.0.10-1").unwrap()));
        assert!(req.matches(&Version::parse("2.1.0-1").unwrap()));
        assert!(!req.matches(&Version::parse("2.0.9-1").unwrap()));
    }

    #[test]
    fn parse_semver_prerelease() {
        let prerelease = Version::parse("1.2.3-alpha.1").unwrap();
        assert_eq!(
            prerelease,
            Version::SemVer(SemVer {
                version: "1.2.3-alpha.1".parse().unwrap(),
                component_count: 3,
                specrev: 1.into()
            })
        );
        assert_eq!(prerelease.to_string(), "1.2.3-alpha.1");
        assert_eq!(Version::parse("1.2.3-1").unwrap().to_string(), "1.2.3-1");
        assert_eq!(Version::parse(&prerelease.to_string()).unwrap(), prerelease);
    }

    #[test]
    fn prerelease_req_matches() {
        let req = VersionReq::parse("=1.2.3-alpha.1").unwrap();
        assert!(req.matches(&Version::parse("1.2.3-alpha.1").unwrap()));
        assert!(!req.matches(&Version::parse("1.2.3-alpha.2").unwrap()));
    }

    #[test]
    fn parse_dev_version() {
        assert_eq!(
            Version::parse("dev-1").unwrap(),
            Version::DevVer(DevVer {
                modrev: DevVersion::Dev,
                specrev: 1.into()
            })
        );
        assert_eq!(
            Version::parse("scm-1").unwrap(),
            Version::DevVer(DevVer {
                modrev: DevVersion::Scm,
                specrev: 1.into()
            })
        );
        assert_eq!(
            Version::parse("git-1").unwrap(),
            Version::StringVer(StringVer {
                modrev: "git".into(),
                specrev: 1.into()
            })
        );
        assert_eq!(
            Version::parse("scm-1").unwrap(),
            Version::DevVer(DevVer {
                modrev: DevVersion::Scm,
                specrev: 1.into()
            })
        );
    }

    #[test]
    fn parse_dev_version_req() {
        assert_eq!(
            VersionReq::parse("dev").unwrap(),
            VersionReq::DevVer(DevVersion::Dev)
        );
        assert_eq!(
            VersionReq::parse("scm").unwrap(),
            VersionReq::DevVer(DevVersion::Scm)
        );
        assert_eq!(
            VersionReq::parse("git").unwrap(),
            VersionReq::StringVer("git".into())
        );
        assert_eq!(
            VersionReq::parse("==dev").unwrap(),
            VersionReq::DevVer(DevVersion::Dev)
        );
        assert_eq!(
            VersionReq::parse("==git").unwrap(),
            VersionReq::StringVer("git".into())
        );
        assert_eq!(
            VersionReq::parse("== dev").unwrap(),
            VersionReq::DevVer(DevVersion::Dev)
        );
        assert_eq!(
            VersionReq::parse("== scm").unwrap(),
            VersionReq::DevVer(DevVersion::Scm)
        );
        assert_eq!(
            VersionReq::parse("@dev").unwrap(),
            VersionReq::DevVer(DevVersion::Dev)
        );
        assert_eq!(
            VersionReq::parse("@git").unwrap(),
            VersionReq::StringVer("git".into())
        );
        assert_eq!(
            VersionReq::parse("@ dev").unwrap(),
            VersionReq::DevVer(DevVersion::Dev)
        );
        assert_eq!(
            VersionReq::parse("@ scm").unwrap(),
            VersionReq::DevVer(DevVersion::Scm)
        );
        assert_eq!(
            VersionReq::parse(">1-1,<1.2-2").unwrap(),
            VersionReq::SemVer(">1,<1.2".parse().unwrap())
        );
        assert_eq!(
            VersionReq::parse("> 1-1, < 1.2-2").unwrap(),
            VersionReq::SemVer("> 1, < 1.2".parse().unwrap())
        );
        assert_eq!(
            VersionReq::parse("> 2.1.0.10, < 2.1.1").unwrap(),
            VersionReq::SemVer("> 2.1.0-10, < 2.1.1".parse().unwrap())
        );
    }

    #[test]
    fn parse_any_version_req() {
        assert_eq!(VersionReq::parse("*").unwrap(), VersionReq::Any);
    }

    #[test]
    fn package_version_req_semver_roundtrips() {
        let req = VersionReq::parse("==0.7.1").unwrap();
        assert_eq!(req.to_string(), "==0.7.1");

        let req = VersionReq::parse("0.7.1").unwrap();
        assert_eq!(req.to_string(), "==0.7.1");

        let req = VersionReq::parse(">=0.7.1").unwrap();
        assert_eq!(req.to_string(), ">=0.7.1");

        let req = VersionReq::parse(">0.7.1").unwrap();
        assert_eq!(req.to_string(), ">0.7.1");

        let req = VersionReq::parse("<0.7.1").unwrap();
        assert_eq!(req.to_string(), "<0.7.1");

        let req = VersionReq::parse("~> 0.7.1").unwrap();
        assert_eq!(req.to_string(), ">=0.7.1, <0.7.2");
    }

    #[test]
    fn package_version_req_devver_roundtrips() {
        let req = VersionReq::parse("==scm").unwrap();
        assert_eq!(req.to_string(), "==scm");

        let req = VersionReq::parse("@scm").unwrap();
        assert_eq!(req.to_string(), "==scm");

        let req = VersionReq::parse("scm").unwrap();
        assert_eq!(req.to_string(), "==scm");

        let req = VersionReq::parse("==a144124839f027a2d0a95791936c478d047126fc").unwrap();
        assert_eq!(
            req.to_string(),
            "==a144124839f027a2d0a95791936c478d047126fc"
        );
    }
}
