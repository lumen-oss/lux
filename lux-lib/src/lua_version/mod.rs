use std::{
    fmt::Display,
    path::{Path, PathBuf},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    config::Config,
    package::{PackageVersion, PackageVersionReq},
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum LuaVersion {
    #[serde(rename = "5.1")]
    Lua51,
    #[serde(rename = "5.2")]
    Lua52,
    #[serde(rename = "5.3")]
    Lua53,
    #[serde(rename = "5.4")]
    Lua54,
    #[serde(rename = "5.5")]
    Lua55,
    #[serde(rename = "jit")]
    LuaJIT,
    #[serde(rename = "jit5.2")]
    LuaJIT52,
    // TODO(vhyrro): Support luau?
    // LuaU,
}

#[derive(Debug, Error)]
pub enum LuaVersionError {
    #[error("unsupported Lua version: {0}")]
    UnsupportedLuaVersion(PackageVersion),
}

impl LuaVersion {
    pub fn as_version(&self) -> PackageVersion {
        unsafe {
            match self {
                LuaVersion::Lua51 => "5.1.0".parse().unwrap_unchecked(),
                LuaVersion::Lua52 => "5.2.0".parse().unwrap_unchecked(),
                LuaVersion::Lua53 => "5.3.0".parse().unwrap_unchecked(),
                LuaVersion::Lua54 => "5.4.0".parse().unwrap_unchecked(),
                LuaVersion::Lua55 => "5.5.0".parse().unwrap_unchecked(),
                LuaVersion::LuaJIT => "5.1.0".parse().unwrap_unchecked(),
                LuaVersion::LuaJIT52 => "5.2.0".parse().unwrap_unchecked(),
            }
        }
    }
    pub fn version_compatibility_str(&self) -> String {
        match self {
            LuaVersion::Lua51 | LuaVersion::LuaJIT => "5.1".into(),
            LuaVersion::Lua52 | LuaVersion::LuaJIT52 => "5.2".into(),
            LuaVersion::Lua53 => "5.3".into(),
            LuaVersion::Lua54 => "5.4".into(),
            LuaVersion::Lua55 => "5.5".into(),
        }
    }
    pub fn as_version_req(&self) -> PackageVersionReq {
        unsafe {
            format!("~> {}", self.version_compatibility_str())
                .parse()
                .unwrap_unchecked()
        }
    }

    /// Get the LuaVersion from a version that has been parsed from the `lua -v` output
    pub fn from_version(version: PackageVersion) -> Result<LuaVersion, LuaVersionError> {
        // NOTE: Special case. luajit -v outputs 2.x.y as a version
        let luajit_version_req: PackageVersionReq = unsafe { "~> 2".parse().unwrap_unchecked() };
        if luajit_version_req.matches(&version) {
            Ok(LuaVersion::LuaJIT)
        } else if LuaVersion::Lua51.as_version_req().matches(&version) {
            Ok(LuaVersion::Lua51)
        } else if LuaVersion::Lua52.as_version_req().matches(&version) {
            Ok(LuaVersion::Lua52)
        } else if LuaVersion::Lua53.as_version_req().matches(&version) {
            Ok(LuaVersion::Lua53)
        } else if LuaVersion::Lua54.as_version_req().matches(&version) {
            Ok(LuaVersion::Lua54)
        } else if LuaVersion::Lua55.as_version_req().matches(&version) {
            Ok(LuaVersion::Lua55)
        } else {
            Err(LuaVersionError::UnsupportedLuaVersion(version))
        }
    }

    pub(crate) fn is_luajit(&self) -> bool {
        matches!(self, Self::LuaJIT | Self::LuaJIT52)
    }

    /// Searches for the path to the lux-lua library for this version
    pub fn lux_lib_dir(&self) -> Option<PathBuf> {
        option_env!("LUX_LIB_DIR")
            .map(PathBuf::from)
            .map(|path| path.join(self.to_string()))
            .or_else(|| {
                let lib_name = format!("lux-lua{self}");
                pkg_config::Config::new()
                    .print_system_libs(false)
                    .cargo_metadata(false)
                    .env_metadata(false)
                    .probe(&lib_name)
                    .ok()
                    .and_then(|library| library.link_paths.first().cloned())
            })
            .or_else(|| lux_lib_resource_dir().map(|path| path.join(self.to_string())))
    }
}

#[derive(Error, Debug)]
#[error(
    r#"lua version not set.
Please provide a version through `lx --lua-version <ver> <cmd>`
Valid versions are: '5.1', '5.2', '5.3', '5.4', '5.5', 'jit' and 'jit52'.
"#
)]
pub struct LuaVersionUnset;

impl LuaVersion {
    pub fn from(config: &Config) -> Result<&Self, LuaVersionUnset> {
        config.lua_version().ok_or(LuaVersionUnset)
    }
}

impl FromStr for LuaVersion {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "5.1" | "51" => Ok(LuaVersion::Lua51),
            "5.2" | "52" => Ok(LuaVersion::Lua52),
            "5.3" | "53" => Ok(LuaVersion::Lua53),
            "5.4" | "54" => Ok(LuaVersion::Lua54),
            "5.5" | "55" => Ok(LuaVersion::Lua55),
            "jit" | "luajit" => Ok(LuaVersion::LuaJIT),
            "jit52" | "luajit52" => Ok(LuaVersion::LuaJIT52),
            _ => Err(r#"unrecognized Lua version.
                 Supported versions: '5.1', '5.2', '5.3', '5.4', '5.5', 'jit', 'jit52'.
                 "#
            .into()),
        }
    }
}

impl Display for LuaVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            LuaVersion::Lua51 => "5.1",
            LuaVersion::Lua52 => "5.2",
            LuaVersion::Lua53 => "5.3",
            LuaVersion::Lua54 => "5.4",
            LuaVersion::Lua55 => "5.5",
            LuaVersion::LuaJIT => "jit",
            LuaVersion::LuaJIT52 => "jit52",
        })
    }
}

/// Searches for the lux-lib directory in a binary distribution's resources
fn lux_lib_resource_dir() -> Option<PathBuf> {
    if cfg!(target_env = "msvc") {
        // The msvc .exe and .msi binary installers install lux-lua to the executable's directory.
        std::env::current_exe()
            .ok()
            .and_then(|exe_path| exe_path.parent().map(Path::to_path_buf))
            .and_then(|exe_dir| {
                let lib_dir = exe_dir.join("lux-lua");
                if lib_dir.is_dir() {
                    Some(lib_dir)
                } else {
                    None
                }
            })
    } else if cfg!(target_os = "macos") {
        // Currently, we only bundle resources with an .app ApplicationBundle
        std::env::current_exe()
            .ok()
            .and_then(|exe_path| exe_path.parent().map(Path::to_path_buf))
            .and_then(|macos_dir| macos_dir.parent().map(Path::to_path_buf))
            .and_then(|contents_dir| {
                let lib_dir = contents_dir.join("Resources").join("lux-lua");
                if lib_dir.is_dir() {
                    Some(lib_dir)
                } else {
                    None
                }
            })
    } else {
        // .deb and AppImage packages
        let lib_dir = PathBuf::from("/usr/share/lux-lua");
        if lib_dir.is_dir() {
            Some(lib_dir)
        } else {
            None
        }
    }
}
