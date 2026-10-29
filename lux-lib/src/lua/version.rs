use crate::package;
use std::{
    fmt::Display,
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::config::Config;
use itertools::Itertools;
use miette::Diagnostic;
use serde::{Deserialize, Serialize};
use strum::IntoEnumIterator;
use strum_macros::EnumIter;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, EnumIter)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Version {
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
    #[serde(rename = "luau")]
    Luau,
}

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("unsupported Lua version: '{0}'")]
    UnsupportedVersion(package::Version),
    #[error("lua version not set")]
    Unset {
        #[help]
        help: String,
    },
}

impl Version {
    pub fn as_version(&self) -> package::Version {
        unsafe {
            match self {
                Version::Lua51 => "5.1.0".parse().unwrap_unchecked(),
                Version::Lua52 => "5.2.0".parse().unwrap_unchecked(),
                Version::Lua53 => "5.3.0".parse().unwrap_unchecked(),
                Version::Lua54 => "5.4.0".parse().unwrap_unchecked(),
                Version::Lua55 => "5.5.0".parse().unwrap_unchecked(),
                Version::LuaJIT => "5.1.0".parse().unwrap_unchecked(),
                Version::LuaJIT52 => "5.2.0".parse().unwrap_unchecked(),
                Version::Luau => "5.1.0".parse().unwrap_unchecked(),
            }
        }
    }
    pub fn version_compatibility_str(&self) -> String {
        match self {
            Version::Lua51 | Version::LuaJIT | Version::Luau => "5.1".into(),
            Version::Lua52 | Version::LuaJIT52 => "5.2".into(),
            Version::Lua53 => "5.3".into(),
            Version::Lua54 => "5.4".into(),
            Version::Lua55 => "5.5".into(),
        }
    }
    pub fn as_version_req(&self) -> package::VersionReq {
        unsafe {
            format!("~> {}", self.version_compatibility_str())
                .parse()
                .unwrap_unchecked()
        }
    }

    /// Get the Version from a version that has been parsed from the `lua -v` output
    pub fn from_version(version: package::Version) -> Result<Version, Error> {
        // NOTE: Special case. luajit -v outputs 2.x.y as a version
        let luajit_version_req: package::VersionReq = unsafe { "~> 2".parse().unwrap_unchecked() };
        if luajit_version_req.matches(&version) {
            Ok(Version::LuaJIT)
        } else if Version::Lua51.as_version_req().matches(&version) {
            Ok(Version::Lua51)
        } else if Version::Lua52.as_version_req().matches(&version) {
            Ok(Version::Lua52)
        } else if Version::Lua53.as_version_req().matches(&version) {
            Ok(Version::Lua53)
        } else if Version::Lua54.as_version_req().matches(&version) {
            Ok(Version::Lua54)
        } else if Version::Lua55.as_version_req().matches(&version) {
            Ok(Version::Lua55)
        } else {
            Err(Error::UnsupportedVersion(version))
        }
    }

    pub(crate) fn is_luajit(&self) -> bool {
        matches!(self, Self::LuaJIT | Self::LuaJIT52)
    }

    pub(crate) fn is_luau(&self) -> bool {
        matches!(self, Self::Luau)
    }

    /// Searches for the path to the lux-lua library for this version
    pub fn lux_lib_dir(&self) -> Option<PathBuf> {
        option_env!("LUX_LIB_DIR")
            .map(PathBuf::from)
            .map(|path| path.join(self.to_string()))
            .or_else(|| self.probe_lux_lib_dir())
            .or_else(|| {
                match self {
                    Version::LuaJIT => Some(Version::Lua51),
                    Version::LuaJIT52 => Some(Version::Lua52),
                    _ => None,
                }
                .and_then(|compat_version| compat_version.probe_lux_lib_dir())
            })
            .or_else(|| lux_lib_resource_dir().map(|path| path.join(self.to_string())))
    }

    fn probe_lux_lib_dir(&self) -> Option<PathBuf> {
        let lib_name = format!("lux-lua{self}");
        pkg_config::Config::new()
            .print_system_libs(false)
            .cargo_metadata(false)
            .env_metadata(false)
            .probe(&lib_name)
            .ok()
            .and_then(|library| library.link_paths.first().cloned())
    }
}

impl Version {
    pub fn from(config: &Config) -> Result<&Self, Error> {
        config.lua_version().ok_or(Error::Unset {
            help: format!(
                r#"provide a version through `lx --lua-version <ver> <cmd>`
or make sure Lux can discover your Lua installation on your PATH.
valid versions are: {}
"#,
                Version::iter().join(",")
            ),
        })
    }
}

impl FromStr for Version {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "5.1" | "51" => Ok(Version::Lua51),
            "5.2" | "52" => Ok(Version::Lua52),
            "5.3" | "53" => Ok(Version::Lua53),
            "5.4" | "54" => Ok(Version::Lua54),
            "5.5" | "55" => Ok(Version::Lua55),
            "jit" | "luajit" => Ok(Version::LuaJIT),
            "jit52" | "luajit52" => Ok(Version::LuaJIT52),
            "luau" => Ok(Version::Luau),
            _ => Err(format!(
                r#"unrecognized Lua version.
supported versions: {}
            "#,
                Version::iter().join(",")
            )),
        }
    }
}

impl Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Version::Lua51 => "5.1",
            Version::Lua52 => "5.2",
            Version::Lua53 => "5.3",
            Version::Lua54 => "5.4",
            Version::Lua55 => "5.5",
            Version::LuaJIT => "jit",
            Version::LuaJIT52 => "jit52",
            Version::Luau => "luau",
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
        if let Some(appdir) = std::env::var_os("APPDIR") {
            let lib_dir = Path::new(&appdir).join("usr/share/lux-lua");
            if lib_dir.is_dir() {
                return Some(lib_dir);
            }
        }
        let lib_dir = PathBuf::from("/usr/share/lux-lua");
        if lib_dir.is_dir() {
            Some(lib_dir)
        } else {
            None
        }
    }
}
