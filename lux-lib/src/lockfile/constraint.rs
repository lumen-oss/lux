use miette::Diagnostic;
use serde::Deserialize;
use thiserror::Error;

use crate::package;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum Constraint {
    #[default]
    Unconstrained,
    Constrained(package::VersionReq),
}

impl<'de> Deserialize<'de> for Constraint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let str = String::deserialize(deserializer)?;
        match str.as_str() {
            "*" => Ok(Constraint::Unconstrained),
            _ => Ok(Constraint::Constrained(
                str.parse().map_err(serde::de::Error::custom)?,
            )),
        }
    }
}

impl Constraint {
    pub fn to_string_opt(&self) -> Option<String> {
        match self {
            Constraint::Unconstrained => None,
            Constraint::Constrained(req) => Some(req.to_string()),
        }
    }

    pub(crate) fn matches_version_req(&self, req: &package::VersionReq) -> bool {
        match self {
            Constraint::Unconstrained => req.is_any(),
            Constraint::Constrained(package_version_req) => package_version_req == req,
        }
    }
}

impl From<package::VersionReq> for Constraint {
    fn from(value: package::VersionReq) -> Self {
        if value.is_any() {
            Self::Unconstrained
        } else {
            Self::Constrained(value)
        }
    }
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("invalid constraint in LuaPackage")]
    #[diagnostic(forward(0))]
    VersionReq(#[from] package::version::ReqError),
}

impl TryFrom<&Option<String>> for Constraint {
    type Error = Error;

    fn try_from(constraint: &Option<String>) -> Result<Self, Self::Error> {
        match constraint {
            Some(constraint) => {
                let package_version_req = constraint.parse()?;
                Ok(Constraint::Constrained(package_version_req))
            }
            None => Ok(Constraint::Unconstrained),
        }
    }
}
