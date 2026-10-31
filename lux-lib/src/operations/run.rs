use crate::package;
use crate::rockspec;
use std::{ops::Deref, path::PathBuf};

use bon::Builder;
use itertools::Itertools;
use miette::Diagnostic;
use serde::Deserialize;
use thiserror::Error;
use tokio::process::Command as ProcessCommand;

use crate::{
    config::Config,
    lua,
    operations::run_lua::{self, RunLua},
    path::{self, LuaEnvironment, Paths},
    tree::InstallTree,
    workspace::{self, TreeError, Workspace},
};

#[derive(Debug, Error, Diagnostic)]
#[error("'{0}' should not be used as a `command` as it is not cross-platform.
You should only change the default `command` if it is a different Lua interpreter that behaves identically on all platforms.
Consider removing the `command` field and letting Lux choose the default Lua interpreter instead.")]
pub struct CommandError(String);

#[derive(Debug, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", serde(transparent))]
pub struct Command(String);

impl Command {
    pub fn from(command: String) -> Result<Self, CommandError> {
        match command.as_str() {
            // Common Lua interpreters that could lead to cross-platform issues
            // Luajit is also included because it may or may not have lua52 syntax support compiled in.
            "lua" | "lua5.1" | "lua5.2" | "lua5.3" | "lua5.4" | "luajit" => {
                Err(CommandError(command))
            }
            _ => Ok(Self(command)),
        }
    }
}

impl<'de> Deserialize<'de> for Command {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let command = String::deserialize(deserializer)?;

        Command::from(command).map_err(serde::de::Error::custom)
    }
}

impl Deref for Command {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Error, Diagnostic)]
#[error(transparent)]
#[non_exhaustive]
pub enum Error {
    #[diagnostic(transparent)]
    Toml(#[from] rockspec::toml::Error),
    #[diagnostic(transparent)]
    Command(#[from] CommandError),
    #[diagnostic(transparent)]
    LuaVersion(#[from] rockspec::Error),
    #[diagnostic(transparent)]
    RunLua(#[from] run_lua::Error),
    #[error("failed to detect or install a luau runtime")]
    #[diagnostic(forward(0))]
    LuaInstallation(#[from] lua::installation::Error),
    #[diagnostic(transparent)]
    Workspace(#[from] workspace::Error),
    #[diagnostic(transparent)]
    WorkspaceTree(#[from] TreeError),
    Io(#[from] std::io::Error),
    #[diagnostic(transparent)]
    Paths(#[from] path::Error),
    #[error("No `run` field found in `lux.toml`")]
    NoRunField,
}

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Run<'a> {
    workspace: &'a Workspace,
    package: Option<package::Name>,
    dir: Option<PathBuf>,
    args: &'a [String],
    config: &'a Config,
    disable_loader: Option<bool>,
    environment: LuaEnvironment,
}

impl<State> RunBuilder<'_, State>
where
    State: run_builder::State + run_builder::IsComplete,
{
    pub async fn run(self) -> Result<(), Error> {
        let run = self._build();
        let workspace = run.workspace;
        let config = run.config;
        let extra_args = run.args;
        let project = workspace.single_member_or_select(&run.package)?;
        let toml = project.toml().into_local()?;

        let run_spec = toml
            .run()
            .ok_or(Error::NoRunField)?
            .current_platform()
            .clone();

        let mut args: Vec<String> = run_spec.args.map(|args| args.into()).unwrap_or_default();

        if !extra_args.is_empty() {
            args.extend(extra_args.iter().cloned());
        }
        let disable_loader = run.disable_loader.unwrap_or(false);
        let environment = run.environment;
        match &run_spec.command {
            Some(command) => {
                run_with_command(
                    workspace,
                    command,
                    run.dir,
                    disable_loader,
                    &args,
                    config,
                    environment,
                )
                .await
            }
            None => {
                run_with_local_lua(
                    workspace,
                    run.dir,
                    disable_loader,
                    &args,
                    config,
                    environment,
                )
                .await
            }
        }
    }
}

async fn run_with_local_lua(
    workspace: &Workspace,
    root_dir: Option<PathBuf>,
    disable_loader: bool,
    args: &Vec<String>,
    config: &Config,
    environment: LuaEnvironment,
) -> Result<(), Error> {
    let version = workspace.lua_version(config)?;

    let tree = workspace.tree(config)?;

    let lua_cmd = lua::Binary::from(run_lua::resolve_runtime(&version, config).await?);

    RunLua::new()
        .root(&root_dir.unwrap_or(workspace.root().to_path_buf()))
        .tree(&tree)
        .config(config)
        .lua_cmd(lua_cmd)
        .disable_loader(disable_loader)
        .args(args)
        .environment(environment)
        .run_lua()
        .await?;

    Ok(())
}

async fn run_with_command(
    workspace: &Workspace,
    command: &Command,
    root_dir: Option<PathBuf>,
    disable_loader: bool,
    args: &[String],
    config: &Config,
    environment: LuaEnvironment,
) -> Result<(), Error> {
    let tree = workspace.tree(config)?;
    let paths = Paths::for_environment(&tree, config, environment)?;

    let lua_init = if tree.version().is_luau() || disable_loader {
        None
    } else if tree.version().lux_lib_dir().is_none() {
        tracing::warn!("lux-lua library not found.\n    Cannot use the `lux.loader`.\n    To suppress this warning, set the `--no-loader` option.");
        None
    } else {
        Some(paths.init())
    };

    let mut cmd = ProcessCommand::new(command.deref());
    if let Some(dir) = root_dir {
        cmd.current_dir(dir);
    } else {
        cmd.current_dir(workspace.root());
    }
    match cmd
        .args(args.iter().cloned().collect_vec())
        .env("PATH", paths.path_prepended().joined())
        .envs(paths.lua_env_vars(environment, &lua_init.unwrap_or_default()))
        .status()
        .await?
        .code()
    {
        Some(0) => Ok(()),
        code => Err(run_lua::Error::LuaCommandNonZeroExitCode {
            lua_cmd: command.to_string(),
            exit_code: code,
        }
        .into()),
    }
}
