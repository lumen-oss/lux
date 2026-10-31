use std::{
    fmt::{self, Display},
    str::FromStr,
};

use serde::{de::Error as _, ser::Serializer, Deserialize, Deserializer, Serialize};
use thiserror::Error;

/// A package name, of the form `scope/name`.
///
/// Both parts contain only lowercase letters, digits, and dashes (`-`), and
/// are at most 64 characters long.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Name {
    scope: PackageNamePart,
    name: PackageNamePart,
}

impl Name {
    pub(crate) fn scope(&self) -> &str {
        self.scope.as_str()
    }

    pub(crate) fn name(&self) -> &str {
        self.name.as_str()
    }
}

/// A part of a [`Name`]: lowercase letters, digits, and dashes, 1-64 chars.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct PackageNamePart(String);

impl PackageNamePart {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for PackageNamePart {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let valid = s
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !valid || s.is_empty() || s.len() > 64 {
            return Err(Error::InvalidPart(s.to_string()));
        }
        Ok(Self(s.to_string()))
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("wally package name part '{0}' is invalid: it must contain only lowercase letters, digits and '-', and be 1-64 characters long")]
    InvalidPart(String),
    #[error("wally package name must be of the form SCOPE/NAME")]
    InvalidFormat,
}

impl Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.scope.as_str(), self.name.as_str())
    }
}

impl FromStr for Name {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (scope, name) = s.split_once('/').ok_or(Error::InvalidFormat)?;
        if name.contains('/') {
            return Err(Error::InvalidFormat);
        }
        Ok(Self {
            scope: scope.parse()?,
            name: name.parse()?,
        })
    }
}

impl Serialize for Name {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Name {
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
    fn test_parse_package_name() {
        assert!("Upper-Skewer".parse::<PackageNamePart>().is_err());
        assert!("snake_case".parse::<PackageNamePart>().is_err());
        assert!("hello/world/foo".parse::<Name>().is_err());
        assert!("hello/world".parse::<Name>().is_ok());
    }
}
