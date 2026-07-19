use itertools::Itertools;
use miette::Diagnostic;
use path_slash::PathBufExt;
use std::{
    io,
    path::PathBuf,
    process::{ExitStatus, Stdio},
};
use thiserror::Error;
use tokio::process::Command;
use tracing::{info_span, Instrument};

use crate::{
    build::{
        backend::{BuildBackend, BuildInfo, RunBuildArgs},
        utils,
    },
    lua_rockspec::MakeBuildSpec,
    path::{Paths, PathsError},
    tree::{InstallTree, TreeError},
    variables::VariableSubstitutionError,
};

#[derive(Error, Debug, Diagnostic)]
pub enum MakeError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Paths(#[from] PathsError),
    #[error("{name} step failed.\n\n{status}\n\nstdout:\n{stdout}\n\nstderr:\n{stderr}")]
    CommandFailure {
        name: String,
        status: ExitStatus,
        stdout: String,
        stderr: String,
    },
    #[error("failed to run `make` step: {0}")]
    Io(io::Error),
    #[error("failed to run `make` step: '{0}' command not found!")]
    #[diagnostic(help("run `lx debug toolchains` to check available build tools."))]
    CommandNotFound(String),
    #[error(transparent)]
    #[diagnostic(transparent)]
    VariableSubstitution(#[from] VariableSubstitutionError),
    #[error("{0} not found")]
    #[diagnostic(help(
        "ensure the Makefile exists in the build directory or check the rockspec source layout."
    ))]
    MakefileNotFound(PathBuf),
}

impl BuildBackend for MakeBuildSpec {
    type Err = MakeError;

    #[tracing::instrument(name = "🛠️ make::run", skip_all, level = "debug")]
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

        let build_tree = args.tree.build_tree(config)?;
        let build_paths = Paths::new(&build_tree)?;
        let lua_path = build_paths.package_path_prepended().joined();
        let lua_cpath = build_paths.package_cpath_prepended().joined();
        let bin_path = build_paths.path_prepended().joined();

        // Build step
        if self.build_pass {
            let build_args = self
                .variables
                .iter()
                .chain(&self.build_variables)
                .filter(|(_, value)| !value.is_empty())
                .map(|(key, value)| {
                    let substituted_value = utils::substitute_variables(
                        value,
                        output_paths,
                        lua,
                        external_dependencies,
                        config,
                    )?;
                    Ok(format!("{key}={substituted_value}").trim().to_string())
                })
                .try_collect::<_, Vec<_>, Self::Err>()?;
            let mut cmd = Command::new(config.make_cmd());
            if let Some(build_target) = &self.build_target {
                cmd.arg(build_target);
            }
            let span = info_span!("🛠️ Make build pass");
            match cmd
                .current_dir(build_dir)
                .args(["-f", &self.makefile.to_slash_lossy()])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .args(build_args)
                .env("PATH", &bin_path)
                .env("LUA_PATH", &lua_path)
                .env("LUA_CPATH", &lua_cpath)
                .spawn()
            {
                Ok(child) => match child.wait_with_output().instrument(span).await {
                    Ok(output) if output.status.success() => utils::trace_command_output(&output),
                    Ok(output) => {
                        return Err(MakeError::CommandFailure {
                            name: match self.build_target {
                                Some(build_target) => {
                                    format!("{} {}", config.make_cmd(), build_target)
                                }
                                None => config.make_cmd(),
                            },

                            status: output.status,
                            stdout: String::from_utf8_lossy(&output.stdout).into(),
                            stderr: String::from_utf8_lossy(&output.stderr).into(),
                        });
                    }
                    Err(err) => return Err(MakeError::Io(err)),
                },
                Err(_) => return Err(MakeError::CommandNotFound(config.make_cmd().clone())),
            }
        };

        // Install step
        if self.install_pass && !no_install {
            let install_args = self
                .variables
                .iter()
                .chain(&self.install_variables)
                .filter(|(_, value)| !value.is_empty())
                .map(|(key, value)| {
                    let substituted_value = utils::substitute_variables(
                        value,
                        output_paths,
                        lua,
                        external_dependencies,
                        config,
                    )?;
                    Ok(format!("{key}={substituted_value}").trim().to_string())
                })
                .try_collect::<_, Vec<_>, Self::Err>()?;
            let span = info_span!("🛠️ Make install pass");
            match Command::new(config.make_cmd())
                .current_dir(build_dir)
                .arg(&self.install_target)
                .args(["-f", &self.makefile.to_slash_lossy()])
                .args(install_args)
                .env("PATH", &bin_path)
                .env("LUA_PATH", &lua_path)
                .env("LUA_CPATH", &lua_cpath)
                .output()
                .instrument(span)
                .await
            {
                Ok(output) if output.status.success() => utils::trace_command_output(&output),
                Ok(output) => {
                    return Err(MakeError::CommandFailure {
                        name: format!("{} {}", config.make_cmd(), self.install_target),
                        status: output.status,
                        stdout: String::from_utf8_lossy(&output.stdout).into(),
                        stderr: String::from_utf8_lossy(&output.stderr).into(),
                    })
                }
                Err(err) => return Err(MakeError::Io(err)),
            }
        };

        Ok(BuildInfo::default())
    }
}
