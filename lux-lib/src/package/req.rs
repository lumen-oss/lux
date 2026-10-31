use std::{fmt::Display, str::FromStr};

use itertools::Itertools;
use miette::Diagnostic;
use serde::{de, Deserialize, Deserializer};
use thiserror::Error;

use super::spec::Spec;
use super::version::ReqError;
use super::{Name, VersionReq};

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

#[derive(Error, Debug, Diagnostic)]
pub enum Error {
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
    type Err = Error;

    fn from_str(str: &str) -> Result<Self, Error> {
        let rock_name_str = str
            .chars()
            .peeking_take_while(|t| t.is_alphanumeric() || matches!(t, '-' | '_' | '.'))
            .collect::<String>();

        if rock_name_str.is_empty() {
            return Err(Error::InvalidDependencyName(str.to_string()));
        }

        let constraints = str.trim_start_matches(&rock_name_str).trim();
        let version_req = match constraints {
            "" => VersionReq::any(),
            constraints => VersionReq::parse(constraints.trim_start()).map_err(|error| {
                Error::InvalidPackageVersionReq {
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

    pub fn parse(pkg_constraints: &str) -> Result<Self, Error> {
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
