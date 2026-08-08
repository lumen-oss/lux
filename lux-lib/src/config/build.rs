use serde::{Deserialize, Serialize};

/// Configuration for the build process.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct BuildConfig {
    /// Command prefix with which to wrap all build commands.
    ///
    /// If set, every command spawned by the build backends is invoked as
    /// `runner + [command, arguments...]`.
    ///
    /// If unset, no wrapping is performed.
    #[serde(default)]
    pub(super) runner: Vec<String>,
}
