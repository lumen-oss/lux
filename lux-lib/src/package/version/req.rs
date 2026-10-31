use std::{cmp::Ordering, fmt::Display, str::FromStr};

use html_escape::decode_html_entities;
use itertools::Itertools;
use miette::Diagnostic;
use semver::{Comparator, Error as SemverError, Op};
use serde::{Deserialize, Deserializer};
use thiserror::Error;

use super::{
    cmp_prerelease, correct_prerelease_version_string, correct_version_string, DevVersion, Version,
};

#[derive(Debug, Error, Diagnostic)]
pub enum FromReqError {
    #[error("cannot parse version from non-exact version requirement '{0}'")]
    NonExactVersionReq(semver::VersionReq),
    #[error("cannot parse version from version requirement '*' (any version)")]
    Any,
}

/// Based on `semver`'s matching, but with the prerelease compared as an extension
/// for compatibility with LuaRocks.
/// The "prereleases only match prerelease comparators" rule is not applied.
fn comparator_matches(comparator: &Comparator, version: &semver::Version) -> bool {
    match comparator.op {
        Op::Exact | Op::Wildcard => matches_exact(comparator, version),
        Op::Greater => matches_greater(comparator, version),
        Op::GreaterEq => matches_greater(comparator, version) || matches_exact(comparator, version),
        Op::Less => matches_less(comparator, version),
        Op::LessEq => matches_less(comparator, version) || matches_exact(comparator, version),
        Op::Tilde => matches_tilde(comparator, version),
        Op::Caret => matches_caret(comparator, version),
        _ => false,
    }
}

fn matches_exact(comparator: &Comparator, version: &semver::Version) -> bool {
    if version.major != comparator.major {
        return false;
    }
    if let Some(minor) = comparator.minor {
        if version.minor != minor {
            return false;
        }
    }
    if let Some(patch) = comparator.patch {
        if version.patch != patch {
            return false;
        }
    }
    version.pre == comparator.pre
}

fn matches_greater(comparator: &Comparator, version: &semver::Version) -> bool {
    if version.major != comparator.major {
        return version.major > comparator.major;
    }
    match comparator.minor {
        None => return false,
        Some(minor) => {
            if version.minor != minor {
                return version.minor > minor;
            }
        }
    }
    match comparator.patch {
        None => return false,
        Some(patch) => {
            if version.patch != patch {
                return version.patch > patch;
            }
        }
    }
    cmp_prerelease(&version.pre, &comparator.pre) == Ordering::Greater
}

fn matches_less(comparator: &Comparator, version: &semver::Version) -> bool {
    if version.major != comparator.major {
        return version.major < comparator.major;
    }
    match comparator.minor {
        None => return false,
        Some(minor) => {
            if version.minor != minor {
                return version.minor < minor;
            }
        }
    }
    match comparator.patch {
        None => return false,
        Some(patch) => {
            if version.patch != patch {
                return version.patch < patch;
            }
        }
    }
    cmp_prerelease(&version.pre, &comparator.pre) == Ordering::Less
}

fn matches_tilde(comparator: &Comparator, version: &semver::Version) -> bool {
    if version.major != comparator.major {
        return false;
    }
    if let Some(minor) = comparator.minor {
        if version.minor != minor {
            return false;
        }
    }
    if let Some(patch) = comparator.patch {
        if version.patch != patch {
            return version.patch > patch;
        }
    }
    cmp_prerelease(&version.pre, &comparator.pre) != Ordering::Less
}

fn matches_caret(comparator: &Comparator, version: &semver::Version) -> bool {
    if version.major != comparator.major {
        return false;
    }
    let minor = match comparator.minor {
        None => return true,
        Some(minor) => minor,
    };
    let patch = match comparator.patch {
        None => {
            if comparator.major > 0 {
                return version.minor >= minor;
            } else {
                return version.minor == minor;
            }
        }
        Some(patch) => patch,
    };
    if comparator.major > 0 {
        if version.minor != minor {
            return version.minor > minor;
        } else if version.patch != patch {
            return version.patch > patch;
        }
    } else if minor > 0 {
        if version.minor != minor {
            return false;
        } else if version.patch != patch {
            return version.patch > patch;
        }
    } else if version.minor != minor || version.patch != patch {
        return false;
    }
    cmp_prerelease(&version.pre, &comparator.pre) != Ordering::Less
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
#[error(transparent)]
pub struct ReqError(#[from] SemverError);

/// **SemVer version** requirement as defined by <https://semver.org>.
/// or a **Dev** version requirement, which can be one of "dev", "scm", or "git"
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum VersionReq {
    /// A VersionReq that matches a SemVer version.
    SemVer(semver::VersionReq),
    /// A VersionReq that matches only known dev versions.
    DevVer(DevVersion),
    /// A VersionReq that matches a arbitrary string version.
    StringVer(String),
    /// A VersionReq that has no version constraint.
    Any,
}

impl VersionReq {
    /// Returns a `VersionReq` that matches any version.
    pub fn any() -> Self {
        VersionReq::Any
    }

    pub fn parse(text: &str) -> Result<Self, ReqError> {
        VersionReq::from_str(text)
    }

    pub fn matches(&self, version: &Version) -> bool {
        match (self, version) {
            (VersionReq::SemVer(req), Version::SemVer(ver)) => req
                .comparators
                .iter()
                .all(|comparator| comparator_matches(comparator, &ver.version)),
            (VersionReq::DevVer(req), Version::DevVer(ver)) => req == &ver.modrev,
            (VersionReq::StringVer(req), Version::StringVer(ver)) => req == &ver.modrev,
            (VersionReq::Any, _) => true,
            _ => false,
        }
    }

    pub fn is_any(&self) -> bool {
        matches!(self, VersionReq::Any)
    }

    /// Returns true if this requirement pins the Luau dialect (`lua = "luau"`).
    pub fn is_luau(&self) -> bool {
        matches!(self, VersionReq::StringVer(req) if req == "luau")
    }
}

impl Display for VersionReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VersionReq::SemVer(version_req) => {
                let mut str = version_req.to_string();
                if str.starts_with("=") {
                    str = str.replacen("=", "==", 1);
                } else if str.starts_with("^") {
                    str = str.replacen("^", "~>", 1);
                }
                str.fmt(f)
            }
            VersionReq::DevVer(name_req) => write!(f, "=={}", name_req),
            VersionReq::StringVer(name_req) => write!(f, "=={}", name_req),
            VersionReq::Any => f.write_str("any"),
        }
    }
}

impl<'de> Deserialize<'de> for VersionReq {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl FromStr for VersionReq {
    type Err = ReqError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = correct_version_req_str(text);

        let trimmed = text.trim_start_matches('=').trim_start_matches('@').trim();

        match parse_version_req(&text) {
            Ok(ver) if ver.comparators.is_empty() => Ok(VersionReq::Any),
            Ok(ver) => Ok(VersionReq::SemVer(ver)),
            Err(_) => match trimmed {
                "scm" => Ok(VersionReq::DevVer(DevVersion::Scm)),
                "dev" => Ok(VersionReq::DevVer(DevVersion::Dev)),
                ver => Ok(VersionReq::StringVer(ver.to_string())),
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
        let suffix = &version_str[pos + 1..];
        if suffix.chars().all(|c| c.is_ascii_digit()) {
            &version_str[..pos]
        } else {
            version_str
        }
    } else {
        version_str
    }
}

fn is_known_dev_version_str(text: &str) -> bool {
    matches!(text, "dev" | "scm")
}

/// Transform LuaRocks constraints into constraints that can be parsed by the semver crate.
fn parse_version_req(version_constraints: &str) -> Result<semver::VersionReq, SemverError> {
    let unescaped = decode_html_entities(version_constraints)
        .to_string()
        .as_str()
        .to_owned();
    let transformed = match unescaped {
        s if s.starts_with("~>") => parse_pessimistic_version_constraint(s)?,
        s if s.starts_with("@") => format!("={}", &s[1..]),
        // The semver crate only understands "= version", unlike luarocks which understands "== version".
        s if s.starts_with("==") => s[1..].to_string(),
        s if s // semver parses no constraint prefix as ^, unlike luarocks which reads it as "=="
            .find(|c: char| c.is_alphanumeric())
            .is_some_and(|idx| idx == 0) =>
        {
            format!("={}", s)
        }
        s => s,
    };

    let version_req = semver::VersionReq::parse(&transformed)?;
    Ok(version_req)
}

fn parse_pessimistic_version_constraint(version_constraint: String) -> Result<String, SemverError> {
    // pessimistic operator
    let min_version_str = &version_constraint[2..].trim();
    let min_version = semver::Version::parse(&correct_version_string(min_version_str))?;

    let max_version = match min_version_str.matches('.').count() {
        0 => semver::Version {
            major: &min_version.major + 1,
            ..min_version.clone()
        },
        1 => semver::Version {
            minor: &min_version.minor + 1,
            ..min_version.clone()
        },
        _ => semver::Version {
            patch: &min_version.patch + 1,
            ..min_version.clone()
        },
    };

    Ok(format!(">= {min_version}, < {max_version}"))
}
