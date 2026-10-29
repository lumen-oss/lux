use is_executable::IsExecutable;
use itertools::Itertools;
use miette::Diagnostic;
use path_slash::PathBufExt;
use std::path::Path;
use std::path::PathBuf;
use thiserror::Error;
use which::which;

use super::binary::Binary;
use super::detect;
use super::version::{self, Version};
use crate::build::external_dependency::to_lib_name;
use crate::build::external_dependency::ExternalDependencyInfo;
use crate::config::external_deps::ExternalDependencySearchConfig;
use crate::fs;
use crate::operations;
use crate::operations::BuildLuaError;
use crate::package;
use crate::rockspec;
use crate::tree::InstallTree;
use crate::variables::GetVariableError;
use crate::{config::Config, variables::HasVariables};
use crate::{path, target};
use lazy_static::lazy_static;
use tokio::sync::Mutex;

// Because installing lua is not thread-safe, we have to synchronize with a global Mutex
lazy_static! {
    static ref NEW_MUTEX: Mutex<i32> = Mutex::new(0i32);
    static ref INSTALL_MUTEX: Mutex<i32> = Mutex::new(0i32);
}

#[derive(Debug)]
pub struct Installation {
    pub version: Version,
    dependency_info: ExternalDependencyInfo,
    /// Binary to the Lua executable, if present
    pub(crate) bin: Option<PathBuf>,
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("could not find a Lua installation and failed to build Lua from source")]
    #[diagnostic(forward(0))]
    Build(#[from] BuildLuaError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Version(#[from] version::Error),
    #[error("`luau` binary not found after building Luau from source")]
    #[diagnostic(help("this is probably a bug"))]
    LuauBinaryNotFoundAfterBuild,
    #[error("neither `lua` nor `luajit` found on the PATH")]
    BinaryNotFound,
    #[error("neither `luau` nor `lune` found on the PATH.")]
    #[diagnostic(
        help("ensure luau is installed or run `lx install-lua --lua-version luau`"),
        url("https://github.com/luau-lang/luau/releases")
    )]
    LuauBinaryNotFound,
    #[error(transparent)]
    #[diagnostic(transparent)]
    DetectVersion(#[from] detect::Error),
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
        cmd,
        installed_version,
        required_version
    )]
    VersionMismatch {
        cmd: String,
        installed_version: package::Version,
        required_version: Version,
    },
    #[error("{0} not found on the PATH")]
    CustomBinaryNotFound(String),
}

impl Installation {
    #[tracing::instrument(level = "trace", skip_all)]
    pub async fn new_from_config(config: &Config) -> Result<Self, Error> {
        Self::new(Version::from(config)?, config).await
    }

    #[tracing::instrument(level = "trace", skip(config))]
    pub async fn new(version: &Version, config: &Config) -> Result<Self, Error> {
        let _lock = NEW_MUTEX.lock().await;
        if let Some(lua_intallation) = Self::probe(version, config.external_deps()) {
            return Ok(lua_intallation);
        }
        if version.is_luau() {
            if let Ok(bin) = PathBuf::try_from(Binary::new(version.clone(), config)) {
                return Ok(Self::from_bin_path(version.clone(), bin));
            }
            return match Self::luau_from_root_dir(&Self::root_dir(version, config)) {
                Some(lua_installation) => Ok(lua_installation),
                None => Self::install(version, config).await,
            };
        }
        let output = Self::root_dir(version, config);
        let include_dir = output.join("include");
        let lib_dir = output.join("lib");
        let lua_lib_name = lib_name(&lib_dir, version);
        if include_dir.is_dir() && lua_lib_name.is_some() {
            let bin_dir = Some(output.join("bin")).filter(|bin_path| bin_path.is_dir());
            let bin = bin_dir
                .as_ref()
                .and_then(|bin_path| find_executable(bin_path, version));
            let lib_dir = output.join("lib");
            let lua_lib_name = lib_name(&lib_dir, version);
            let include_dir = Some(output.join("include"));
            Ok(Installation {
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
            Self::install(version, config).await
        }
    }

    #[tracing::instrument(level = "trace")]
    pub(crate) fn probe(
        version: &Version,
        search_config: &ExternalDependencySearchConfig,
    ) -> Option<Self> {
        let pkg_name_probes = match version {
            Version::Lua51 => vec!["lua5.1", "lua-5.1"],
            Version::Lua52 => vec!["lua5.2", "lua-5.2"],
            Version::Lua53 => vec!["lua5.3", "lua-5.3"],
            Version::Lua54 => vec!["lua5.4", "lua-5.4"],
            Version::Lua55 => vec!["lua5.5", "lua-5.5"],
            Version::LuaJIT | Version::LuaJIT52 => vec!["luajit"],
            // NOTE: lune is a standalone binary with no pkg-config metadata or C API
            Version::Luau => vec!["luau"],
        };

        let mut dependency_info = pkg_name_probes
            .iter()
            .map(|pkg_name| {
                ExternalDependencyInfo::probe(
                    pkg_name,
                    &rockspec::ExternalDependencySpec::default(),
                    search_config,
                )
            })
            .find_map(Result::ok);

        if let Some(info) = &mut dependency_info {
            if info.include_dir.is_none() || info.lib_dir.is_none() {
                return None;
            }
            let bin = info.lib_dir.as_ref().and_then(|lib_dir| {
                lib_dir
                    .parent()
                    .map(|parent| parent.join("bin"))
                    .filter(|dir| dir.is_dir())
                    .and_then(|bin_path| find_executable(&bin_path, version))
            });
            let lua_lib_name = info
                .lib_dir
                .as_ref()
                .and_then(|lib_dir| lib_name(lib_dir, version));
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

    #[tracing::instrument(level = "trace", skip(config))]
    pub async fn install(version: &Version, config: &Config) -> Result<Self, Error> {
        let _lock = INSTALL_MUTEX.lock().await;

        let target = Self::root_dir(version, config);

        operations::BuildLua::new()
            .lua_version(version)
            .install_dir(&target)
            .config(config)
            .build()
            .await?;

        if version.is_luau() {
            return Self::luau_from_root_dir(&target).ok_or(Error::LuauBinaryNotFoundAfterBuild);
        }

        let include_dir = target.join("include");
        let lib_dir = target.join("lib");
        let bin_dir = Some(target.join("bin")).filter(|bin_path| bin_path.is_dir());
        let bin = bin_dir
            .as_ref()
            .and_then(|bin_path| find_executable(bin_path, version));
        let lua_lib_name = lib_name(&lib_dir, version);
        Ok(Installation {
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

    fn luau_from_root_dir(root: &Path) -> Option<Self> {
        let bin_dir = root.join("bin");
        let bin = find_executable(&bin_dir, &Version::Luau)?;
        Some(Self {
            version: Version::Luau,
            dependency_info: ExternalDependencyInfo {
                include_dir: None,
                lib_dir: None,
                bin_dir: bin.parent().map(Path::to_path_buf),
                lib_info: None,
                lib_name: None,
            },
            bin: Some(bin),
        })
    }

    /// A Lua installation referencing a binary found on the PATH,
    /// without probing for headers/libraries or building from source.
    /// Used for installations that don't link against the Lua C API.
    fn from_bin_path(version: Version, bin: PathBuf) -> Self {
        Self {
            version,
            dependency_info: ExternalDependencyInfo {
                include_dir: None,
                lib_dir: None,
                bin_dir: bin.parent().map(Path::to_path_buf),
                lib_info: None,
                lib_name: None,
            },
            bin: Some(bin),
        }
    }

    fn root_dir(version: &Version, config: &Config) -> PathBuf {
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
            .map(|name| format!("{}.{}", name, target::c_lib_extension()))
    }

    #[cfg(target_env = "msvc")]
    fn lua_lib(&self) -> Option<String> {
        self.dependency_info.lib_name.clone()
    }

    pub(crate) fn define_flags(&self) -> Vec<String> {
        let mut defines = self.dependency_info.define_flags();
        if self.version.is_luajit() {
            defines.push("-DLUAJIT_VERSION".into());
        }
        defines
    }

    pub(crate) fn lib_link_args(&self, compiler: &cc::Tool) -> Vec<String> {
        self.dependency_info.lib_link_args(compiler)
    }

    /// Get the Lua binary (if present), prioritising
    /// a potentially overridden value in the config.
    pub(crate) fn lua_binary_or_config_override(&self, config: &Config) -> Option<String> {
        let variable = if self.version.is_luau() {
            "LUAU"
        } else {
            "LUA"
        };
        config.variables().get(variable).cloned().or(self
            .bin
            .clone()
            .or(Binary::new(self.version.clone(), config).try_into().ok())
            .map(|bin| bin.to_slash_lossy().to_string()))
    }
}

impl HasVariables for Installation {
    #[tracing::instrument(level = "trace")]
    fn get_variable(&self, input: &str) -> Result<Option<String>, GetVariableError> {
        Ok(match input {
            "LUA_INCDIR" => self
                .dependency_info
                .include_dir
                .as_ref()
                .map(|dir| path::quoted(dir)),
            "LUA_LIBDIR" => self
                .dependency_info
                .lib_dir
                .as_ref()
                .map(|dir| path::quoted(dir)),
            "LUA_BINDIR" => self
                .bin
                .as_ref()
                .and_then(|bin| bin.parent().map(path::quoted)),
            "LUA" => self
                .bin
                .clone()
                .or(Binary::Lua {
                    version: self.version.clone(),
                }
                .try_into()
                .ok())
                .map(|lua| path::quoted(&lua)),
            "LUAU" if self.version.is_luau() => self.bin.as_ref().map(|bin| path::quoted(bin)),
            "LUALIB" => self.lua_lib().or(Some("".into())),
            _ => None,
        })
    }
}

impl TryFrom<Binary> for PathBuf {
    type Error = Error;

    fn try_from(value: Binary) -> Result<Self, Self::Error> {
        match value {
            Binary::Lua { version } => {
                if version.is_luau() {
                    return which("luau")
                        .or_else(|_| which("lune"))
                        .map_err(|_| Error::LuauBinaryNotFound);
                }
                if let Some(lua_binary) =
                    Installation::probe(&version, &ExternalDependencySearchConfig::default())
                        .and_then(|lua_installation| lua_installation.bin)
                {
                    return Ok(lua_binary);
                }
                if version.is_luajit() {
                    if let Ok(path) = which("luajit") {
                        return Ok(path);
                    }
                }
                match which(format!("lua{}", version)) {
                    Ok(path) => Ok(path),
                    Err(_) => default_bin(version),
                }
            }
            Binary::Custom(bin) => match which(&bin) {
                Ok(path) => Ok(path),
                Err(_) => Err(Error::CustomBinaryNotFound(bin)),
            },
        }
    }
}

fn default_bin(version: Version) -> Result<PathBuf, Error> {
    match which("lua") {
        Ok(path) => {
            let installed_version = detect::version_from_path(&path)?;
            if version.clone().as_version_req().matches(&installed_version) {
                Ok(path)
            } else {
                Err(Error::VersionMismatch {
                    cmd: path.to_slash_lossy().to_string(),
                    installed_version,
                    required_version: version,
                })?
            }
        }
        Err(_) => Err(Error::BinaryNotFound),
    }
}

/// Determine the Lua version to use with the `nvim` preset.
/// Prefers LuaJIT if installed, falling back to Lua 5.1.
pub fn version_for_neovim() -> Option<Version> {
    if Installation::probe(&Version::LuaJIT, &Default::default()).is_some() {
        Some(Version::LuaJIT)
    } else if Installation::probe(&Version::Lua51, &Default::default()).is_some() {
        Some(Version::Lua51)
    } else {
        None
    }
}

fn find_executable(bin_path: &Path, version: &Version) -> Option<PathBuf> {
    fs::sync::read_dir(bin_path).ok().and_then(|entries| {
        let bin_files = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path().to_path_buf())
            .collect_vec();

        #[cfg(windows)]
        let ext = ".exe";

        #[cfg(not(windows))]
        let ext = "";

        if version.is_luau() {
            let luau_bin = format!("luau{ext}");
            return bin_files
                .into_iter()
                .filter(|file| {
                    file.is_executable()
                        && file
                            .file_name()
                            .is_some_and(|name| name.to_string_lossy() == luau_bin)
                })
                .collect_vec()
                .first()
                .cloned();
        }

        // Prioritise Lua binaries with version suffix
        // (see https://github.com/lumen-oss/lux/issues/1215)
        let version_bin = format!("lua{}{}", version, ext);
        if let Some(lua_bin) = bin_files
            .iter()
            .filter(|file| {
                file.is_executable()
                    && file
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy() == version_bin)
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

fn is_lib_name(name: &str, version: &Version) -> bool {
    let prefixes = match version {
        Version::LuaJIT | Version::LuaJIT52 => vec!["luajit", "lua"],
        _ => vec!["lua"],
    };
    let version_str = version.version_compatibility_str();
    let version_suffix = version_str.replace(".", "");
    #[cfg(target_family = "unix")]
    let name = name.trim_start_matches("lib");
    prefixes
        .iter()
        .any(|prefix| name == format!("{}.{}", *prefix, target::c_lib_extension()))
        || prefixes.iter().any(|prefix| name.starts_with(*prefix))
            && (name.contains(&version_str) || name.contains(&version_suffix))
}

fn lib_name(lib_dir: &Path, version: &Version) -> Option<String> {
    fs::sync::read_dir(lib_dir)
        .ok()
        .and_then(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path().to_path_buf())
                .filter(|file| {
                    file.extension()
                        .is_some_and(|ext| ext == target::c_lib_extension())
                })
                .filter(|file| {
                    file.file_name()
                        .is_some_and(|name| is_lib_name(&name.to_string_lossy(), version))
                })
                .collect_vec()
                .first()
                .cloned()
        })
        .map(|file| to_lib_name(&file))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "impure_tests")]
    use flaky_test::flaky_test;

    #[cfg(feature = "impure_tests")]
    #[flaky_test(tokio, times = 5)]
    async fn lua_installation_bin() {
        let config = crate::config::ConfigBuilder::new()
            .unwrap()
            .build()
            .unwrap();
        let lua_version = config.lua_version().unwrap();
        let lua_installation = Installation::new(lua_version, &config).await.unwrap();
        // FIXME: This fails when run in the nix checkPhase
        assert!(lua_installation.bin.is_some());
        let lua_binary: Binary = lua_installation.bin.unwrap().into();
        let lua_bin_path: PathBuf = lua_binary.try_into().unwrap();
        let pkg_version = detect::version_from_path(&lua_bin_path).unwrap();
        assert_eq!(&Version::from_version(pkg_version).unwrap(), lua_version);
    }

    #[cfg(not(target_env = "msvc"))]
    #[tokio::test]
    async fn test_is_lib_name() {
        assert!(is_lib_name("lua.a", &Version::Lua51));
        assert!(is_lib_name("lua-5.1.a", &Version::Lua51));
        assert!(is_lib_name("lua5.1.a", &Version::Lua51));
        assert!(is_lib_name("lua51.a", &Version::Lua51));
        assert!(!is_lib_name("lua-5.2.a", &Version::Lua51));
        assert!(is_lib_name("luajit-5.2.a", &Version::LuaJIT52));
        assert!(is_lib_name("lua-5.2.a", &Version::LuaJIT52));
        assert!(is_lib_name("liblua.a", &Version::Lua51));
        assert!(is_lib_name("liblua-5.1.a", &Version::Lua51));
        assert!(is_lib_name("liblua53.a", &Version::Lua53));
        assert!(is_lib_name("liblua-54.a", &Version::Lua54));
        assert!(is_lib_name("liblua-55.a", &Version::Lua55));
    }

    #[cfg(target_env = "msvc")]
    #[tokio::test]
    async fn test_is_lib_name() {
        assert!(is_lib_name("lua.lib", &Version::Lua51));
        assert!(is_lib_name("lua-5.1.lib", &Version::Lua51));
        assert!(!is_lib_name("lua-5.2.lib", &Version::Lua51));
        assert!(!is_lib_name("lua53.lib", &Version::Lua53));
        assert!(!is_lib_name("lua53.lib", &Version::Lua53));
        assert!(is_lib_name("luajit-5.2.lib", &Version::LuaJIT52));
        assert!(is_lib_name("lua-5.2.lib", &Version::LuaJIT52));
    }
}
