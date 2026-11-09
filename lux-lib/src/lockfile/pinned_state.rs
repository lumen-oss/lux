use std::fmt::Display;

use serde::{Deserialize, Serialize};

#[derive(Copy, Debug, PartialEq, Eq, Hash, Clone, PartialOrd, Ord, Default)]
pub enum PinnedState {
    /// Unpinned packages can be updated
    #[default]
    Unpinned,
    /// Pinned packages cannot be updated
    Pinned,
}

impl Display for PinnedState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            PinnedState::Unpinned => "unpinned".fmt(f),
            PinnedState::Pinned => "pinned".fmt(f),
        }
    }
}

impl From<bool> for PinnedState {
    fn from(value: bool) -> Self {
        if value {
            Self::Pinned
        } else {
            Self::Unpinned
        }
    }
}

impl PinnedState {
    pub fn as_bool(&self) -> bool {
        match self {
            Self::Unpinned => false,
            Self::Pinned => true,
        }
    }

    pub(super) fn is_default(&self) -> bool {
        &Self::default() == self
    }
}

impl Serialize for PinnedState {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_bool(self.as_bool())
    }
}

impl<'de> Deserialize<'de> for PinnedState {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(match bool::deserialize(deserializer)? {
            false => Self::Unpinned,
            true => Self::Pinned,
        })
    }
}
