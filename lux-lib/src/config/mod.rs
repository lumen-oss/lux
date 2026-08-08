use build::BuildConfig;
use directories::ProjectDirs;
use external_deps::ExternalDependencySearchConfig;
use itertools::Itertools;

use miette::Diagnostic;
use serde::{Deserialize, Serialize, Serializer};
use std::ffi::OsStr;
use std::path::Path;
use std::{collections::HashMap, env, path::PathBuf, time::Duration};
use thiserror::Error;
use tokio::process::Command;
use tree::RockLayoutConfig;
use url::Url;

use crate::fs;
use crate::lua_version::LuaVersion;
use crate::package::RemotePackageTypeFilterSpec;
use crate::project::TomlDeError;
use crate::tree::{Tree, TreeError};
use crate::variables::GetVariableError;
use crate::{build::utils, variables::HasVariables};

pub mod build;
pub mod external_deps;
pub mod tree;

const DEV_PATH: &str = "dev/";
const DEFAULT_USER_AGENT: &str = concat!("lux-lib/", env!("CARGO_PKG_VERSION"));

#[derive(Error, Debug, Diagnostic)]
#[error("could not find a valid home directory")]
#[diagnostic(
    code(lux_lib::no_home_directory),
    help("this usually means you're running Lux in a managed environment like LDAP or a live session.")
)]
pub struct NoValidHomeDirectory;

/// The resolved configuration for a Lux session.
/// Can be constructed via [`ConfigBuilder`], which supports layering multiple
/// configuration sources (config file, CLI flags, environment variables).
#[derive(Debug, Clone)]
pub struct Config {
    enable_development_packages: bool,
    server: Url,
    extra_servers: Vec<Url>,
    namespace: Option<String>,
    lua_dir: Option<PathBuf>,
    lua_version: Option<LuaVersion>,
    user_tree: PathBuf,
    workspace_tree: Option<PathBuf>,
    verbose: bool,
    /// Don't display progress bars
    no_progress: bool,
    /// Skip prompts (choosing the default choice)
    no_prompt: bool,
    timeout: Duration,
    max_jobs: usize,
    variables: HashMap<String, String>,
    external_deps: ExternalDependencySearchConfig,
    build: BuildConfig,
    entrypoint_layout: RockLayoutConfig,

    cache_dir: PathBuf,
    data_dir: PathBuf,
    vendor_dir: Option<PathBuf>,

    user_agent: String,

    generate_luarc: bool,
    luarc_file_name: String,
    wrap_bin_scripts: bool,
    package_types: RemotePackageTypeFilterSpec,
    no_tfa: bool,
}

impl Config {
    /// Lux application directories
    pub(crate) fn project_dirs() -> Result<ProjectDirs, NoValidHomeDirectory> {
        directories::ProjectDirs::from("org", "lumenlabs", "lux").ok_or(NoValidHomeDirectory)
    }

    /// Lux cache directory
    fn default_cache_path() -> Result<PathBuf, NoValidHomeDirectory> {
        let project_dirs = Config::project_dirs()?;
        Ok(project_dirs.cache_dir().to_path_buf())
    }

    /// Lux data directory
    fn default_data_path() -> Result<PathBuf, NoValidHomeDirectory> {
        let project_dirs = Config::project_dirs()?;
        Ok(project_dirs.data_local_dir().to_path_buf())
    }

    /// Create a copy of this config for the specified Lua version
    pub fn with_lua_version(self, lua_version: LuaVersion) -> Self {
        Self {
            lua_version: Some(lua_version),
            ..self
        }
    }

    /// Create a copy of this config with the specified install tree
    pub fn with_tree(self, tree: PathBuf) -> Self {
        Self {
            user_tree: tree,
            ..self
        }
    }

    /// Create a copy of this config with the specified workspace tree root
    pub fn with_workspace_tree(self, tree: Option<PathBuf>) -> Self {
        Self {
            workspace_tree: tree,
            ..self
        }
    }

    /// The luarocks repository server
    pub fn server(&self) -> &Url {
        &self.server
    }

    /// Additional luarocks repository servers
    pub fn extra_servers(&self) -> &Vec<Url> {
        self.extra_servers.as_ref()
    }

    /// Enabled luarocks repository servers that provide dev/scm rocks
    pub fn enabled_dev_servers(&self) -> Result<Vec<Url>, ConfigError> {
        let mut enabled_dev_servers = Vec::new();
        if self.enable_development_packages {
            let config_file = ConfigBuilder::config_file()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            enabled_dev_servers.push(self.server().join(DEV_PATH).map_err(|source| {
                ConfigError::UrlParseError {
                    source,
                    help: Some(format!("check the `server` URL in {config_file}")),
                }
            })?);
            for server in self.extra_servers() {
                enabled_dev_servers.push(server.join(DEV_PATH).map_err(|source| {
                    ConfigError::UrlParseError {
                        source,
                        help: Some(format!("check the `extra_servers` URLs in {config_file}")),
                    }
                })?);
            }
        }
        Ok(enabled_dev_servers)
    }

    /// The luarocks server namespace to use
    pub fn namespace(&self) -> Option<&String> {
        self.namespace.as_ref()
    }

    /// The directory in which to install Lua{n} if not found
    pub fn lua_dir(&self) -> Option<&PathBuf> {
        self.lua_dir.as_ref()
    }

    // TODO(vhyrro): Remove `LuaVersion::from(&config)` and keep this only.
    pub fn lua_version(&self) -> Option<&LuaVersion> {
        self.lua_version.as_ref()
    }

    /// The tree in which to install rocks.
    /// If installing packages for a project, use `Project::tree` instead.
    pub fn user_tree(&self, version: LuaVersion) -> Result<Tree, TreeError> {
        Tree::new(self.user_tree.clone(), version, self)
    }

    /// The detached workspace tree root, if set.
    pub fn workspace_tree(&self) -> Option<&PathBuf> {
        self.workspace_tree.as_ref()
    }

    /// Whether to display verbose output of commands executed
    pub fn verbose(&self) -> bool {
        self.verbose
    }

    /// Whether to disable printing progress bars and spinners
    pub fn no_progress(&self) -> bool {
        self.no_progress
    }

    /// Whether to skip prompts, selecting the default option
    pub fn no_prompt(&self) -> bool {
        self.no_prompt
    }

    /// Timeout on network operations, in seconds.
    /// 0 means no timeout (wait forever).
    pub fn timeout(&self) -> &Duration {
        &self.timeout
    }

    /// Maximum buffer size for parallel jobs, such as downloading rockspecs and installing rocks.
    /// 0 means no limit.
    pub fn max_jobs(&self) -> usize {
        self.max_jobs
    }

    /// Command to use for running `make` builds
    pub fn make_cmd(&self) -> String {
        match self.variables.get("MAKE") {
            Some(make) => make.clone(),
            None => "make".into(),
        }
    }

    /// Command to use for running `cmake` builds
    pub fn cmake_cmd(&self) -> String {
        match self.variables.get("CMAKE") {
            Some(cmake) => cmake.clone(),
            None => "cmake".into(),
        }
    }

    /// Construct a [`Command`] for the given program and arguments,
    /// wrapped in the configured [`BuildConfig::runner`], if any.
    ///
    /// If no runner is configured, this is equivalent to
    /// `Command::new(program).args(args)`.
    pub(crate) fn wrapped_command<P, A>(&self, program: P, args: A) -> Command
    where
        P: AsRef<OsStr>,
        A: IntoIterator,
        A::Item: AsRef<OsStr>,
    {
        if self.build.runner.is_empty() {
            let mut cmd = Command::new(program);
            cmd.args(args);
            cmd
        } else {
            let runner = &self.build.runner;
            let mut cmd = Command::new(&runner[0]);
            cmd.args(&runner[1..]).arg(program).args(args);
            cmd
        }
    }

    /// Variable names, mapped to their values.
    /// Lux populates variables in the `lux.toml` and in RockSpecs
    /// with these before building.
    pub fn variables(&self) -> &HashMap<String, String> {
        &self.variables
    }

    pub fn external_deps(&self) -> &ExternalDependencySearchConfig {
        &self.external_deps
    }

    /// The rock layout for entrypoints of new install trees.
    /// Does not affect existing install trees or dependency rock layouts.
    pub fn entrypoint_layout(&self) -> &RockLayoutConfig {
        &self.entrypoint_layout
    }

    /// The Lux cache directory
    pub fn cache_dir(&self) -> &PathBuf {
        &self.cache_dir
    }

    /// The Lux data directory
    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    /// Specifies a directory with locally vendored sources and RockSpecs.
    /// When building or installing a package with this flag,
    /// Lux will fetch sources from the <vendor-dir> instead of from a remote server.
    pub fn vendor_dir(&self) -> Option<&PathBuf> {
        self.vendor_dir.as_ref()
    }

    /// The user agent to use when making web requests.
    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    /// Whether to generate a `.luarc.json` on build.
    pub fn generate_luarc(&self) -> bool {
        self.generate_luarc
    }

    // Lua runtime configuration file name
    pub fn luarc_file_name(&self) -> &str {
        &self.luarc_file_name
    }

    /// Whether to wrap installed Lua bin scripts to be executed with
    /// the detected or configured Lua installation.
    /// If `true`, individual rocks can still disable wrapping of their own bin scripts.
    pub fn wrap_bin_scripts(&self) -> bool {
        self.wrap_bin_scripts
    }

    /// Filter specification for package types to include in searches.
    pub fn package_types(&self) -> &RemotePackageTypeFilterSpec {
        &self.package_types
    }
    /// Whether to disable prompts for two-factor authentication (2FA) codes.
    pub fn no_tfa(&self) -> bool {
        self.no_tfa
    }
}

impl HasVariables for Config {
    #[tracing::instrument(level = "trace")]
    fn get_variable(&self, input: &str) -> Result<Option<String>, GetVariableError> {
        Ok(self.variables.get(input).cloned())
    }
}

#[derive(Error, Debug, Diagnostic)]
pub enum ConfigError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::FsError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    NoValidHomeDirectory(#[from] NoValidHomeDirectory),
    #[error("error parsing {config_file}")]
    Deserialize {
        config_file: String,
        #[diagnostic_source]
        source: TomlDeError,
    },
    #[error("error parsing URL: {source}")]
    UrlParseError {
        source: url::ParseError,
        #[help]
        help: Option<String>,
    },
}

/// Incrementally builds a [`Config`] by layering configuration sources.
///
/// - Call [`ConfigBuilder::default`] to start with a blank slate,
///   or call [`ConfigBuilder::new`] to start from a deserialised configuration file.
/// - Populate the fields from overriding sources (e.g. CLI arguments).
/// - Finish with [`ConfigBuilder::build`].
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
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
    namespace: Option<String>,
    lua_version: Option<LuaVersion>,
    user_tree: Option<PathBuf>,
    workspace_tree: Option<PathBuf>,
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
    #[serde(default)]
    build: BuildConfig,
    #[serde(default)]
    entrypoint_layout: RockLayoutConfig,
    user_agent: Option<String>,
    generate_luarc: Option<bool>,
    luarc_file_name: Option<String>,
    wrap_bin_scripts: Option<bool>,
    package_types: Option<RemotePackageTypeFilterSpec>,
    no_tfa: Option<bool>,
}

/// A builder for the lux `Config`.
impl ConfigBuilder {
    /// Create a new `ConfigBuilder` by deserializing from a config file
    /// if present, or otherwise by instantiating the default config.
    pub fn new() -> Result<Self, ConfigError> {
        let config_file = Self::config_file()?;
        if config_file.is_file() {
            Self::from_file(&config_file)
        } else {
            Ok(Self::default())
        }
    }

    pub(crate) fn from_file(config_file: &Path) -> Result<Self, ConfigError> {
        let config_file_name = config_file.to_string_lossy().to_string();
        let content = fs::sync::read_to_string(config_file)?;
        crate::project::parse_toml(&config_file_name, &content).map_err(|source| {
            ConfigError::Deserialize {
                config_file: config_file_name,
                source,
            }
        })
    }

    /// Get the path to the lux config file.
    pub fn config_file() -> Result<PathBuf, NoValidHomeDirectory> {
        let project_dirs = directories::ProjectDirs::from("org", "lumenlabs", "lux")
            .ok_or(NoValidHomeDirectory)?;
        Ok(project_dirs.config_dir().join("config.toml").to_path_buf())
    }

    /// Whether to enable development packages
    /// Default: `false`
    pub fn dev(self, dev: Option<bool>) -> Self {
        Self {
            enable_development_packages: dev.or(self.enable_development_packages),
            ..self
        }
    }

    /// Fetch rocks/rockspecs from this luarocks server
    /// Default: `"https://luarocks.org/"`
    pub fn server(self, server: Option<Url>) -> Self {
        Self {
            server: server.or(self.server),
            ..self
        }
    }

    /// Fetch rocks/rockspecs from these servers in addition to the main server
    pub fn extra_servers(self, extra_servers: Option<Vec<Url>>) -> Self {
        Self {
            extra_servers: extra_servers.or(self.extra_servers),
            ..self
        }
    }

    /// The luarocks server namespace to use
    pub fn namespace(self, namespace: Option<String>) -> Self {
        Self {
            namespace: namespace.or(self.namespace),
            ..self
        }
    }

    /// The directory in which to install Lua if not found
    pub fn lua_dir(self, lua_dir: Option<PathBuf>) -> Self {
        Self {
            lua_dir: lua_dir.or(self.lua_dir),
            ..self
        }
    }

    /// Which Lua version to use.
    /// Default: The installed Lua version, if detected
    pub fn lua_version(self, lua_version: Option<LuaVersion>) -> Self {
        Self {
            lua_version: lua_version.or(self.lua_version),
            ..self
        }
    }

    /// Which tree to operate on
    pub fn user_tree(self, tree: Option<PathBuf>) -> Self {
        Self {
            user_tree: tree.or(self.user_tree),
            ..self
        }
    }

    /// Which tree to operate on when in a workspace
    /// Default: A `.lux` directory in the workspace root.
    pub fn workspace_tree(self, tree: Option<PathBuf>) -> Self {
        Self {
            workspace_tree: tree.or(self.workspace_tree),
            ..self
        }
    }

    /// Variable names, mapped to their values.
    /// Lux populates variables in the `lux.toml` and in RockSpecs
    /// with these before building.
    pub fn variables(self, variables: Option<HashMap<String, String>>) -> Self {
        Self {
            variables: variables.or(self.variables),
            ..self
        }
    }

    /// Whether to display verbose output of commands executed.
    /// Default: `false`
    pub fn verbose(self, verbose: Option<bool>) -> Self {
        Self {
            verbose: verbose.or(self.verbose),
            ..self
        }
    }

    /// Whether to disable printing progress bars and spinners
    /// Default: `false`
    pub fn no_progress(self, no_progress: Option<bool>) -> Self {
        Self {
            no_progress: no_progress.or(self.no_progress),
            ..self
        }
    }

    /// Whether to disable user prompts
    /// Default: `false`
    pub fn no_prompt(self, no_prompt: Option<bool>) -> Self {
        Self {
            no_prompt: no_prompt.or(self.no_prompt),
            ..self
        }
    }

    /// Timeout on network operations, in seconds.
    /// 0 means no timeout (wait forever).
    /// Default: 30 s
    pub fn timeout(self, timeout: Option<Duration>) -> Self {
        Self {
            timeout: timeout.or(self.timeout),
            ..self
        }
    }

    /// Maximum buffer size for parallel jobs, such as downloading rockspecs and installing rocks.
    /// 0 means no limit.
    /// Default: 0
    pub fn max_jobs(self, max_jobs: Option<usize>) -> Self {
        Self {
            max_jobs: max_jobs.or(self.max_jobs),
            ..self
        }
    }

    /// The cache directory, e.g. for luarocks manifests.
    pub fn cache_dir(self, cache_dir: Option<PathBuf>) -> Self {
        Self {
            cache_dir: cache_dir.or(self.cache_dir),
            ..self
        }
    }

    /// The data directory, in which the default user install tree resides.
    pub fn data_dir(self, data_dir: Option<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.or(self.data_dir),
            ..self
        }
    }

    /// Specifies a directory with locally vendored sources and RockSpecs.
    /// When building or installing a package with this flag,
    /// Lux will fetch sources from the <vendor-dir> instead of from a remote server.
    pub fn vendor_dir(self, vendor_dir: Option<PathBuf>) -> Self {
        Self {
            vendor_dir: vendor_dir.or(self.vendor_dir),
            ..self
        }
    }

    /// The rock layout for entrypoints of new install trees.
    /// Does not affect existing install trees or dependency rock layouts.
    pub fn entrypoint_layout(self, rock_layout: RockLayoutConfig) -> Self {
        Self {
            entrypoint_layout: rock_layout,
            ..self
        }
    }

    /// The user agent to set when making web requests.
    /// Default: "lux-lib/<version>".
    pub fn user_agent(self, user_agent: Option<String>) -> Self {
        Self {
            user_agent: user_agent.or(self.user_agent),
            ..self
        }
    }

    /// Whether to generate a `.luarc.json` on build.
    /// Default: `true`
    pub fn generate_luarc(self, generate: Option<bool>) -> Self {
        Self {
            generate_luarc: generate.or(self.generate_luarc),
            ..self
        }
    }

    /// Lua runtime configuration file name
    /// Default: `.luarc.json`
    pub fn luarc_file_name(self, file: Option<String>) -> Self {
        Self {
            luarc_file_name: file.or(self.luarc_file_name),
            ..self
        }
    }

    /// Whether to wrap installed Lua bin scripts to be executed with
    /// the detected or configured Lua installation.
    /// Setting this to `false` disables wrapping globally.
    /// If set to `true`, individual rocks can still disable wrapping of their own bin scripts.
    /// Default: `true`.
    pub fn wrap_bin_scripts(self, generate: Option<bool>) -> Self {
        Self {
            wrap_bin_scripts: generate.or(self.generate_luarc),
            ..self
        }
    }
    /// Whether to disable prompts for two-factor authentication (2FA) codes.
    /// Default: `false`.
    pub fn no_tfa(self, tfa: Option<bool>) -> Self {
        Self {
            no_tfa: tfa.or(self.no_tfa),
            ..self
        }
    }

    /// The command prefix with which to wrap all build commands.
    ///
    /// If set, every command spawned by the build backends (`make`, `cmake`,
    /// `rust-mlua`, `command`, and `luarocks`) is invoked as
    /// `runner + [command, arguments...]`.
    ///
    /// If unset, no wrapping is performed.
    ///
    /// # Examples
    ///
    /// Use [`bubblewrap`](https://github.com/containers/bubblewrap) to run
    /// builds in a sandbox with read-only access to the rest of the
    /// filesystem (Linux):
    ///
    /// ```toml
    /// [build]
    /// runner = [
    ///     "bwrap",
    ///     "--ro-bind", "/", "/",
    ///     "--dev-bind", "/dev", "/dev",
    ///     "--proc", "/proc",
    ///     "--bind", "/tmp", "/tmp",
    ///     "--bind", "/home/user/.cache/lux", "/home/user/.cache/lux",
    ///     "--bind", "/home/user/.local/share/lux", "/home/user/.local/share/lux",
    ///     "--unshare-net",
    ///     "--new-session",
    /// ]
    /// ```
    ///
    /// Or use `sandbox-exec` with a seatbelt profile (macOS):
    ///
    /// ```text
    /// (version 1)
    /// (deny default)
    /// (allow file-read*)
    /// (allow process*)
    /// (allow sysctl-read)
    /// (allow file-write* (subpath "/tmp") (subpath "/Users/user/Library/Caches/lux") (subpath "/Users/user/.local/share/lux"))
    /// ```
    ///
    /// ```toml
    /// [build]
    /// runner = ["sandbox-exec", "-f", "/Users/user/sandbox.sb"]
    /// ```
    ///
    /// The writable directories must cover the places lux writes to during a
    /// build: the temporary build directory (a tempfile, usually under
    /// `/tmp`), the install tree (under the data directory), and cache
    /// directories used by the various build backends.
    pub fn build_runner(self, runner: Option<Vec<String>>) -> Self {
        Self {
            build: BuildConfig {
                runner: runner.unwrap_or(self.build.runner),
            },
            ..self
        }
    }

    /// Merge with another [`ConfigBuilder`]. The other one takes precedence.
    pub fn merge(self, other: Self) -> Self {
        Self {
            server: other.server.or(self.server),
            extra_servers: other.extra_servers.or(self.extra_servers),
            namespace: other.namespace.or(self.namespace),
            lua_version: other.lua_version.or(self.lua_version),
            user_tree: other.user_tree.or(self.user_tree),
            workspace_tree: other.workspace_tree.or(self.workspace_tree),
            lua_dir: other.lua_dir.or(self.lua_dir),
            cache_dir: other.cache_dir.or(self.cache_dir),
            data_dir: other.data_dir.or(self.data_dir),
            vendor_dir: other.vendor_dir.or(self.vendor_dir),
            enable_development_packages: other
                .enable_development_packages
                .or(self.enable_development_packages),
            verbose: other.verbose.or(self.verbose),
            no_progress: other.no_progress.or(self.no_progress),
            no_prompt: other.no_prompt.or(self.no_prompt),
            timeout: other.timeout.or(self.timeout),
            max_jobs: other.max_jobs.or(self.max_jobs),
            variables: other.variables.or(self.variables),
            external_deps: other.external_deps,
            build: other.build,
            entrypoint_layout: other.entrypoint_layout,
            user_agent: other.user_agent.or(self.user_agent),
            generate_luarc: other.generate_luarc.or(self.generate_luarc),
            luarc_file_name: other.luarc_file_name.or(self.luarc_file_name),
            wrap_bin_scripts: other.wrap_bin_scripts.or(self.wrap_bin_scripts),
            package_types: other.package_types.or(self.package_types),
            no_tfa: other.no_tfa.or(self.no_tfa),
        }
    }

    #[tracing::instrument(level = "trace")]
    pub fn build(self) -> Result<Config, ConfigError> {
        let data_dir = self.data_dir.unwrap_or(Config::default_data_path()?);
        let cache_dir = self.cache_dir.unwrap_or(Config::default_cache_path()?);
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
            namespace: self.namespace,
            lua_dir: self.lua_dir,
            lua_version,
            user_tree,
            workspace_tree: self.workspace_tree,
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
            build: self.build,
            entrypoint_layout: self.entrypoint_layout,
            cache_dir,
            data_dir,
            vendor_dir: self.vendor_dir,
            user_agent: self.user_agent.unwrap_or(DEFAULT_USER_AGENT.into()),
            generate_luarc: self.generate_luarc.unwrap_or(true),
            luarc_file_name: self
                .luarc_file_name
                .unwrap_or_else(|| ".luarc.json".to_string()),
            wrap_bin_scripts: self.wrap_bin_scripts.unwrap_or(true),
            package_types: self.package_types.unwrap_or_default(),
            no_tfa: self.no_tfa.unwrap_or(false),
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
            namespace: value.namespace,
            lua_dir: value.lua_dir,
            lua_version: value.lua_version,
            user_tree: Some(value.user_tree),
            workspace_tree: value.workspace_tree,
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
            build: value.build,
            entrypoint_layout: value.entrypoint_layout,
            user_agent: Some(value.user_agent),
            generate_luarc: Some(value.generate_luarc),
            luarc_file_name: Some(value.luarc_file_name),
            wrap_bin_scripts: Some(value.wrap_bin_scripts),
            package_types: Some(value.package_types),
            no_tfa: Some(value.no_tfa),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wrapped_command_without_runner() {
        let echo = which::which("echo").unwrap();
        let config = ConfigBuilder::default().build().unwrap();
        let output = config
            .wrapped_command(&echo, ["hello", "world"])
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "hello world"
        );
    }

    #[tokio::test]
    async fn wrapped_command_prepends_runner_argv() {
        let echo = which::which("echo").unwrap();
        let config = ConfigBuilder::default()
            .build_runner(Some(vec![echo.to_string_lossy().into(), "--prefix".into()]))
            .build()
            .unwrap();
        let output = config
            .wrapped_command("world", Vec::<String>::new())
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "--prefix world"
        );
    }
}
