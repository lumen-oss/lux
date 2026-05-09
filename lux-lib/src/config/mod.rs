use directories::ProjectDirs;
use external_deps::ExternalDependencySearchConfig;
use itertools::Itertools;

use serde::{Deserialize, Serialize, Serializer};
use std::{collections::HashMap, env, io, path::PathBuf, time::Duration};
use thiserror::Error;
use tree::RockLayoutConfig;
use url::Url;

use crate::lua_version::LuaVersion;
use crate::tree::{Tree, TreeError};
use crate::variables::GetVariableError;
use crate::{build::utils, variables::HasVariables};

pub mod external_deps;
pub mod tree;

const DEV_PATH: &str = "dev/";
const DEFAULT_USER_AGENT: &str = concat!("lux-lib/", env!("CARGO_PKG_VERSION"));

#[derive(Error, Debug)]
#[error("could not find a valid home directory")]
pub struct NoValidHomeDirectory;

#[derive(Debug, Clone)]
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
    /// Skip prompts (choosing the default choice)
    no_prompt: bool,
    timeout: Duration,
    max_jobs: usize,
    variables: HashMap<String, String>,
    external_deps: ExternalDependencySearchConfig,
    /// The rock layout for entrypoints of new install trees.
    /// Does not affect existing install trees or dependency rock layouts.
    entrypoint_layout: RockLayoutConfig,

    cache_dir: PathBuf,
    data_dir: PathBuf,
    vendor_dir: Option<PathBuf>,

    /// The user agent to set when making web requests.
    /// Default: "lux-lib/<version>".
    user_agent: String,

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

    // TODO(vhyrro): Remove `LuaVersion::from(&config)` and keep this only.
    pub fn lua_version(&self) -> Option<&LuaVersion> {
        self.lua_version.as_ref()
    }

    /// The tree in which to install rocks.
    /// If installing packges for a project, use `Project::tree` instead.
    pub fn user_tree(&self, version: LuaVersion) -> Result<Tree, TreeError> {
        Tree::new(self.user_tree.clone(), version, self)
    }

    pub fn verbose(&self) -> bool {
        self.verbose
    }

    pub fn no_progress(&self) -> bool {
        self.no_progress
    }

    pub fn no_prompt(&self) -> bool {
        self.no_prompt
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

    pub fn vendor_dir(&self) -> Option<&PathBuf> {
        self.vendor_dir.as_ref()
    }

    pub fn user_agent(&self) -> &str {
        &self.user_agent
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
    vendor_dir: Option<PathBuf>,
    enable_development_packages: Option<bool>,
    verbose: Option<bool>,
    no_progress: Option<bool>,
    no_prompt: Option<bool>,
    timeout: Option<Duration>,
    max_jobs: Option<usize>,
    variables: Option<HashMap<String, String>>,
    #[serde(default)]
    external_deps: ExternalDependencySearchConfig,
    /// The rock layout for new install trees.
    /// Does not affect existing install trees.
    #[serde(default)]
    entrypoint_layout: RockLayoutConfig,
    user_agent: Option<String>,
    generate_luarc: Option<bool>,
}

/// A builder for the lux `Config`.
impl ConfigBuilder {
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

    pub fn no_prompt(self, no_prompt: Option<bool>) -> Self {
        Self {
            no_prompt: no_prompt.or(self.no_prompt),
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

    pub fn vendor_dir(self, vendor_dir: Option<PathBuf>) -> Self {
        Self {
            vendor_dir: vendor_dir.or(self.vendor_dir),
            ..self
        }
    }

    pub fn entrypoint_layout(self, rock_layout: RockLayoutConfig) -> Self {
        Self {
            entrypoint_layout: rock_layout,
            ..self
        }
    }

    pub fn user_agent(self, user_agent: Option<String>) -> Self {
        Self {
            user_agent: user_agent.or(self.user_agent),
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

        let lua_version = self
            .lua_version
            .or(crate::lua_installation::detect_installed_lua_version());

        Ok(Config {
            enable_development_packages: self.enable_development_packages.unwrap_or(false),
            server: self.server.unwrap_or_else(|| unsafe {
                Url::parse("https://luarocks.org/").unwrap_unchecked()
            }),
            extra_servers: self.extra_servers.unwrap_or_default(),
            only_sources: self.only_sources,
            namespace: self.namespace,
            lua_dir: self.lua_dir,
            lua_version,
            user_tree,
            verbose: self.verbose.unwrap_or(false),
            no_progress: self.no_progress.unwrap_or(false),
            no_prompt: self.no_prompt.unwrap_or(false),
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
            vendor_dir: self.vendor_dir,
            user_agent: self.user_agent.unwrap_or(DEFAULT_USER_AGENT.into()),
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
            no_prompt: Some(value.no_prompt),
            timeout: Some(value.timeout),
            max_jobs: if value.max_jobs == usize::MAX {
                None
            } else {
                Some(value.max_jobs)
            },
            variables: Some(value.variables),
            cache_dir: Some(value.cache_dir),
            data_dir: Some(value.data_dir),
            vendor_dir: value.vendor_dir,
            external_deps: value.external_deps,
            entrypoint_layout: value.entrypoint_layout,
            user_agent: Some(value.user_agent),
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
