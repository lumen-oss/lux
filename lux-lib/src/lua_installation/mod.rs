use is_executable::IsExecutable;
use itertools::Itertools;
use path_slash::PathBufExt;
use std::fmt;
use std::fmt::Display;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use thiserror::Error;
use which::which;

use crate::build::external_dependency::to_lib_name;
use crate::build::external_dependency::ExternalDependencyInfo;
use crate::build::utils::{c_lib_extension, format_path};
use crate::config::external_deps::ExternalDependencySearchConfig;
use crate::config::LuaVersionUnset;
use crate::lua_rockspec::ExternalDependencySpec;
use crate::operations;
use crate::operations::BuildLuaError;
use crate::progress::Progress;
use crate::progress::ProgressBar;
use crate::variables::GetVariableError;
use crate::{
    config::{Config, LuaVersion},
    package::PackageVersion,
    variables::HasVariables,
};
use lazy_static::lazy_static;
use tokio::sync::Mutex;

// Because installing lua is not thread-safe, we have to synchronize with a global Mutex
lazy_static! {
    static ref NEW_MUTEX: Mutex<i32> = Mutex::new(0i32);
    static ref INSTALL_MUTEX: Mutex<i32> = Mutex::new(0i32);
}

#[derive(Debug)]
pub struct LuaInstallation {
    pub version: LuaVersion,
    dependency_info: ExternalDependencyInfo,
    /// Binary to the Lua executable, if present
    pub(crate) bin: Option<PathBuf>,
}

#[derive(Debug, Error)]
pub enum LuaBinaryError {
    #[error("neither `lua` nor `luajit` found on the PATH")]
    LuaBinaryNotFound,
    #[error(transparent)]
    DetectLuaVersion(#[from] DetectLuaVersionError),
    #[error(
        r#"
{} -v (= {}) does not match expected Lua version: {}.

Try setting

```toml
[variables]
LUA = "/path/to/lua_binary"
```

in your config, or use `-v LUA=/path/to/lua_binary`.
    "#,
        lua_cmd,
        installed_version,
        lua_version
    )]
    LuaVersionMismatch {
        lua_cmd: String,
        installed_version: PackageVersion,
        lua_version: LuaVersion,
    },
    #[error("{0} not found on the PATH")]
    CustomBinaryNotFound(String),
}

#[derive(Error, Debug)]
pub enum DetectLuaVersionError {
    #[error("failed to run {0}: {1}")]
    RunLuaCommand(String, io::Error),
    #[error("failed to parse Lua version from output: {0}")]
    ParseLuaVersion(String),
    #[error(transparent)]
    PackageVersionParse(#[from] crate::package::PackageVersionParseError),
    #[error(transparent)]
    LuaVersion(#[from] crate::config::LuaVersionError),
}

#[derive(Error, Debug)]
pub enum LuaInstallationError {
    #[error("could not find a Lua installation and failed to build Lua from source:\n{0}")]
    Build(#[from] BuildLuaError),
    #[error(transparent)]
    LuaVersionUnset(#[from] LuaVersionUnset),
}

impl LuaInstallation {
    pub async fn new_from_config(
        config: &Config,
        progress: &Progress<ProgressBar>,
    ) -> Result<Self, LuaInstallationError> {
        Self::new(LuaVersion::from(config)?, config, progress).await
    }

    pub async fn new(
        version: &LuaVersion,
        config: &Config,
        progress: &Progress<ProgressBar>,
    ) -> Result<Self, LuaInstallationError> {
        let _lock = NEW_MUTEX.lock().await;
        if let Some(lua_intallation) = Self::probe(version, config.external_deps()) {
            return Ok(lua_intallation);
        }
        let output = Self::root_dir(version, config);
        let include_dir = output.join("include");
        let lib_dir = output.join("lib");
        let lua_lib_name = get_lua_lib_name(&lib_dir, version);
        if include_dir.is_dir() && lua_lib_name.is_some() {
            let bin_dir = Some(output.join("bin")).filter(|bin_path| bin_path.is_dir());
            let bin = bin_dir
                .as_ref()
                .and_then(|bin_path| find_lua_executable(bin_path, version));
            let lib_dir = output.join("lib");
            let lua_lib_name = get_lua_lib_name(&lib_dir, version);
            let include_dir = Some(output.join("include"));
            Ok(LuaInstallation {
                version: version.clone(),
                dependency_info: ExternalDependencyInfo {
                    include_dir,
                    lib_dir: Some(lib_dir),
                    bin_dir,
                    lib_info: None,
                    lib_name: lua_lib_name,
                },
                bin,
            })
        } else {
            Self::install(version, config, progress).await
        }
    }

    pub(crate) fn probe(
        version: &LuaVersion,
        search_config: &ExternalDependencySearchConfig,
    ) -> Option<Self> {
        let pkg_name_probes = match version {
            LuaVersion::Lua51 => vec!["lua5.1", "lua-5.1"],
            LuaVersion::Lua52 => vec!["lua5.2", "lua-5.2"],
            LuaVersion::Lua53 => vec!["lua5.3", "lua-5.3"],
            LuaVersion::Lua54 => vec!["lua5.4", "lua-5.4"],
            LuaVersion::LuaJIT | LuaVersion::LuaJIT52 => vec!["luajit"],
        };

        let mut dependency_info = pkg_name_probes
            .iter()
            .map(|pkg_name| {
                ExternalDependencyInfo::probe(
                    pkg_name,
                    &ExternalDependencySpec::default(),
                    search_config,
                )
            })
            .find_map(Result::ok);

        if let Some(info) = &mut dependency_info {
            let bin = info.lib_dir.as_ref().and_then(|lib_dir| {
                lib_dir
                    .parent()
                    .map(|parent| parent.join("bin"))
                    .filter(|dir| dir.is_dir())
                    .and_then(|bin_path| find_lua_executable(&bin_path, version))
            });
            let lua_lib_name = info
                .lib_dir
                .as_ref()
                .and_then(|lib_dir| get_lua_lib_name(lib_dir, version));
            info.lib_name = lua_lib_name;
            dependency_info.map(|dependency_info| Self {
                version: version.clone(),
                dependency_info,
                bin,
            })
        } else {
            None
        }
    }

    pub async fn install(
        version: &LuaVersion,
        config: &Config,
        progress: &Progress<ProgressBar>,
    ) -> Result<Self, LuaInstallationError> {
        let _lock = INSTALL_MUTEX.lock().await;

        let target = Self::root_dir(version, config);

        operations::BuildLua::new()
            .lua_version(version)
            .install_dir(&target)
            .config(config)
            .progress(progress)
            .build()
            .await?;

        let include_dir = target.join("include");
        let lib_dir = target.join("lib");
        let bin_dir = Some(target.join("bin")).filter(|bin_path| bin_path.is_dir());
        let bin = bin_dir
            .as_ref()
            .and_then(|bin_path| find_lua_executable(bin_path, version));
        let lua_lib_name = get_lua_lib_name(&lib_dir, version);
        Ok(LuaInstallation {
            version: version.clone(),
            dependency_info: ExternalDependencyInfo {
                include_dir: Some(include_dir),
                lib_dir: Some(lib_dir),
                bin_dir,
                lib_info: None,
                lib_name: lua_lib_name,
            },
            bin,
        })
    }

    pub fn includes(&self) -> Vec<&PathBuf> {
        self.dependency_info.include_dir.iter().collect_vec()
    }

    pub fn bin(&self) -> &Option<PathBuf> {
        &self.bin
    }

    fn root_dir(version: &LuaVersion, config: &Config) -> PathBuf {
        if let Some(lua_dir) = config.lua_dir() {
            return lua_dir.clone();
        } else if let Ok(tree) = config.user_tree(version.clone()) {
            return tree.root().join(".lua");
        }
        config.data_dir().join(".lua").join(version.to_string())
    }

    #[cfg(not(target_env = "msvc"))]
    fn lua_lib(&self) -> Option<String> {
        self.dependency_info
            .lib_name
            .as_ref()
            .map(|name| format!("{}.{}", name, c_lib_extension()))
    }

    #[cfg(target_env = "msvc")]
    fn lua_lib(&self) -> Option<String> {
        self.dependency_info.lib_name.clone()
    }

    pub(crate) fn define_flags(&self) -> Vec<String> {
        self.dependency_info.define_flags()
    }

    /// NOTE: In luarocks, these are behind a link_lua_explicity config option
    pub(crate) fn lib_link_args(&self, compiler: &cc::Tool) -> Vec<String> {
        if cfg!(target_os = "macos") {
            // On macos, linking Lua can lead to duplicate symbol errors
            Vec::new()
        } else {
            self.dependency_info.lib_link_args(compiler)
        }
    }

    /// Get the Lua binary (if present), prioritising
    /// a potentially overridden value in the config.
    pub(crate) fn lua_binary_or_config_override(&self, config: &Config) -> Option<String> {
        config.variables().get("LUA").cloned().or(self
            .bin
            .clone()
            .or(LuaBinary::new(self.version.clone(), config).try_into().ok())
            .map(|bin| bin.to_slash_lossy().to_string()))
    }
}

impl HasVariables for LuaInstallation {
    fn get_variable(&self, input: &str) -> Result<Option<String>, GetVariableError> {
        Ok(match input {
            "LUA_INCDIR" => self
                .dependency_info
                .include_dir
                .as_ref()
                .map(|dir| format_path(dir)),
            "LUA_LIBDIR" => self
                .dependency_info
                .lib_dir
                .as_ref()
                .map(|dir| format_path(dir)),
            "LUA_BINDIR" => self
                .bin
                .as_ref()
                .and_then(|bin| bin.parent().map(format_path)),
            "LUA" => self
                .bin
                .clone()
                .or(LuaBinary::Lua {
                    lua_version: self.version.clone(),
                }
                .try_into()
                .ok())
                .map(|lua| format_path(&lua)),
            "LUALIB" => self.lua_lib().or(Some("".into())),
            _ => None,
        })
    }
}

#[derive(Clone)]
pub enum LuaBinary {
    /// The regular Lua interpreter.
    Lua { lua_version: LuaVersion },
    /// Custom Lua interpreter.
    Custom(String),
}

impl Display for LuaBinary {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            LuaBinary::Lua { lua_version } => write!(f, "lua {lua_version}"),
            LuaBinary::Custom(cmd) => write!(f, "{cmd}"),
        }
    }
}

impl LuaBinary {
    /// Construct a new `LuaBinary` for the given `LuaVersion`,
    /// potentially prioritising an overridden value in the config.
    pub fn new(lua_version: LuaVersion, config: &Config) -> Self {
        match config.variables().get("LUA").cloned() {
            Some(lua) => Self::Custom(lua),
            None => Self::Lua { lua_version },
        }
    }
}

impl From<PathBuf> for LuaBinary {
    fn from(value: PathBuf) -> Self {
        Self::Custom(value.to_string_lossy().to_string())
    }
}

impl TryFrom<LuaBinary> for PathBuf {
    type Error = LuaBinaryError;

    fn try_from(value: LuaBinary) -> Result<Self, Self::Error> {
        match value {
            LuaBinary::Lua { lua_version } => {
                if let Some(lua_binary) =
                    LuaInstallation::probe(&lua_version, &ExternalDependencySearchConfig::default())
                        .and_then(|lua_installation| lua_installation.bin)
                {
                    return Ok(lua_binary);
                }
                if lua_version.is_luajit() {
                    if let Ok(path) = which("luajit") {
                        return Ok(path);
                    }
                }
                match which(format!("lua{}", lua_version)) {
                    Ok(path) => Ok(path),
                    Err(_) => detect_default_lua_bin(lua_version),
                }
            }
            LuaBinary::Custom(bin) => match which(&bin) {
                Ok(path) => Ok(path),
                Err(_) => Err(LuaBinaryError::CustomBinaryNotFound(bin)),
            },
        }
    }
}

fn detect_default_lua_bin(lua_version: LuaVersion) -> Result<PathBuf, LuaBinaryError> {
    match which("lua") {
        Ok(path) => {
            let installed_version = detect_installed_lua_version_from_path(&path)?;
            if lua_version
                .clone()
                .as_version_req()
                .matches(&installed_version)
            {
                Ok(path)
            } else {
                Err(LuaBinaryError::LuaVersionMismatch {
                    lua_cmd: path.to_slash_lossy().to_string(),
                    installed_version,
                    lua_version,
                })?
            }
        }
        Err(_) => Err(LuaBinaryError::LuaBinaryNotFound),
    }
}

pub fn detect_installed_lua_version() -> Option<LuaVersion> {
    which("lua")
        .ok()
        .or(which("luajit").ok())
        .and_then(|lua_cmd| {
            detect_installed_lua_version_from_path(&lua_cmd)
                .ok()
                .and_then(|version| LuaVersion::from_version(version).ok())
        })
}

fn find_lua_executable(bin_path: &Path, version: &LuaVersion) -> Option<PathBuf> {
    std::fs::read_dir(bin_path).ok().and_then(|entries| {
        let bin_files = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path().to_path_buf())
            .collect_vec();

        #[cfg(windows)]
        let ext = ".exe";

        #[cfg(not(windows))]
        let ext = "";

        // Prioritise Lua binaries with version suffix
        // (see https://github.com/lumen-oss/lux/issues/1215)
        let lua_version_bin = format!("lua{}{}", version, ext);
        if let Some(lua_bin) = bin_files
            .iter()
            .filter(|file| {
                file.is_executable()
                    && file
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy() == lua_version_bin)
            })
            .collect_vec()
            .first()
            .cloned()
        {
            Some(lua_bin.clone())
        } else {
            // Fall back to Lua binary without version suffix
            bin_files
                .into_iter()
                .filter(|file| {
                    file.is_executable()
                        && file.file_name().is_some_and(|name| {
                            matches!(
                                name.to_string_lossy().to_string().as_str(),
                                "lua" | "luajit" | "lua.exe" | "luajit.exe"
                            )
                        })
                })
                .collect_vec()
                .first()
                .cloned()
        }
    })
}

fn is_lua_lib_name(name: &str, lua_version: &LuaVersion) -> bool {
    let prefixes = match lua_version {
        LuaVersion::LuaJIT | LuaVersion::LuaJIT52 => vec!["luajit", "lua"],
        _ => vec!["lua"],
    };
    let version_str = lua_version.version_compatibility_str();
    let version_suffix = version_str.replace(".", "");
    #[cfg(target_family = "unix")]
    let name = name.trim_start_matches("lib");
    prefixes
        .iter()
        .any(|prefix| name == format!("{}.{}", *prefix, c_lib_extension()))
        || prefixes.iter().any(|prefix| name.starts_with(*prefix))
            && (name.contains(&version_str) || name.contains(&version_suffix))
}

fn get_lua_lib_name(lib_dir: &Path, lua_version: &LuaVersion) -> Option<String> {
    std::fs::read_dir(lib_dir)
        .ok()
        .and_then(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path().to_path_buf())
                .filter(|file| file.extension().is_some_and(|ext| ext == c_lib_extension()))
                .filter(|file| {
                    file.file_name()
                        .is_some_and(|name| is_lua_lib_name(&name.to_string_lossy(), lua_version))
                })
                .collect_vec()
                .first()
                .cloned()
        })
        .map(|file| to_lib_name(&file))
}

fn detect_installed_lua_version_from_path(
    lua_cmd: &Path,
) -> Result<PackageVersion, DetectLuaVersionError> {
    let output = match std::process::Command::new(lua_cmd).arg("-v").output() {
        Ok(output) => Ok(output),
        Err(err) => Err(DetectLuaVersionError::RunLuaCommand(
            lua_cmd.to_string_lossy().to_string(),
            err,
        )),
    }?;
    let output_vec = if output.stderr.is_empty() {
        output.stdout
    } else {
        // Yes, Lua 5.1 prints to stderr (-‸ლ)
        output.stderr
    };
    let lua_output = String::from_utf8_lossy(&output_vec).to_string();
    parse_lua_version_from_output(&lua_output)
}

fn parse_lua_version_from_output(
    lua_output: &str,
) -> Result<PackageVersion, DetectLuaVersionError> {
    let lua_version_str = lua_output
        .trim_start_matches("Lua")
        .trim_start_matches("JIT")
        .split_whitespace()
        .next()
        .map(|s| s.to_string())
        .ok_or(DetectLuaVersionError::ParseLuaVersion(
            lua_output.to_string(),
        ))?;
    Ok(PackageVersion::parse(&lua_version_str)?)
}

#[cfg(test)]
mod test {
    use crate::{config::ConfigBuilder, progress::MultiProgress};

    use super::*;

    #[tokio::test]
    async fn parse_luajit_version() {
        let luajit_output =
            "LuaJIT 2.1.1713773202 -- Copyright (C) 2005-2023 Mike Pall. https://luajit.org/";
        parse_lua_version_from_output(luajit_output).unwrap();
    }

    #[tokio::test]
    async fn parse_lua_51_version() {
        let lua_output = "Lua 5.1.5  Copyright (C) 1994-2012 Lua.org, PUC-Rio";
        parse_lua_version_from_output(lua_output).unwrap();
    }

    #[tokio::test]
    async fn lua_installation_bin() {
        if std::env::var("LUX_SKIP_IMPURE_TESTS").unwrap_or("0".into()) == "1" {
            println!("Skipping impure test");
            return;
        }
        let config = ConfigBuilder::new().unwrap().build().unwrap();
        let lua_version = config.lua_version().unwrap();
        let progress = MultiProgress::new(&config);
        let bar = progress.map(MultiProgress::new_bar);
        let lua_installation = LuaInstallation::new(lua_version, &config, &bar)
            .await
            .unwrap();
        // FIXME: This fails when run in the nix checkPhase
        assert!(lua_installation.bin.is_some());
        let lua_binary: LuaBinary = lua_installation.bin.unwrap().into();
        let lua_bin_path: PathBuf = lua_binary.try_into().unwrap();
        let pkg_version = detect_installed_lua_version_from_path(&lua_bin_path).unwrap();
        assert_eq!(&LuaVersion::from_version(pkg_version).unwrap(), lua_version);
    }

    #[cfg(not(target_env = "msvc"))]
    #[tokio::test]
    async fn test_is_lua_lib_name() {
        assert!(is_lua_lib_name("lua.a", &LuaVersion::Lua51));
        assert!(is_lua_lib_name("lua-5.1.a", &LuaVersion::Lua51));
        assert!(is_lua_lib_name("lua5.1.a", &LuaVersion::Lua51));
        assert!(is_lua_lib_name("lua51.a", &LuaVersion::Lua51));
        assert!(!is_lua_lib_name("lua-5.2.a", &LuaVersion::Lua51));
        assert!(is_lua_lib_name("luajit-5.2.a", &LuaVersion::LuaJIT52));
        assert!(is_lua_lib_name("lua-5.2.a", &LuaVersion::LuaJIT52));
        assert!(is_lua_lib_name("liblua.a", &LuaVersion::Lua51));
        assert!(is_lua_lib_name("liblua-5.1.a", &LuaVersion::Lua51));
        assert!(is_lua_lib_name("liblua53.a", &LuaVersion::Lua53));
        assert!(is_lua_lib_name("liblua-54.a", &LuaVersion::Lua54));
    }

    #[cfg(target_env = "msvc")]
    #[tokio::test]
    async fn test_is_lua_lib_name() {
        assert!(is_lua_lib_name("lua.lib", &LuaVersion::Lua51));
        assert!(is_lua_lib_name("lua-5.1.lib", &LuaVersion::Lua51));
        assert!(!is_lua_lib_name("lua-5.2.lib", &LuaVersion::Lua51));
        assert!(!is_lua_lib_name("lua53.lib", &LuaVersion::Lua53));
        assert!(!is_lua_lib_name("lua53.lib", &LuaVersion::Lua53));
        assert!(is_lua_lib_name("luajit-5.2.lib", &LuaVersion::LuaJIT52));
        assert!(is_lua_lib_name("lua-5.2.lib", &LuaVersion::LuaJIT52));
    }
}
