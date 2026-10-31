use std::fmt::Display;

use miette::Diagnostic;
use thiserror::Error;

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
        let current = self.current;
        self.current += 1;
        if self.current == 0 {
            None // overflow
        } else {
            Some(SpecRev(current))
        }
    }
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum SpecRevParseError {
    #[error("specrev {specrev} in version {full_version} contains non-numeric characters")]
    InvalidSpecrev {
        specrev: String,
        full_version: String,
    },
    #[error("could not parse specrev in version {0}")]
    InvalidVersion(String),
}

pub(super) fn split_specrev(version_str: &str) -> Result<(&str, SpecRev), SpecRevParseError> {
    if let Some(pos) = version_str.rfind('-') {
        if let Some(specrev_str) = version_str.get(pos + 1..) {
            if specrev_str.chars().all(|c| c.is_ascii_digit()) {
                let specrev =
                    specrev_str
                        .parse::<u16>()
                        .map_err(|_| SpecRevParseError::InvalidSpecrev {
                            specrev: specrev_str.into(),
                            full_version: version_str.into(),
                        })?;
                Ok((&version_str[..pos], specrev.into()))
            } else {
                // A non-numeric suffix is a semver prerelease, not a specrev.
                Ok((version_str, 1.into()))
            }
        } else {
            Err(SpecRevParseError::InvalidVersion(version_str.into()))
        }
    } else {
        // We assume a specrev of 1 if none can be found.
        Ok((version_str, 1.into()))
    }
}
