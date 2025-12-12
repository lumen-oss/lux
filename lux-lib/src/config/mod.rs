use directories::ProjectDirs;
use external_deps::ExternalDependencySearchConfig;
use itertools::Itertools;
use mlua::{ExternalError, ExternalResult, FromLua, IntoLua, UserData};
use serde::{Deserialize, Serialize, Serializer};
use std::env::current_exe;
use std::path::Path;
use std::{
    collections::HashMap, env, fmt::Display, io, path::PathBuf, str::FromStr, time::Duration,
};
use thiserror::Error;
use tree::RockLayoutConfig;
use url::Url;

use crate::project::Project;
use crate::tree::{Tree, TreeError};
use crate::variables::GetVariableError;
use crate::{
    build::utils,
    package::{PackageVersion, PackageVersionReq},
    variables::HasVariables,
};

pub mod external_deps;
pub mod tree;

const DEV_PATH: &str = "dev/";

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
    #[serde(rename = "jit")]
    LuaJIT,
    #[serde(rename = "jit5.2")]
    LuaJIT52,
    // TODO(vhyrro): Support luau?
    // LuaU,
}

impl FromLua for LuaVersion {
    fn from_lua(value: mlua::Value, lua: &mlua::Lua) -> mlua::Result<Self> {
        let version_str: String = FromLua::from_lua(value, lua)?;
        LuaVersion::from_str(&version_str).into_lua_err()
    }
}

impl IntoLua for LuaVersion {
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        self.to_string().into_lua(lua)
    }
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

/// Searches for the lux-lib directory in a binary distribution's resources
fn lux_lib_resource_dir() -> Option<PathBuf> {
    if cfg!(target_env = "msvc") {
        // The msvc .exe and .msi binary installers install lux-lua to the executable's directory.
        current_exe()
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
        current_exe()
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

#[derive(Error, Debug)]
#[error(
    r#"lua version couldn't be inferred!
Please provide a version through `lx --lua-version <ver> <cmd>`
Valid versions are: '5.1', '5.2', '5.3', '5.4', 'jit' and 'jit52'.
"#
)]
pub struct LuaVersionUnset;

impl LuaVersion {
    pub fn from_detected() -> Result<Self, LuaVersionUnset> {
        crate::lua_installation::detect_installed_lua_version().ok_or(LuaVersionUnset)
    }

    pub fn from_config(config: &Config) -> Result<Self, LuaVersionUnset> {
        config
            .lua_version
            .clone()
            .or_else(crate::lua_installation::detect_installed_lua_version)
            .ok_or(LuaVersionUnset)
    }

    pub fn from_current_project_or_config(config: &Config) -> Result<Self, LuaVersionUnset> {
        match config.lua_version.clone() {
            Some(lua_version) => Ok(lua_version),
            None => match Project::current_or_err() {
                Ok(project) => project.lua_version(config).map_err(|_| LuaVersionUnset),
                Err(_) => LuaVersion::from_detected(),
            },
        }
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
            "jit" | "luajit" => Ok(LuaVersion::LuaJIT),
            "jit52" | "luajit52" => Ok(LuaVersion::LuaJIT52),
            _ => Err(
                "unrecognized Lua version. Allowed versions: '5.1', '5.2', '5.3', '5.4', 'jit', 'jit52'."
                    .into(),
            ),
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
            LuaVersion::LuaJIT => "jit",
            LuaVersion::LuaJIT52 => "jit52",
        })
    }
}

#[derive(Error, Debug)]
#[error("could not find a valid home directory")]
pub struct NoValidHomeDirectory;

#[derive(Debug, Clone, FromLua)]
pub struct Config {
    enable_development_packages: bool,
    server: Url,
    extra_servers: Vec<Url>,
    only_sources: Option<String>,
    namespace: Option<String>,
    lua_dir: Option<PathBuf>,
    lua_version: Option<LuaVersion>,
    user_tree: PathBuf,
    verbose: bool,
    /// Don't display progress bars
    no_progress: bool,
    timeout: Duration,
    max_jobs: usize,
    variables: HashMap<String, String>,
    external_deps: ExternalDependencySearchConfig,
    /// The rock layout for entrypoints of new install trees.
    /// Does not affect existing install trees or dependency rock layouts.
    entrypoint_layout: RockLayoutConfig,

    cache_dir: PathBuf,
    data_dir: PathBuf,
    generate_luarc: bool,
}

impl Config {
    pub fn get_project_dirs() -> Result<ProjectDirs, NoValidHomeDirectory> {
        directories::ProjectDirs::from("org", "lumenlabs", "lux").ok_or(NoValidHomeDirectory)
    }

    pub fn get_default_cache_path() -> Result<PathBuf, NoValidHomeDirectory> {
        let project_dirs = Config::get_project_dirs()?;
        Ok(project_dirs.cache_dir().to_path_buf())
    }

    pub fn get_default_data_path() -> Result<PathBuf, NoValidHomeDirectory> {
        let project_dirs = Config::get_project_dirs()?;
        Ok(project_dirs.data_local_dir().to_path_buf())
    }

    pub fn with_lua_version(self, lua_version: LuaVersion) -> Self {
        Self {
            lua_version: Some(lua_version),
            ..self
        }
    }

    pub fn with_tree(self, tree: PathBuf) -> Self {
        Self {
            user_tree: tree,
            ..self
        }
    }

    pub fn server(&self) -> &Url {
        &self.server
    }

    pub fn extra_servers(&self) -> &Vec<Url> {
        self.extra_servers.as_ref()
    }

    pub fn enabled_dev_servers(&self) -> Result<Vec<Url>, ConfigError> {
        let mut enabled_dev_servers = Vec::new();
        if self.enable_development_packages {
            enabled_dev_servers.push(self.server().join(DEV_PATH)?);
            for server in self.extra_servers() {
                enabled_dev_servers.push(server.join(DEV_PATH)?);
            }
        }
        Ok(enabled_dev_servers)
    }

    pub fn only_sources(&self) -> Option<&String> {
        self.only_sources.as_ref()
    }

    pub fn namespace(&self) -> Option<&String> {
        self.namespace.as_ref()
    }

    pub fn lua_dir(&self) -> Option<&PathBuf> {
        self.lua_dir.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn lua_version(&self) -> Option<&LuaVersion> {
        self.lua_version.as_ref()
    }

    /// The tree in which to install rocks.
    /// If installing packages for a project, use `Project::tree` instead.
    pub fn user_tree(&self, version: LuaVersion) -> Result<Tree, TreeError> {
        Tree::new(self.user_tree.clone(), version, self)
    }

    pub fn verbose(&self) -> bool {
        self.verbose
    }

    pub fn no_progress(&self) -> bool {
        self.no_progress
    }

    pub fn timeout(&self) -> &Duration {
        &self.timeout
    }

    pub fn max_jobs(&self) -> usize {
        self.max_jobs
    }

    pub fn make_cmd(&self) -> String {
        match self.variables.get("MAKE") {
            Some(make) => make.clone(),
            None => "make".into(),
        }
    }

    pub fn cmake_cmd(&self) -> String {
        match self.variables.get("CMAKE") {
            Some(cmake) => cmake.clone(),
            None => "cmake".into(),
        }
    }

    pub fn variables(&self) -> &HashMap<String, String> {
        &self.variables
    }

    pub fn external_deps(&self) -> &ExternalDependencySearchConfig {
        &self.external_deps
    }

    pub fn entrypoint_layout(&self) -> &RockLayoutConfig {
        &self.entrypoint_layout
    }

    pub fn cache_dir(&self) -> &PathBuf {
        &self.cache_dir
    }

    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    pub fn generate_luarc(&self) -> bool {
        self.generate_luarc
    }
}

impl HasVariables for Config {
    fn get_variable(&self, input: &str) -> Result<Option<String>, GetVariableError> {
        Ok(self.variables.get(input).cloned())
    }
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    NoValidHomeDirectory(#[from] NoValidHomeDirectory),
    #[error("error deserializing lux config: {0}")]
    Deserialize(#[from] toml::de::Error),
    #[error("error parsing URL: {0}")]
    UrlParseError(#[from] url::ParseError),
    #[error("error initializing compiler toolchain: {0}")]
    CompilerToolchain(#[from] cc::Error),
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct ConfigBuilder {
    #[serde(
        default,
        deserialize_with = "deserialize_url",
        serialize_with = "serialize_url"
    )]
    server: Option<Url>,
    #[serde(
        default,
        deserialize_with = "deserialize_url_vec",
        serialize_with = "serialize_url_vec"
    )]
    extra_servers: Option<Vec<Url>>,
    only_sources: Option<String>,
    namespace: Option<String>,
    lua_version: Option<LuaVersion>,
    user_tree: Option<PathBuf>,
    lua_dir: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    enable_development_packages: Option<bool>,
    verbose: Option<bool>,
    no_progress: Option<bool>,
    timeout: Option<Duration>,
    max_jobs: Option<usize>,
    variables: Option<HashMap<String, String>>,
    #[serde(default)]
    external_deps: ExternalDependencySearchConfig,
    /// The rock layout for new install trees.
    /// Does not affect existing install trees.
    #[serde(default)]
    entrypoint_layout: RockLayoutConfig,
    generate_luarc: Option<bool>,
}

/// A builder for the lux `Config`.
impl ConfigBuilder {
    #[cfg(not(test))]
    /// Create a new `ConfigBuilder` from a config file by deserializing from a config file
    /// if present, or otherwise by instantiating the default config.
    pub fn new() -> Result<Self, ConfigError> {
        let config_file = Self::config_file()?;
        if config_file.is_file() {
            Ok(toml::from_str(&std::fs::read_to_string(&config_file)?)?)
        } else {
            Ok(Self::default())
        }
    }

    #[cfg(test)]
    /// Create a new `ConfigBuilder` without reading from a config file.
    pub fn new() -> Result<Self, std::convert::Infallible> {
        Ok(Self::default())
    }

    /// Get the path to the lux config file.
    pub fn config_file() -> Result<PathBuf, NoValidHomeDirectory> {
        let project_dirs = directories::ProjectDirs::from("org", "lumenlabs", "lux")
            .ok_or(NoValidHomeDirectory)?;
        Ok(project_dirs.config_dir().join("config.toml").to_path_buf())
    }

    pub fn dev(self, dev: Option<bool>) -> Self {
        Self {
            enable_development_packages: dev.or(self.enable_development_packages),
            ..self
        }
    }

    pub fn server(self, server: Option<Url>) -> Self {
        Self {
            server: server.or(self.server),
            ..self
        }
    }

    pub fn extra_servers(self, extra_servers: Option<Vec<Url>>) -> Self {
        Self {
            extra_servers: extra_servers.or(self.extra_servers),
            ..self
        }
    }

    pub fn only_sources(self, sources: Option<String>) -> Self {
        Self {
            only_sources: sources.or(self.only_sources),
            ..self
        }
    }

    pub fn namespace(self, namespace: Option<String>) -> Self {
        Self {
            namespace: namespace.or(self.namespace),
            ..self
        }
    }

    pub fn lua_dir(self, lua_dir: Option<PathBuf>) -> Self {
        Self {
            lua_dir: lua_dir.or(self.lua_dir),
            ..self
        }
    }

    pub fn lua_version(self, lua_version: Option<LuaVersion>) -> Self {
        Self {
            lua_version: lua_version.or(self.lua_version),
            ..self
        }
    }

    pub fn user_tree(self, tree: Option<PathBuf>) -> Self {
        Self {
            user_tree: tree.or(self.user_tree),
            ..self
        }
    }

    pub fn variables(self, variables: Option<HashMap<String, String>>) -> Self {
        Self {
            variables: variables.or(self.variables),
            ..self
        }
    }

    pub fn verbose(self, verbose: Option<bool>) -> Self {
        Self {
            verbose: verbose.or(self.verbose),
            ..self
        }
    }

    pub fn no_progress(self, no_progress: Option<bool>) -> Self {
        Self {
            no_progress: no_progress.or(self.no_progress),
            ..self
        }
    }

    pub fn timeout(self, timeout: Option<Duration>) -> Self {
        Self {
            timeout: timeout.or(self.timeout),
            ..self
        }
    }

    pub fn max_jobs(self, max_jobs: Option<usize>) -> Self {
        Self {
            max_jobs: max_jobs.or(self.max_jobs),
            ..self
        }
    }

    pub fn cache_dir(self, cache_dir: Option<PathBuf>) -> Self {
        Self {
            cache_dir: cache_dir.or(self.cache_dir),
            ..self
        }
    }

    pub fn data_dir(self, data_dir: Option<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.or(self.data_dir),
            ..self
        }
    }

    pub fn entrypoint_layout(self, rock_layout: RockLayoutConfig) -> Self {
        Self {
            entrypoint_layout: rock_layout,
            ..self
        }
    }

    pub fn generate_luarc(self, generate: Option<bool>) -> Self {
        Self {
            generate_luarc: generate.or(self.generate_luarc),
            ..self
        }
    }

    pub fn build(self) -> Result<Config, ConfigError> {
        let data_dir = self.data_dir.unwrap_or(Config::get_default_data_path()?);
        let cache_dir = self.cache_dir.unwrap_or(Config::get_default_cache_path()?);
        let user_tree = self.user_tree.unwrap_or(data_dir.join("tree"));

        Ok(Config {
            enable_development_packages: self.enable_development_packages.unwrap_or(false),
            server: self.server.unwrap_or_else(|| unsafe {
                Url::parse("https://luarocks.org/").unwrap_unchecked()
            }),
            extra_servers: self.extra_servers.unwrap_or_default(),
            only_sources: self.only_sources,
            namespace: self.namespace,
            lua_dir: self.lua_dir,
            lua_version: self.lua_version,
            user_tree,
            verbose: self.verbose.unwrap_or(false),
            no_progress: self.no_progress.unwrap_or(false),
            timeout: self.timeout.unwrap_or_else(|| Duration::from_secs(30)),
            max_jobs: match self.max_jobs.unwrap_or(usize::MAX) {
                0 => usize::MAX,
                max_jobs => max_jobs,
            },
            variables: default_variables()
                .chain(self.variables.unwrap_or_default())
                .collect(),
            external_deps: self.external_deps,
            entrypoint_layout: self.entrypoint_layout,
            cache_dir,
            data_dir,
            generate_luarc: self.generate_luarc.unwrap_or(true),
        })
    }
}

/// Useful for printing the current config
impl From<Config> for ConfigBuilder {
    fn from(value: Config) -> Self {
        ConfigBuilder {
            enable_development_packages: Some(value.enable_development_packages),
            server: Some(value.server),
            extra_servers: Some(value.extra_servers),
            only_sources: value.only_sources,
            namespace: value.namespace,
            lua_dir: value.lua_dir,
            lua_version: value.lua_version,
            user_tree: Some(value.user_tree),
            verbose: Some(value.verbose),
            no_progress: Some(value.no_progress),
            timeout: Some(value.timeout),
            max_jobs: if value.max_jobs == usize::MAX {
                None
            } else {
                Some(value.max_jobs)
            },
            variables: Some(value.variables),
            cache_dir: Some(value.cache_dir),
            data_dir: Some(value.data_dir),
            external_deps: value.external_deps,
            entrypoint_layout: value.entrypoint_layout,
            generate_luarc: Some(value.generate_luarc),
        }
    }
}

fn default_variables() -> impl Iterator<Item = (String, String)> {
    let cflags = env::var("CFLAGS").unwrap_or(utils::default_cflags().into());
    let ldflags = env::var("LDFLAGS").unwrap_or("".into());
    vec![
        ("MAKE".into(), "make".into()),
        ("CMAKE".into(), "cmake".into()),
        ("LIB_EXTENSION".into(), utils::c_dylib_extension().into()),
        ("OBJ_EXTENSION".into(), utils::c_obj_extension().into()),
        ("CFLAGS".into(), cflags),
        ("LDFLAGS".into(), ldflags),
        ("LIBFLAG".into(), utils::default_libflag().into()),
    ]
    .into_iter()
}

fn deserialize_url<'de, D>(deserializer: D) -> Result<Option<Url>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = Option::<String>::deserialize(deserializer)?;
    s.map(|s| Url::parse(&s).map_err(serde::de::Error::custom))
        .transpose()
}

fn serialize_url<S>(url: &Option<Url>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match url {
        Some(url) => serializer.serialize_some(url.as_str()),
        None => serializer.serialize_none(),
    }
}

fn deserialize_url_vec<'de, D>(deserializer: D) -> Result<Option<Vec<Url>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = Option::<Vec<String>>::deserialize(deserializer)?;
    s.map(|v| {
        v.into_iter()
            .map(|s| Url::parse(&s).map_err(serde::de::Error::custom))
            .try_collect()
    })
    .transpose()
}

fn serialize_url_vec<S>(urls: &Option<Vec<Url>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match urls {
        Some(urls) => {
            let url_strings: Vec<String> = urls.iter().map(|url| url.to_string()).collect();
            serializer.serialize_some(&url_strings)
        }
        None => serializer.serialize_none(),
    }
}

struct LuaUrl(Url);

impl FromLua for LuaUrl {
    fn from_lua(value: mlua::Value, lua: &mlua::Lua) -> mlua::Result<Self> {
        let url_str: String = FromLua::from_lua(value, lua)?;

        Url::parse(&url_str).map(LuaUrl).into_lua_err()
    }
}

impl UserData for Config {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_function("default", |_, _: ()| {
            ConfigBuilder::default()
                .build()
                .map_err(|err| err.into_lua_err())
        });

        methods.add_function("builder", |_, ()| ConfigBuilder::new().into_lua_err());

        methods.add_method("server", |_, this, ()| Ok(this.server().to_string()));
        methods.add_method("extra_servers", |_, this, ()| {
            Ok(this
                .extra_servers()
                .iter()
                .map(|url| url.to_string())
                .collect_vec())
        });
        methods.add_method("only_sources", |_, this, ()| {
            Ok(this.only_sources().cloned())
        });
        methods.add_method("namespace", |_, this, ()| Ok(this.namespace().cloned()));
        methods.add_method("lua_dir", |_, this, ()| Ok(this.lua_dir().cloned()));
        methods.add_method("user_tree", |_, this, lua_version: LuaVersion| {
            this.user_tree(lua_version).into_lua_err()
        });
        methods.add_method("verbose", |_, this, ()| Ok(this.verbose()));
        methods.add_method("no_progress", |_, this, ()| Ok(this.no_progress()));
        methods.add_method("timeout", |_, this, ()| Ok(this.timeout().as_secs()));
        methods.add_method("cache_dir", |_, this, ()| Ok(this.cache_dir().clone()));
        methods.add_method("data_dir", |_, this, ()| Ok(this.data_dir().clone()));
        methods.add_method("entrypoint_layout", |_, this, ()| {
            Ok(this.entrypoint_layout().clone())
        });
        methods.add_method("variables", |_, this, ()| Ok(this.variables().clone()));
        // FIXME: This is a temporary workaround to get the external_deps hooked up to Lua
        // methods.add_method("external_deps", |_, this, ()| {
        //     Ok(this.external_deps().clone())
        // });
        methods.add_method("make_cmd", |_, this, ()| Ok(this.make_cmd()));
        methods.add_method("cmake_cmd", |_, this, ()| Ok(this.cmake_cmd()));
        methods.add_method("enabled_dev_servers", |_, this, ()| {
            Ok(this
                .enabled_dev_servers()
                .into_lua_err()?
                .into_iter()
                .map(|url| url.to_string())
                .collect_vec())
        });
    }
}

impl UserData for ConfigBuilder {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("dev", |_, this, dev: Option<bool>| {
            Ok(this.clone().dev(dev))
        });
        methods.add_method("server", |_, this, server: Option<LuaUrl>| {
            Ok(this.clone().server(server.map(|url| url.0)))
        });
        methods.add_method("extra_servers", |_, this, servers: Option<Vec<LuaUrl>>| {
            Ok(this
                .clone()
                .extra_servers(servers.map(|urls| urls.into_iter().map(|url| url.0).collect())))
        });
        methods.add_method("only_sources", |_, this, sources: Option<String>| {
            Ok(this.clone().only_sources(sources))
        });
        methods.add_method("namespace", |_, this, namespace: Option<String>| {
            Ok(this.clone().namespace(namespace))
        });
        methods.add_method("lua_dir", |_, this, lua_dir: Option<PathBuf>| {
            Ok(this.clone().lua_dir(lua_dir))
        });
        methods.add_method("lua_version", |_, this, lua_version: Option<LuaVersion>| {
            Ok(this.clone().lua_version(lua_version))
        });
        methods.add_method("user_tree", |_, this, tree: Option<PathBuf>| {
            Ok(this.clone().user_tree(tree))
        });
        methods.add_method("verbose", |_, this, verbose: Option<bool>| {
            Ok(this.clone().verbose(verbose))
        });
        methods.add_method("no_progress", |_, this, no_progress: Option<bool>| {
            Ok(this.clone().no_progress(no_progress))
        });
        methods.add_method("timeout", |_, this, timeout: Option<u64>| {
            Ok(this.clone().timeout(timeout.map(Duration::from_secs)))
        });
        methods.add_method("cache_dir", |_, this, cache_dir: Option<PathBuf>| {
            Ok(this.clone().cache_dir(cache_dir))
        });
        methods.add_method("data_dir", |_, this, data_dir: Option<PathBuf>| {
            Ok(this.clone().data_dir(data_dir))
        });
        methods.add_method(
            "entrypoint_layout",
            |_, this, entrypoint_layout: Option<RockLayoutConfig>| {
                Ok(this
                    .clone()
                    .entrypoint_layout(entrypoint_layout.unwrap_or_default()))
            },
        );
        methods.add_method("generate_luarc", |_, this, generate: Option<bool>| {
            Ok(this.clone().generate_luarc(generate))
        });
        methods.add_method("build", |_, this, ()| this.clone().build().into_lua_err());
    }
}
