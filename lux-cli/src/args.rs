use clap::ValueEnum;
use lux_lib::{
    config::ConfigBuilder,
    lua_installation::nvim_lua_version,
    lua_version::LuaVersion,
    package::PackageReq,
    tree::{NvimLayout, RojoLayout},
};
use miette::{miette, Result};
use std::{path::PathBuf, str::FromStr};

#[derive(Debug, Clone)]
pub enum PackageOrRockspec {
    Package(PackageReq),
    RockSpec(PathBuf),
}

#[derive(Debug, Clone, PartialEq, ValueEnum)]
pub enum OutputFormat {
    Json,
    Text,
}

/// Configures Lux for a specific environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Preset {
    /// Configure Lux for [Neovim](https://neovim.io/) plugins.
    Nvim,
    /// Configure Lux for [Rojo](https://rojo.space/).
    Rojo,
}

impl Preset {
    pub fn lua_version(self) -> Option<LuaVersion> {
        match self {
            Self::Nvim => nvim_lua_version(),
            Self::Rojo => Some(LuaVersion::Luau),
        }
    }

    pub fn apply(self, config: ConfigBuilder) -> ConfigBuilder {
        match self {
            Self::Nvim => config.entrypoint_layout(NvimLayout),
            Self::Rojo => config.entrypoint_layout(RojoLayout),
        }
    }
}

impl FromStr for PackageOrRockspec {
    type Err = miette::Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let path = PathBuf::from(s);
        if path.is_file() {
            Ok(Self::RockSpec(path))
        } else {
            let pkg = PackageReq::from_str(s).map_err(|err| {
                miette!(
                    help = format!("if '{s}' is a path to a file, ensure it exists"),
                    "No file '{s}' found and cannot parse package query: {err}",
                )
            })?;
            Ok(Self::Package(pkg))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Cli;
    use clap::Parser;
    use lux_lib::lua_version::LuaVersion;

    #[test]
    fn parses_rojo_preset() {
        let cli = Cli::try_parse_from(["lx", "--preset", "rojo", "list"]).unwrap();
        assert_eq!(cli.preset, Some(Preset::Rojo));
        assert_eq!(Preset::Rojo.lua_version(), Some(LuaVersion::Luau));
    }

    #[test]
    fn parses_preset_and_deprecated_nvim_flag() {
        let with_preset = Cli::try_parse_from(["lx", "--preset", "nvim", "list"]).unwrap();
        assert_eq!(with_preset.preset, Some(Preset::Nvim));
        assert!(!with_preset.nvim);

        let with_nvim = Cli::try_parse_from(["lx", "--nvim", "list"]).unwrap();
        assert!(with_nvim.nvim);
        assert_eq!(with_nvim.preset, None);

        assert!(Cli::try_parse_from(["lx", "--preset", "nvim", "--nvim", "list"]).is_err());
    }
}
