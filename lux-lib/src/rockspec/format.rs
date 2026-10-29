use miette::Diagnostic;
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::str::FromStr;
use thiserror::Error;

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
#[error("invalid rockspec format")]
pub struct InvalidRockspecFormat(String);

#[derive(Default, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Format {
    #[serde(rename = "1.0")]
    _1_0,
    #[serde(rename = "2.0")]
    _2_0,
    #[serde(rename = "3.0")]
    #[default]
    _3_0,
}

impl FromStr for Format {
    type Err = InvalidRockspecFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "1.0" => Ok(Self::_1_0),
            "2.0" => Ok(Self::_2_0),
            "3.0" => Ok(Self::_3_0),
            txt => Err(InvalidRockspecFormat(txt.to_string())),
        }
    }
}

impl Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::_1_0 => write!(f, "1.0"),
            Self::_2_0 => write!(f, "2.0"),
            Self::_3_0 => write!(f, "3.0"),
        }
    }
}
