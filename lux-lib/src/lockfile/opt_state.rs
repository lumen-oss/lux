use std::fmt::Display;

use serde::{Deserialize, Serialize};

#[derive(Copy, Debug, PartialEq, Eq, Hash, Clone, PartialOrd, Ord, Default)]
pub enum OptState {
    /// A required package
    #[default]
    Required,
    /// An optional package
    Optional,
}

impl OptState {
    pub(crate) fn as_bool(&self) -> bool {
        match self {
            Self::Required => false,
            Self::Optional => true,
        }
    }

    pub(super) fn is_default(&self) -> bool {
        &Self::default() == self
    }
}

impl From<bool> for OptState {
    fn from(value: bool) -> Self {
        if value {
            Self::Optional
        } else {
            Self::Required
        }
    }
}

impl Display for OptState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            OptState::Required => "required".fmt(f),
            OptState::Optional => "optional".fmt(f),
        }
    }
}

impl Serialize for OptState {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_bool(self.as_bool())
    }
}

impl<'de> Deserialize<'de> for OptState {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(match bool::deserialize(deserializer)? {
            false => Self::Required,
            true => Self::Optional,
        })
    }
}
