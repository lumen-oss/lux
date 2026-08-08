use serde::{Deserialize, Serialize};
use strum_macros::Display;

/// Configuration for the build process.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct BuildConfig {
    /// The build profile to use when compiling packages.
    /// Default: [`BuildProfile::Release`]
    pub(super) profile: Option<Profile>,
    /// Command prefix with which to wrap all build commands.
    ///
    /// If set, every command spawned by the build backends is invoked as
    /// `runner + [command, arguments...]`.
    ///
    /// If unset, no wrapping is performed.
    #[serde(default)]
    pub(super) runner: Vec<String>,
}

/// The build profile to use when compiling packages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Display)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum Profile {
    Release,
    Dev,
}

impl Profile {
    /// The C compiler optimization level for this profile.
    pub fn opt_level(self) -> u32 {
        match self {
            Self::Release => 3,
            Self::Dev => 0,
        }
    }
}
