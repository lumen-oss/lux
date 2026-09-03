use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Template configuration for a rock's tree layout
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct RockLayoutConfig {
    /// The root for relocated directories (etc, src).
    /// If unset (the default), directories are placed in the package directory.
    /// If set, it is a directory relative to the given Lua version's install tree root.
    /// With the `--nvim` preset, this is `site/pack/lux`.
    #[serde(alias = "etc_root")]
    pub(crate) root: Option<PathBuf>,
    /// The `etc` directory for non-optional packages
    /// Default: `etc` With the `--nvim` preset, this is `start`
    /// Note: If `root` is set, the package ID is appended.
    pub(crate) etc: PathBuf,
    /// The `etc` directory for optional packages
    /// Default: `etc`
    /// With the `--nvim` preset, this is `opt`
    /// Note: If `root` is set, the package ID is appended.
    pub(crate) opt_etc: PathBuf,
    /// The `src` directory name
    /// Default: `src`
    /// With the `--nvim` preset, this is `lua`
    pub(crate) src: PathBuf,
    /// The `lib` directory name
    /// Default: `lib`
    pub(crate) lib: PathBuf,
    /// The `conf` directory name
    /// Default: `conf`
    pub(crate) conf: PathBuf,
    /// The `doc` directory name
    /// Default: `doc`
    pub(crate) doc: PathBuf,
}

impl RockLayoutConfig {
    /// Creates a `RockLayoutConfig` for use with Neovim
    /// - `root`: `site/pack/lux`
    /// - `etc`: `start`
    /// - `opt_etc`: `opt`
    /// - `src`: `lua`
    /// - `lib`: `lib`
    pub fn new_nvim_layout() -> Self {
        Self {
            root: Some("site/pack/lux".into()),
            etc: "start".into(),
            opt_etc: "opt".into(),
            src: "lua".into(),
            lib: "lib".into(),
            conf: "conf".into(),
            doc: "doc".into(),
        }
    }

    pub(crate) fn is_default(&self) -> bool {
        &Self::default() == self
    }
}

impl Default for RockLayoutConfig {
    fn default() -> Self {
        Self {
            root: None,
            etc: "etc".into(),
            opt_etc: "etc".into(),
            src: "src".into(),
            lib: "lib".into(),
            conf: "conf".into(),
            doc: "doc".into(),
        }
    }
}
