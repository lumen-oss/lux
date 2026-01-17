use crate::{completion::Completion, format::Fmt, project::NewProject};
use std::error::Error;
use std::path::PathBuf;

use add::Add;
use build::Build;
use check::Check;
use clap::{Parser, Subcommand};
use config::ConfigCmd;
use debug::Debug;
use doc::Doc;
use download::Download;
use exec::Exec;
use generate_rockspec::GenerateRockspec;
use info::Info;
use install::Install;
use install_rockspec::InstallRockspec;
use lint::Lint;
use list::ListCmd;
use lux_lib::config::LuaVersion;
use outdated::Outdated;
use pack::Pack;
use path::Path;
use pin::ChangePin;
use remove::Remove;
use run::Run;
use run_lua::RunLua;
use search::Search;
use shell::Shell;
use test::Test;
use uninstall::Uninstall;
use update::Update;
use upload::Upload;
use url::Url;
use vendor::Vendor;
use which::Which;

pub mod add;
pub mod build;
pub mod check;
pub mod completion;
pub mod config;
pub mod debug;
pub mod doc;
pub mod download;
pub mod exec;
pub mod fetch;
pub mod format;
pub mod generate_rockspec;
pub mod info;
pub mod install;
pub mod install_lua;
pub mod install_rockspec;
pub mod lint;
pub mod list;
pub mod outdated;
pub mod pack;
pub mod path;
pub mod pin;
pub mod project;
pub mod purge;
pub mod remove;
pub mod run;
pub mod run_lua;
pub mod search;
pub mod shell;
pub mod test;
pub mod uninstall;
pub mod unpack;
pub mod update;
pub mod upload;
pub mod utils;
pub mod vendor;
pub mod which;

/// A luxurious package manager for Lua.
#[derive(Parser)]
#[command(author, version, about, long_about = None, arg_required_else_help = true)]
pub struct Cli {
    /// Enable the sub-repositories in luarocks servers for rockspecs of in-development versions.
    #[arg(long)]
    pub dev: bool,

    /// Fetch rocks/rockspecs from this server (takes priority over config file).
    #[arg(long, value_name = "server")]
    pub server: Option<Url>,

    /// Fetch rocks/rockspecs from this server in addition to the main server{n}
    /// (overrides any entries in the config file).
    #[arg(long, value_name = "extra-server")]
    pub extra_servers: Option<Vec<Url>>,

    /// Restrict downloads to paths matching the given URL.
    #[arg(long, value_name = "url")]
    pub only_sources: Option<String>,

    /// Specify the luarocks server namespace to use.
    #[arg(long, value_name = "namespace")]
    pub namespace: Option<String>,

    /// Specify the directory in which to install Lua{n}
    /// if not found.
    #[arg(long, value_name = "prefix")]
    pub lua_dir: Option<PathBuf>,

    /// Which Lua installation to use.{n}
    /// Valid versions are: '5.1', '5.2', '5.3', '5.4', 'jit' and 'jit52'.
    #[arg(long, value_name = "ver")]
    pub lua_version: Option<LuaVersion>,

    /// Which tree to operate on.
    #[arg(long, value_name = "tree")]
    pub tree: Option<PathBuf>,

    /// Specifies the cache directory for e.g. luarocks manifests.
    #[arg(long, value_name = "cache-dir")]
    pub cache_dir: Option<PathBuf>,

    /// Specifies the data directory (e.g. ~/.local/share/lux).
    #[arg(long, value_name = "data-dir")]
    pub data_dir: Option<PathBuf>,

    /// Specifies a directory with locally vendored sources and RockSpecs.{n}
    /// When building or installing a package with this flag,{n}
    /// Lux will fetch sources from the <vendor-dir> instead of from a remote server.
    #[arg(long, value_name = "vendor-dir")]
    pub vendor_dir: Option<PathBuf>,

    /// Override config variables.{n}
    /// Example: `lx -v "LUA=/path/to/lua" ...`
    #[arg(long, value_name = "variable", visible_short_alias = 'v', value_parser = parse_key_val::<String, String>)]
    pub variables: Option<Vec<(String, String)>>,

    /// Display verbose output of commands executed.
    #[arg(long)]
    pub verbose: bool,

    /// Don't print any progress bars or spinners.
    #[arg(long)]
    pub no_progress: bool,

    /// Configure lux for installing Neovim packages.
    #[arg(long)]
    pub nvim: bool,

    /// Timeout on network operations, in seconds.{n}
    /// 0 means no timeout (wait forever). Default is 30.
    #[arg(long, value_name = "seconds")]
    pub timeout: Option<usize>,

    /// Maximum buffer size for parallel jobs, such as downloading rockspecs and installing rocks.
    /// 0 means no limit. Default is 0.
    #[arg(long, visible_short_alias = 'j')]
    pub max_jobs: Option<usize>,

    /// Do not generate or update a `.luarc.json` file when building{n}
    /// a project.
    #[arg(long)]
    pub no_luarc: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Add a dependency to the current project.
    Add(Add),
    /// Build/compile a project.
    Build(Build),
    /// [EXPERIMENTAL]{n}
    /// Type check the current project based on EmmyLua/LuaCATS annotations.{n}
    /// Respects `.emmyrc.json` and `.luarc.json` files in the project directory.
    Check(Check),
    /// Interact with the lux configuration.
    #[command(subcommand, arg_required_else_help = true)]
    Config(ConfigCmd),
    /// Generate autocompletion scripts for the shell.{n}
    /// Example: `lx completion zsh > ~/.zsh/completions/_lx`
    Completion(Completion),
    /// Internal commands for debugging Lux itself.
    #[command(subcommand, arg_required_else_help = true)]
    Debug(Debug),
    /// Show documentation for an installed rock.
    Doc(Doc),
    /// Download a specific rock file from a luarocks server.
    #[command(arg_required_else_help = true)]
    Download(Download),
    /// Formats the codebase with stylua.
    Fmt(Fmt),
    /// Generate a rockspec file from a project.
    GenerateRockspec(GenerateRockspec),
    /// Show metadata for any rock.
    Info(Info),
    /// Install a rock for use on the system.
    #[command(arg_required_else_help = true)]
    Install(Install),
    /// Install a local rockspec for use on the system.
    #[command(arg_required_else_help = true)]
    InstallRockspec(InstallRockspec),
    /// Manually install and manage Lua headers for various Lua versions.
    InstallLua,
    /// Lint the current project using `luacheck`.
    Lint(Lint),
    /// List currently installed rocks.
    List(ListCmd),
    /// Run lua, with the `LUA_PATH` and `LUA_CPATH` set to the specified lux tree.
    Lua(RunLua),
    /// Create a new Lua project.
    New(NewProject),
    /// List outdated rocks.
    Outdated(Outdated),
    /// Create a packed rock for distribution, packing sources or binaries.
    Pack(Pack),
    /// Return the currently configured package path.
    Path(Path),
    /// Pin an existing rock, preventing any updates to the package.
    Pin(ChangePin),
    /// Remove all installed rocks from a tree.
    Purge,
    /// Remove a rock from the current project's lux.toml dependencies.
    Remove(Remove),
    /// Run the current project with the provided arguments.
    Run(Run),
    /// Execute a command that has been installed with lux.
    /// If the command is not found, a package named after the command
    /// will be installed.
    Exec(Exec),
    /// Query the luarocks servers.
    #[command(arg_required_else_help = true)]
    Search(Search),
    /// Run the test suite in the current project directory.{n}
    /// Lux supports the following test backends, specified by the `[test]` table in the lux.toml:{n}
    /// {n}
    ///   - busted:{n}
    ///     {n}
    ///     https://lunarmodules.github.io/busted/{n}
    ///     {n}
    ///     Example:{n}
    ///     {n}
    ///     ```toml{n}
    ///     [test]{n}
    ///     type = "busted"{n}
    ///     flags = [ ] # Optional CLI flags to pass to busted{n}
    ///     ```{n}
    ///     {n}
    ///     `lx test` will default to using `busted` if no test backend is specified and:{n}
    ///         * there is a `.busted` file in the project root{n}
    ///         * or `busted` is one of the `test_dependencies`).{n}
    /// {n}
    ///   - busted-nlua:{n}:
    ///     {n}
    ///     [currently broken on macOS and Windows]
    ///     A build backend for running busted tests with Neovim as the Lua interpreter.
    ///     Used for testing Neovim plugins.
    ///     {n}
    ///     Example:{n}
    ///     {n}
    ///     ```toml{n}
    ///     [test]{n}
    ///     type = "busted-nlua"{n}
    ///     flags = [ ] # Optional CLI flags to pass to busted{n}
    ///     ```{n}
    ///     {n}
    ///     `lx test` will default to using `busted-nlua` if no test backend is specified and:{n}
    ///         * there is a `.busted` file in the project root{n}
    ///         * or `busted` and `nlua` are `test_dependencies`.{n}
    /// {n}
    ///   - command:{n}
    ///     {n}
    ///     Name/file name of a shell command that will run the test suite.{n}
    ///     Example:{n}
    ///     {n}
    ///     ```toml{n}
    ///     [test]{n}
    ///     type = "command"{n}
    ///     command = "make"{n}
    ///     flags = [ "test" ]{n}
    ///     ```{n}
    ///     {n}
    ///   - script:{n}
    ///     {n}
    ///     Relative path to a Lua script that will run the test suite.{n}
    ///     Example:{n}
    ///     {n}
    ///     ```toml{n}
    ///     [test]{n}
    ///     type = "script"{n}
    ///     script = "tests.lua" # Expects a tests.lua file in the project root{n}
    ///     flags = [ ] # Optional arguments passed to the test script{n}
    ///     ```{n}
    Test(Test),
    /// Uninstall a rock from the system.
    Uninstall(Uninstall),
    /// Unpins an existing rock, allowing updates to alter the package.
    Unpin(ChangePin),
    /// Updates all rocks in a project.
    Update(Update),
    /// Generate a Lua rockspec for a Lux project and upload it to the public luarocks repository.{n}
    /// You can specify a source template for release and dev packages in the lux.toml.{n}
    /// {n}
    /// Example:{n}
    /// {n}
    /// ```toml{n}
    /// [source]{n}
    /// url = "https://host.com/owner/$(PACKAGE)/refs/tags/$(REF).zip"{n}
    /// dev = "git+https://host.com/owner/$(PACKAGE).git"{n}
    /// ```{n}
    /// {n}
    /// You can use the following variables in the source template:{n}
    /// {n}
    ///  - $(PACKAGE): The package name.{n}
    ///  - $(VERSION): The package version.{n}
    ///  - $(REF): The git tag or revision (if in a git repository).{n}
    ///  - You may also specify environment variables with `$(<VAR_NAME>)`.{n}
    /// {n}
    /// If the `version` is not set in the lux.toml, lux will search the current
    /// commit for SemVer tags and if found, will use it to generate the package version.
    Upload(Upload),
    /// Vendor the dependencies of a project or RockSpec locally.
    /// When building or installing a package with the `--vendor-dir` option{n}
    /// or the `[vendor_dir]` config option, Lux will fetch sources from the <vendor-dir>{n}
    /// instead of from a remote server.
    Vendor(Vendor),
    /// Tell which file corresponds to a given module name.
    Which(Which),
    /// Spawns an interactive shell with PATH, LUA_PATH, LUA_CPATH and LUA_INIT set.
    Shell(Shell),
}

/// Parse a key=value pair.
fn parse_key_val<T, U>(s: &str) -> Result<(T, U), Box<dyn Error + Send + Sync + 'static>>
where
    T: std::str::FromStr,
    T::Err: Error + Send + Sync + 'static,
    U: std::str::FromStr,
    U::Err: Error + Send + Sync + 'static,
{
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=value: no `=` found in `{s}`"))?;
    Ok((s[..pos].parse()?, s[pos + 1..].parse()?))
}
