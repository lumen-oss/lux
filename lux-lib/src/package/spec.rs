use miette::Diagnostic;
use serde::{Deserialize, Deserializer};
use thiserror::Error;

use super::version::{self, FromReqError, HasModRev, Version};
use super::{Name, Req};
use crate::variables::{GetVariableError, HasVariables};

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("invalid version for rock {rock}: {err}")]
    Version { rock: Name, err: FromReqError },
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
    pub fn parse(name: String, version: String) -> Result<Self, version::Error> {
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
    type Error = Error;

    fn try_from(value: Req) -> Result<Self, Self::Error> {
        let name = value.name;
        let version = value.version_req.try_into().map_err(|err| Error::Version {
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
