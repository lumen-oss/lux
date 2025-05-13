use is_executable::IsExecutable;
use itertools::Itertools;
use path_slash::{PathBufExt, PathExt};
use pkg_config::Library;
use std::io;
use std::path::Path;
use std::{path::PathBuf, process::Command};
use target_lexicon::Triple;
use thiserror::Error;

use crate::build::utils::{format_path, lua_lib_extension};
use crate::project::Project;
use crate::{
    build::variables::HasVariables,
    config::{Config, LuaVersion},
    package::PackageVersion,
};

pub struct LuaInstallation {
    pub include_dir: PathBuf,
    pub lib_dir: PathBuf,
    pub version: LuaVersion,
    /// Binary to the Lua executable, if present
    bin: Option<PathBuf>,
    /// pkg-config library information if available
    lib_info: Option<Library>,
    /// Name of the static Lua library, (without the 'lib' prefix or file extension on unix targets),
    /// for example, "lua" or "lua.dll"
    lua_lib_name: Option<String>,
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
        let lib_info = pkg_config::Config::new()
            .print_system_libs(false)
            .cargo_metadata(false)
            .env_metadata(false)
            .probe(pkg_name)
            .ok();

        if let Some(info) = lib_info {
            if !&info.include_paths.is_empty() && !&info.link_paths.is_empty() {
                let lib_dir = PathBuf::from(&info.link_paths[0]);
                let bin = lib_dir
                    .parent()
                    .map(|parent| parent.join("bin"))
                    .filter(|dir| dir.is_dir())
                    .and_then(|bin_path| find_lua_executable(&bin_path));
                let lua_lib_name = get_lua_lib_name(&lib_dir, version);
                return Self {
                    include_dir: PathBuf::from(&info.include_paths[0]),
                    lib_dir,
                    version: version.clone(),
                    lib_info: Some(info),
                    bin,
                    lua_lib_name,
                };
            }
        }

        let output = Self::root_dir(version, config);
        if output.join("include").is_dir() {
            let bin_path = output.join("bin");
            let bin = if bin_path.is_dir() {
                find_lua_executable(&bin_path)
            } else {
                None
            };
            let lib_dir = output.join("lib");
            let lua_lib_name = get_lua_lib_name(&lib_dir, version);
            LuaInstallation {
                include_dir: output.join("include"),
                lib_dir,
                version: version.clone(),
                lib_info: None,
                bin,
                lua_lib_name,
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

        let bin_path = output.join("bin");
        let bin = if bin_path.is_dir() {
            find_lua_executable(&bin_path)
        } else {
            None
        };
        let lua_lib_name = get_lua_lib_name(&lib_dir, version);
        LuaInstallation {
            include_dir,
            lib_dir,
            version: version.clone(),
            lib_info: None,
            bin,
            lua_lib_name,
        }
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
        self.lua_lib_name
            .as_ref()
            .map(|name| format!("{}.{}", name, lua_lib_extension()))
    }

    #[cfg(target_env = "msvc")]
    fn lua_lib(&self) -> Option<String> {
        self.lua_lib_name.clone()
    }

    pub(crate) fn define_flags(&self) -> Vec<String> {
        if let Some(info) = &self.lib_info {
            info.defines
                .iter()
                .map(|(k, v)| match v {
                    Some(val) => {
                        format!("-D{}={}", k, val)
                    }
                    None => format!("-D{}", k),
                })
                .collect_vec()
        } else {
            Vec::new()
        }
    }

    /// NOTE: In luarocks, these are behind a link_lua_explicity config option
    pub(crate) fn lib_link_args(&self, compiler: &cc::Tool) -> Vec<String> {
        if let Some(info) = &self.lib_info {
            info.link_paths
                .iter()
                .map(|p| lib_dir_compile_arg(p, compiler))
                .chain(
                    info.libs
                        .iter()
                        .map(|lib| format_lib_link_arg(lib, compiler)),
                )
                .chain(info.ld_args.iter().map(|ld_arg_group| {
                    ld_arg_group
                        .iter()
                        .map(|arg| format_linker_arg(arg, compiler))
                        .collect::<Vec<_>>()
                        .join(" ")
                }))
                .collect_vec()
        } else {
            std::iter::once(lib_dir_compile_arg(&self.lib_dir, compiler))
                .chain(
                    self.lua_lib_name
                        .as_ref()
                        .map(|lib_name| {
                            if compiler.is_like_msvc() {
                                self.lib_dir.join(lib_name).to_slash_lossy().to_string()
                            } else {
                                format!("-l{}", lib_name)
                            }
                        })
                        .iter()
                        .cloned(),
                )
                .collect_vec()
        }
    }

    /// Get the Lua binary (if present), prioritising
    /// a potentially overridden value in the config.
    pub(crate) fn lua_binary(&self, config: &Config) -> Option<String> {
        config.variables().get("LUA").cloned().or(self
            .bin
            .as_ref()
            .map(|bin| bin.clone().to_slash_lossy().to_string()))
    }
}

fn lib_dir_compile_arg(dir: &Path, compiler: &cc::Tool) -> String {
    if compiler.is_like_msvc() {
        format!("/LIBPATH:{}", dir.to_slash_lossy())
    } else {
        format!("-L{}", dir.to_slash_lossy())
    }
}

fn format_lib_link_arg(lib: &str, compiler: &cc::Tool) -> String {
    if compiler.is_like_msvc() {
        format!("{}.lib", lib)
    } else {
        format!("-l{}", lib)
    }
}

fn format_linker_arg(arg: &str, compiler: &cc::Tool) -> String {
    if compiler.is_like_msvc() {
        format!("-Wl,{}", arg)
    } else {
        format!("/link {}", arg)
    }
}

impl HasVariables for LuaInstallation {
    fn get_variable(&self, input: &str) -> Option<String> {
        let result = match input {
            "LUA_INCDIR" => Some(format_path(&self.include_dir)),
            "LUA_LIBDIR" => Some(format_path(&self.lib_dir)),
            "LUA" => Some(format_path(&self.bin.clone().unwrap_or("lua".into()))),
            "LUALIB" => self.lua_lib().or(Some("".into())),
            _ => None,
        }?;
        Some(result)
    }
}

#[derive(Error, Debug)]
pub enum GetLuaVersionError {
    #[error("failed to run {0}: {1}")]
    RunLuaCommandError(String, io::Error),
    #[error("failed to parse Lua version from output: {0}")]
    ParseLuaVersionError(String),
    #[error(transparent)]
    PackageVersionParseError(#[from] crate::package::PackageVersionParseError),
    #[error(transparent)]
    LuaVersionError(#[from] crate::config::LuaVersionError),
}

pub fn get_installed_lua_version(lua_cmd: &str) -> Result<PackageVersion, GetLuaVersionError> {
    let output = match Command::new(lua_cmd).arg("-v").output() {
        Ok(output) => Ok(output),
        Err(err) => Err(GetLuaVersionError::RunLuaCommandError(lua_cmd.into(), err)),
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

fn parse_lua_version_from_output(lua_output: &str) -> Result<PackageVersion, GetLuaVersionError> {
    let lua_version_str = lua_output
        .trim_start_matches("Lua")
        .trim_start_matches("JIT")
        .split_whitespace()
        .next()
        .map(|s| s.to_string())
        .ok_or(GetLuaVersionError::ParseLuaVersionError(
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
        .any(|prefix| name == format!("{}.{}", *prefix, lua_lib_extension()))
        || prefixes.iter().any(|prefix| name.starts_with(*prefix))
            && (name.contains(&version_str) || name.contains(&version_suffix))
}

fn to_lua_lib_name(file: &Path) -> String {
    let file_name = file.file_name().unwrap();
    if cfg!(target_family = "unix") {
        file_name
            .to_string_lossy()
            .trim_start_matches("lib")
            .trim_end_matches(".a")
            .to_string()
    } else {
        file_name.to_string_lossy().to_string()
    }
}

fn get_lua_lib_name(lib_dir: &Path, lua_version: &LuaVersion) -> Option<String> {
    std::fs::read_dir(lib_dir)
        .ok()
        .and_then(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path().to_path_buf())
                .filter(|file| {
                    file.extension()
                        .is_some_and(|ext| ext == lua_lib_extension())
                })
                .filter(|file| {
                    file.file_name()
                        .is_some_and(|name| is_lua_lib_name(&name.to_string_lossy(), lua_version))
                })
                .collect_vec()
                .first()
                .cloned()
        })
        .map(|file| to_lua_lib_name(&file))
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
        let pkg_version =
            get_installed_lua_version(&lua_installation.bin.unwrap().to_string_lossy()).unwrap();
        assert_eq!(&LuaVersion::from_version(pkg_version).unwrap(), lua_version);
    }

    #[cfg(not(target_env = "msvc"))]
    #[tokio::test]
    async fn test_to_lua_lib_name() {
        assert_eq!(to_lua_lib_name(&PathBuf::from("lua.a")), "lua".to_string());
        assert_eq!(
            to_lua_lib_name(&PathBuf::from("lua-5.1.a")),
            "lua-5.1".to_string()
        );
        assert_eq!(
            to_lua_lib_name(&PathBuf::from("lua5.1.a")),
            "lua5.1".to_string()
        );
        assert_eq!(
            to_lua_lib_name(&PathBuf::from("lua51.a")),
            "lua51".to_string()
        );
        assert_eq!(
            to_lua_lib_name(&PathBuf::from("luajit-5.2.a")),
            "luajit-5.2".to_string()
        );
        assert_eq!(
            to_lua_lib_name(&PathBuf::from("lua-5.2.a")),
            "lua-5.2".to_string()
        );
        assert_eq!(
            to_lua_lib_name(&PathBuf::from("liblua.a")),
            "lua".to_string()
        );
        assert_eq!(
            to_lua_lib_name(&PathBuf::from("liblua-5.1.a")),
            "lua-5.1".to_string()
        );
        assert_eq!(
            to_lua_lib_name(&PathBuf::from("liblua53.a")),
            "lua53".to_string()
        );
        assert_eq!(
            to_lua_lib_name(&PathBuf::from("liblua-54.a")),
            "lua-54".to_string()
        );
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
