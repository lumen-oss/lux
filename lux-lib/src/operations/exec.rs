use std::io;
use tokio::process::Command;

use crate::{
    config::{Config, LuaVersion, LuaVersionUnset},
    lua_rockspec::LuaVersionError,
    operations::{BuildProject, BuildProjectError, Install},
    package::{PackageReq, PackageVersionReqError},
    path::{Paths, PathsError},
    project::{Project, ProjectTreeError},
    remote_package_db::RemotePackageDBError,
    tree::{self, TreeError},
};
use bon::Builder;
use itertools::Itertools;
use thiserror::Error;
use which::which;

use super::{InstallError, PackageInstallSpec};

/// Rocks package runner, providing fine-grained control
/// over how a package should be run.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _exec, vis = ""))]
pub struct Exec<'a> {
    #[builder(start_fn)]
    command: &'a str,
    #[builder(start_fn)]
    project: Option<&'a Project>,
    #[builder(start_fn)]
    config: &'a Config,

    #[builder(field)]
    args: Vec<String>,

    disable_loader: Option<bool>,
}

impl<State: exec_builder::State> ExecBuilder<'_, State> {
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args(mut self, args: impl IntoIterator<Item: Into<String>>) -> Self {
        self.args.extend(args.into_iter().map_into());
        self
    }
}

impl<State> ExecBuilder<'_, State>
where
    State: exec_builder::State + exec_builder::IsComplete,
{
    pub async fn exec(self) -> Result<(), ExecError>
    where
        State: exec_builder::IsComplete,
    {
        exec(self._exec()).await
    }
}

#[derive(Error, Debug)]
pub enum ExecError {
    #[error("failed to run {cmd}: {source}")]
    RunCommandFailed {
        cmd: String,
        #[source]
        source: io::Error,
    },
    #[error("{cmd} exited with non-zero exit code: {}", exit_code.map(|code| code.to_string()).unwrap_or("unknown".into()))]
    RunCommandNonZeroExitCode { cmd: String, exit_code: Option<i32> },
    #[error(transparent)]
    LuaVersionUnset(#[from] LuaVersionUnset),
    #[error(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    Paths(#[from] PathsError),
    #[error(transparent)]
    LuaVersionError(#[from] LuaVersionError),
    #[error(transparent)]
    BuildProject(#[from] BuildProjectError),
    #[error(transparent)]
    InstallCommand(#[from] InstallCommandError),
    #[error(transparent)]
    ProjectTreeError(#[from] ProjectTreeError),
    #[error("failed to execute `{0}`:\n{1}")]
    Io(String, io::Error),
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum InstallCommandError {
    InstallError(#[from] InstallError),
    PackageVersionReqError(#[from] PackageVersionReqError),
    RemotePackageDBError(#[from] RemotePackageDBError),
    Tree(#[from] TreeError),
    LuaVersionUnset(#[from] LuaVersionUnset),
}

async fn exec(run: Exec<'_>) -> Result<(), ExecError> {
    let lua_version = run
        .project
        .map(|project| project.lua_version(run.config))
        .transpose()?
        .unwrap_or(LuaVersion::from_config(run.config)?.clone());

    if let Some(project) = run.project {
        BuildProject::new(project, run.config)
            .no_lock(false)
            .only_deps(false)
            .build()
            .await?;
    } else if which(run.command).is_err() {
        install_command(run.command, run.config).await?
    };

    let user_tree = run.config.user_tree(lua_version)?;
    let mut paths = Paths::new(&user_tree)?;

    if let Some(project) = run.project {
        paths.prepend(&Paths::new(&project.tree(run.config)?)?);
    }

    let lua_init = if run.disable_loader.unwrap_or(false) {
        None
    } else if user_tree.version().lux_lib_dir().is_none() {
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

    let status = match Command::new(run.command)
        .args(run.args)
        .env("PATH", paths.path_prepended().joined())
        .env("LUA_INIT", lua_init.unwrap_or_default())
        .env("LUA_PATH", paths.package_path().joined())
        .env("LUA_CPATH", paths.package_cpath().joined())
        .status()
        .await
    {
        Ok(status) => Ok(status),
        Err(err) => Err(ExecError::RunCommandFailed {
            cmd: run.command.to_string(),
            source: err,
        }),
    }?;
    if status.success() {
        Ok(())
    } else {
        Err(ExecError::RunCommandNonZeroExitCode {
            cmd: run.command.to_string(),
            exit_code: status.code(),
        })
    }
}

/// Ensure that a command is installed.
/// This defaults to the local project tree if cwd is a project root.
async fn install_command(command: &str, config: &Config) -> Result<(), InstallCommandError> {
    let install_spec = PackageInstallSpec::new(
        PackageReq::new(command.into(), None)?,
        tree::EntryType::Entrypoint,
    )
    .build();
    let tree = config.user_tree(LuaVersion::from_config(config)?.clone())?;
    Install::new(config)
        .package(install_spec)
        .tree(tree)
        .install()
        .await?;
    Ok(())
}
