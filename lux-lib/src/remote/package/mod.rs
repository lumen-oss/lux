//! Remote package types.

pub mod kind;
pub mod source;

pub(crate) use source::Source;

use std::cmp::Ordering;
use std::str::FromStr;

use miette::Diagnostic;
use thiserror::Error;

use crate::lockfile::RemotePackageSourceUrl;
use crate::package::version;
use crate::package::Spec;

#[derive(Clone, Debug)]
pub(crate) struct Package {
    pub spec: Spec,
    pub source: Source,
    /// `Some` if present in a lockfile
    pub source_url: Option<RemotePackageSourceUrl>,
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub(crate) enum Kind {
    Rockspec,
    Src,
    Binary,
}

impl Ord for Kind {
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

impl PartialOrd for Kind {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Error, Debug, Diagnostic)]
pub enum Error {
    #[error("unable to parse package {0}. expected format: `name@version`")]
    InvalidInput(String),
    #[error("unable to parse package {package_str}:\n{error}")]
    InvalidPackageVersion {
        #[source]
        error: version::Error,
        package_str: String,
    },
}

impl Package {
    pub fn new(spec: Spec, source: Source, source_url: Option<RemotePackageSourceUrl>) -> Self {
        Self {
            spec,
            source,
            source_url,
        }
    }
}

impl FromStr for Spec {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let (name, version) = s
            .split_once('@')
            .ok_or_else(|| Error::InvalidInput(s.to_string()))?;

        Self::parse(name.to_string(), version.to_string()).map_err(|error| {
            Error::InvalidPackageVersion {
                error,
                package_str: s.to_string(),
            }
        })
    }
}
