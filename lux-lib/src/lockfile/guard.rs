use std::ops::{Deref, DerefMut};

use serde::Deserialize;

use super::{Lockfile, ReadWrite};

/// Flushes a lockfile automatically when it goes out of scope
pub struct Guard(Lockfile<ReadWrite>);

impl Guard {
    pub(super) fn new(lockfile: Lockfile<ReadWrite>) -> Self {
        Self(lockfile)
    }
}

impl serde::Serialize for Guard {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Guard {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Guard(Lockfile::<ReadWrite>::deserialize(deserializer)?))
    }
}

impl Deref for Guard {
    type Target = Lockfile<ReadWrite>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Guard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}
