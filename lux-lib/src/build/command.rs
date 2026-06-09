use std::{
    collections::HashMap,
    io,
    path::Path,
    process::{ExitStatus, Stdio},
};
use thiserror::Error;
use tokio::process::Command;
use which::which;

use crate::{
    build::backend::{BuildBackend, BuildInfo, RunBuildArgs},
    config::Config,
    lua_installation::LuaInstallation,
    lua_rockspec::CommandBuildSpec,
    path::{Paths, PathsError},
    tree::{InstallTree, RockLayout, TreeError},
    variables::VariableSubstitutionError,
};

use super::external_dependency::ExternalDependencyInfo;
use super::utils;

#[derive(Error, Debug)]
pub enum CommandError {
    #[error(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    Paths(#[from] PathsError),
    #[error("'build_command' and 'install_command' cannot be empty.")]
    EmptyCommand,
    #[error("error parsing command:\n{command}\n\nerror: {err}")]
    ParseError {
        err: shell_words::ParseError,
        command: String,
    },
    #[error("cannot find a shell to execute the command:\n{0}")]
    ShellNotFoundError(#[from] which::Error),
    #[error("error executing command:\n{command}\n\nerror: {err}")]
    Io { err: io::Error, command: String },
    #[error("failed to execute command:\n{command}\n\nstatus: {status}\nstdout: {stdout}\nstderr: {stderr}")]
    CommandFailure {
        command: String,
        status: ExitStatus,
        stdout: String,
        stderr: String,
    },
    #[error(transparent)]
    VariableSubstitutionError(#[from] VariableSubstitutionError),
}

impl BuildBackend for CommandBuildSpec {
    type Err = CommandError;

    async fn run<T>(self, args: RunBuildArgs<'_, T>) -> Result<BuildInfo, Self::Err>
    where
        T: InstallTree,
    {
        let output_paths = args.output_paths;
        let no_install = args.no_install;
        let lua = args.lua;
        let external_dependencies = args.external_dependencies;
        let config = args.config;
        let build_dir = args.build_dir;
        let progress = args.progress;

        let build_tree = args.tree.build_tree(config)?;
        let build_paths = Paths::new(&build_tree)?;

        progress.map(|bar| bar.set_message("Running build_command..."));
        if let Some(build_command) = &self.build_command {
            progress.map(|bar| bar.set_message("Running build_command..."));
            run_command(
                build_command,
                output_paths,
                lua,
                external_dependencies,
                config,
                build_dir,
                &build_paths,
            )
            .await?;
        }
        if !no_install {
            if let Some(install_command) = &self.install_command {
                progress.map(|bar| bar.set_message("Running install_command..."));
                run_command(
                    install_command,
                    output_paths,
                    lua,
                    external_dependencies,
                    config,
                    build_dir,
                    &build_paths,
                )
                .await?;
            }
        }
        Ok(BuildInfo::default())
    }
}

async fn run_command(
    command: &str,
    output_paths: &RockLayout,
    lua: &LuaInstallation,
    external_dependencies: &HashMap<String, ExternalDependencyInfo>,
    config: &Config,
    build_dir: &Path,
    build_paths: &Paths,
) -> Result<(), CommandError> {
    let lua_path = build_paths.package_path_prepended().joined();
    let lua_cpath = build_paths.package_cpath_prepended().joined();
    let bin_path = build_paths.path_prepended().joined();
    let substituted_cmd =
        utils::substitute_variables(command, output_paths, lua, external_dependencies, config)?;

    if substituted_cmd.is_empty() {
        return Err(CommandError::EmptyCommand);
    }

    #[cfg(target_env = "msvc")]
    let (shell, shell_arg) = (which("cmd.exe")?, "/C");
    #[cfg(not(target_env = "msvc"))]
    let (shell, shell_arg) = (which("sh")?, "-c");

    match Command::new(shell)
        .arg(shell_arg)
        .arg(&substituted_cmd)
        .current_dir(build_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PATH", &bin_path)
        .env("LUA_PATH", &lua_path)
        .env("LUA_CPATH", &lua_cpath)
        .spawn()
    {
        Err(err) => {
            return Err(CommandError::Io {
                err,
                command: substituted_cmd,
            })
        }
        Ok(child) => match child.wait_with_output().await {
            Ok(output) if output.status.success() => utils::log_command_output(&output, config),
            Ok(output) => {
                return Err(CommandError::CommandFailure {
                    command: substituted_cmd,
                    status: output.status,
                    stdout: String::from_utf8_lossy(&output.stdout).into(),
                    stderr: String::from_utf8_lossy(&output.stderr).into(),
                });
            }
            Err(err) => {
                return Err(CommandError::Io {
                    err,
                    command: substituted_cmd,
                })
            }
        },
    }
    Ok(())
}
