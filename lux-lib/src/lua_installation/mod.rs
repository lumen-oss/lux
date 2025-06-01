use is_executable::IsExecutable;
use itertools::Itertools;
use path_slash::PathBufExt;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use target_lexicon::Triple;
use thiserror::Error;
use tokio::process::Command;

use crate::build::external_dependency::to_lib_name;
use crate::build::external_dependency::ExternalDependencyInfo;
use crate::build::utils::{c_lib_extension, format_path};
use crate::lua_rockspec::ExternalDependencySpec;
use crate::operations::{LuaBinary, LuaBinaryError};
use crate::project::Project;
use crate::{
    config::{Config, LuaVersion},
    package::PackageVersion,
    variables::HasVariables,
};

pub struct LuaInstallation {
    pub version: LuaVersion,
    dependency_info: ExternalDependencyInfo,
    /// Binary to the Lua executable, if present
    bin: Option<PathBuf>,
}

impl LuaInstallation {
    pub fn new(version: &LuaVersion, config: &Config) -> Self {
        let pkg_name = match version {
            LuaVersion::Lua51 => "lua5.1",
            LuaVersion::Lua52 => "lua5.2",
            LuaVersion::Lua53 => "lua5.3",
            LuaVersion::Lua54 => "lua5.4",
            LuaVersion::LuaJIT | LuaVersion::LuaJIT52 => "luajit",
        };

        let mut dependency_info = ExternalDependencyInfo::probe(
            pkg_name,
            &ExternalDependencySpec::default(),
            config.external_deps(),
        );

        if let Ok(info) = &mut dependency_info {
            let bin = info.lib_dir.as_ref().and_then(|lib_dir| {
                lib_dir
                    .parent()
                    .map(|parent| parent.join("bin"))
                    .filter(|dir| dir.is_dir())
                    .and_then(|bin_path| find_lua_executable(&bin_path))
            });
            let lua_lib_name = info
                .lib_dir
                .as_ref()
                .and_then(|lib_dir| get_lua_lib_name(lib_dir, version));
            info.lib_name = lua_lib_name;
            return Self {
                version: version.clone(),
                dependency_info: dependency_info.unwrap(),
                bin,
            };
        }

        let output = Self::root_dir(version, config);
        if output.join("include").is_dir() {
            let bin_dir = Some(output.join("bin")).filter(|bin_path| bin_path.is_dir());
            let bin = bin_dir
                .as_ref()
                .and_then(|bin_path| find_lua_executable(bin_path));
            let lib_dir = output.join("lib");
            let lua_lib_name = get_lua_lib_name(&lib_dir, version);
            let include_dir = Some(output.join("include"));
            LuaInstallation {
                version: version.clone(),
                dependency_info: ExternalDependencyInfo {
                    include_dir,
                    lib_dir: Some(lib_dir),
                    bin_dir,
                    lib_info: None,
                    lib_name: lua_lib_name,
                },
                bin,
            }
        } else {
            Self::install(version, config)
        }
    }

    pub fn install(version: &LuaVersion, config: &Config) -> Self {
        let host = Triple::host();
        let target = &host.to_string();
        let host_operating_system = &host.operating_system.to_string();

        let output = Self::root_dir(version, config);
        let (include_dir, lib_dir) = match version {
            LuaVersion::LuaJIT | LuaVersion::LuaJIT52 => {
                // XXX: luajit_src panics if this is not set.
                let target_pointer_width =
                    std::env::var("CARGO_CFG_TARGET_POINTER_WIDTH").unwrap_or("64".into());
                std::env::set_var("CARGO_CFG_TARGET_POINTER_WIDTH", target_pointer_width);
                let build = luajit_src::Build::new()
                    .target(target)
                    .host(host_operating_system)
                    .out_dir(&output)
                    .lua52compat(matches!(version, LuaVersion::LuaJIT52))
                    .build();

                (
                    build.include_dir().to_path_buf(),
                    build.lib_dir().to_path_buf(),
                )
            }
            _ => {
                let build = lua_src::Build::new()
                    .target(target)
                    .host(host_operating_system)
                    .out_dir(&output)
                    .build(match version {
                        LuaVersion::Lua51 => lua_src::Version::Lua51,
                        LuaVersion::Lua52 => lua_src::Version::Lua52,
                        LuaVersion::Lua53 => lua_src::Version::Lua53,
                        LuaVersion::Lua54 => lua_src::Version::Lua54,
                        _ => unreachable!(),
                    });

                (
                    build.include_dir().to_path_buf(),
                    build.lib_dir().to_path_buf(),
                )
            }
        };

        let bin_dir = Some(output.join("bin")).filter(|bin_path| bin_path.is_dir());
        let bin = bin_dir
            .as_ref()
            .and_then(|bin_path| find_lua_executable(bin_path));
        let lua_lib_name = get_lua_lib_name(&lib_dir, version);
        LuaInstallation {
            version: version.clone(),
            dependency_info: ExternalDependencyInfo {
                include_dir: Some(include_dir),
                lib_dir: Some(lib_dir),
                bin_dir,
                lib_info: None,
                lib_name: lua_lib_name,
            },
            bin,
        }
    }

    pub fn includes(&self) -> Vec<&PathBuf> {
        self.dependency_info.include_dir.iter().collect_vec()
    }

    fn root_dir(version: &LuaVersion, config: &Config) -> PathBuf {
        if let Some(lua_dir) = config.lua_dir() {
            return lua_dir.clone();
        } else if let Ok(Some(project)) = Project::current() {
            if let Ok(tree) = project.tree(config) {
                return tree.root().join(".lua");
            }
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
        self.dependency_info.lib_link_args(compiler)
    }

    /// Get the Lua binary (if present), prioritising
    /// a potentially overridden value in the config.
    pub(crate) fn lua_binary(&self, config: &Config) -> Option<String> {
        config.variables().get("LUA").cloned().or(self
            .bin
            .clone()
            .or(LuaBinary::default().try_into().ok())
            .map(|bin| bin.to_slash_lossy().to_string()))
    }
}

impl HasVariables for LuaInstallation {
    fn get_variable(&self, input: &str) -> Option<String> {
        let result = match input {
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
                .or(LuaBinary::default().try_into().ok())
                .map(|lua| format_path(&lua)),
            "LUALIB" => self.lua_lib().or(Some("".into())),
            _ => None,
        }?;
        Some(result)
    }
}

#[derive(Error, Debug)]
pub enum DetectLuaVersionError {
    #[error("error detecting Lua version: {0}")]
    LuaBinary(#[from] LuaBinaryError),
    #[error("failed to run {0}: {1}")]
    RunLuaCommand(String, io::Error),
    #[error("failed to parse Lua version from output: {0}")]
    ParseLuaVersion(String),
    #[error(transparent)]
    PackageVersionParse(#[from] crate::package::PackageVersionParseError),
    #[error(transparent)]
    LuaVersion(#[from] crate::config::LuaVersionError),
}

pub async fn detect_installed_lua_version(
    lua: LuaBinary,
) -> Result<PackageVersion, DetectLuaVersionError> {
    let lua_cmd: PathBuf = lua.try_into()?;
    let output = match Command::new(&lua_cmd).arg("-v").output().await {
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

pub fn detect_installed_lua_version_sync(
    lua: LuaBinary,
) -> Result<PackageVersion, DetectLuaVersionError> {
    let lua_cmd: PathBuf = lua.try_into()?;
    let output = match std::process::Command::new(&lua_cmd).arg("-v").output() {
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

fn find_lua_executable(bin_path: &Path) -> Option<PathBuf> {
    std::fs::read_dir(bin_path).ok().and_then(|entries| {
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path().to_path_buf())
            .filter(|file| {
                file.is_executable()
                    && file.file_name().is_some_and(|name| {
                        matches!(
                            name.to_string_lossy().to_string().as_str(),
                            "lua" | "luajit"
                        )
                    })
            })
            .collect_vec()
            .first()
            .cloned()
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

#[cfg(test)]
mod test {
    use crate::config::ConfigBuilder;

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
        let lua_installation = LuaInstallation::new(lua_version, &config);
        // FIXME: This fails when run in the nix checkPhase
        assert!(lua_installation.bin.is_some());
        let pkg_version = detect_installed_lua_version(lua_installation.bin.unwrap().into())
            .await
            .unwrap();
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
