use std::ops::Deref;

use bon::Builder;
use itertools::Itertools;
use nonempty::NonEmpty;
use serde::Deserialize;
use thiserror::Error;
use tokio::process::Command;

use crate::{
    config::Config,
    lua_installation::LuaBinary,
    lua_rockspec::LuaVersionError,
    operations::run_lua::RunLua,
    path::{Paths, PathsError},
    project::{project_toml::LocalProjectTomlValidationError, Project, ProjectTreeError},
};

use super::RunLuaError;

#[derive(Debug, Error)]
#[error("`{0}` should not be used as a `command` as it is not cross-platform.
You should only change the default `command` if it is a different Lua interpreter that behaves identically on all platforms.
Consider removing the `command` field and letting Lux choose the default Lua interpreter instead.")]
pub struct RunCommandError(String);

#[derive(Debug, Clone)]
pub struct RunCommand(String);

impl RunCommand {
    pub fn from(command: String) -> Result<Self, RunCommandError> {
        match command.as_str() {
            // Common Lua interpreters that could lead to cross-platform issues
            // Luajit is also included because it may or may not have lua52 syntax support compiled in.
            "lua" | "lua5.1" | "lua5.2" | "lua5.3" | "lua5.4" | "luajit" => {
                Err(RunCommandError(command))
            }
            _ => Ok(Self(command)),
        }
    }
}

impl<'de> Deserialize<'de> for RunCommand {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let command = String::deserialize(deserializer)?;

        RunCommand::from(command).map_err(serde::de::Error::custom)
    }
}

impl Deref for RunCommand {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Error)]
#[error(transparent)]
pub enum RunError {
    Toml(#[from] LocalProjectTomlValidationError),
    RunCommand(#[from] RunCommandError),
    LuaVersion(#[from] LuaVersionError),
    RunLua(#[from] RunLuaError),
    ProjectTree(#[from] ProjectTreeError),
    Io(#[from] std::io::Error),
    Paths(#[from] PathsError),
    #[error("No `run` field found in `lux.toml`")]
    NoRunField,
}

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Run<'a> {
    project: &'a Project,
    args: &'a [String],
    config: &'a Config,
    disable_loader: Option<bool>,
}

impl<State> RunBuilder<'_, State>
where
    State: run_builder::State + run_builder::IsComplete,
{
    pub async fn run(self) -> Result<(), RunError> {
        let run = self._build();
        let project = run.project;
        let config = run.config;
        let extra_args = run.args;
        let toml = project.toml().into_local()?;

        let run_spec = toml
            .run()
            .ok_or(RunError::NoRunField)?
            .current_platform()
            .clone();

        let mut args = run_spec.args.unwrap_or_default();

        if !extra_args.is_empty() {
            args.extend(extra_args.iter().cloned());
        }
        let disable_loader = run.disable_loader.unwrap_or(false);
        match &run_spec.command {
            Some(command) => {
                run_with_command(project, command, disable_loader, &args, config).await
            }
            None => run_with_local_lua(project, disable_loader, &args, config).await,
        }
    }
}

async fn run_with_local_lua(
    project: &Project,
    disable_loader: bool,
    args: &NonEmpty<String>,
    config: &Config,
) -> Result<(), RunError> {
    let version = project.lua_version(config)?;

    let tree = project.tree(config)?;
    let args = &args.into_iter().cloned().collect();

    RunLua::new()
        .root(project.root())
        .tree(&tree)
        .config(config)
        .lua_cmd(LuaBinary::new(version, config))
        .disable_loader(disable_loader)
        .args(args)
        .run_lua()
        .await?;

    Ok(())
}

async fn run_with_command(
    project: &Project,
    command: &RunCommand,
    disable_loader: bool,
    args: &NonEmpty<String>,
    config: &Config,
) -> Result<(), RunError> {
    let tree = project.tree(config)?;
    let paths = Paths::new(&tree)?;

    let lua_init = if disable_loader {
        None
    } else if tree.version().lux_lib_dir().is_none() {
        eprintln!(
            "⚠️ WARNING: lux-lua library not found.
    Cannot use the `lux.loader`.
    To suppress this warning, set the `--no-loader` option.
                    "
        );
        None
    } else {
        Some(paths.init())
    };

    match Command::new(command.deref())
        .args(args.into_iter().cloned().collect_vec())
        .current_dir(project.root().deref())
        .env("PATH", paths.path_prepended().joined())
        .env("LUA_INIT", lua_init.unwrap_or_default())
        .env("LUA_PATH", paths.package_path().joined())
        .env("LUA_CPATH", paths.package_cpath().joined())
        .status()
        .await?
        .code()
    {
        Some(0) => Ok(()),
        code => Err(RunLuaError::LuaCommandNonZeroExitCode {
            lua_cmd: command.to_string(),
            exit_code: code,
        }
        .into()),
    }
}
