use std::{
    fmt::{self, Display},
    str::FromStr,
};

use semver::{Version, VersionReq};
use serde::{de::Error as _, ser::Serializer, Deserialize, Deserializer, Serialize};
use thiserror::Error;

use super::name;

/// A requirement on a package: a name plus a SemVer range.
///
/// A bare version defaults to the `^` ("compatible") requirement.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Req {
    name: name::Name,
    version_req: VersionReq,
}

impl Req {
    pub(crate) fn new(name: name::Name, version_req: VersionReq) -> Self {
        Self { name, version_req }
    }

    pub(crate) fn name(&self) -> &name::Name {
        &self.name
    }

    pub(crate) fn version_req(&self) -> &VersionReq {
        &self.version_req
    }

    pub(crate) fn matches(&self, name: &name::Name, version: &Version) -> bool {
        self.name == *name && self.version_req.matches(version)
    }
}

impl Display for Req {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.name, self.version_req)
    }
}

impl FromStr for Req {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (name, req) = s.split_once('@').ok_or(Error::InvalidFormat)?;
        if req.is_empty() || req.chars().all(char::is_whitespace) {
            return Err(Error::InvalidFormat);
        }
        let name: name::Name = name.parse()?;
        let version_req = if Version::parse(req).is_ok() {
            VersionReq::parse(&format!("^{req}"))?
        } else {
            VersionReq::parse(req)?
        };
        Ok(Self::new(name, version_req))
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("wally package requirements must be of the form SCOPE/NAME@VERSION_REQ")]
    InvalidFormat,
    #[error(transparent)]
    Name(#[from] name::Error),
    #[error(transparent)]
    Version(#[from] semver::Error),
}

impl Serialize for Req {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Req {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn bare_version_defaults_to_caret() {
        let req: Req = "sleitnick/signal@2.0.0".parse().unwrap();
        assert_eq!(req.version_req(), &VersionReq::parse("^2.0.0").unwrap());
        let name = req.name();
        assert!(req.matches(name, &Version::new(2, 0, 0)));
        assert!(!req.matches(name, &Version::new(3, 0, 0)));
    }
}
