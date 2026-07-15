use itertools::Itertools;
use miette::Diagnostic;
use std::{
    env, io,
    process::{ExitStatus, Stdio},
};
use thiserror::Error;
use tokio::process::Command;

use crate::{
    build::{
        backend::{BuildBackend, BuildInfo, RunBuildArgs},
        utils,
    },
    config::Config,
    lua_rockspec::CMakeBuildSpec,
    path::{Paths, PathsError},
    tree::{InstallTree, TreeError},
    variables::{self, GetVariableError, HasVariables, VariableSubstitutionError},
};

const CMAKE_BUILD_FILE: &str = "build.lux";

#[derive(Error, Debug, Diagnostic)]
pub enum CMakeError {
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
    #[error("failed to run `cmake` step: {0}")]
    Io(io::Error),
    #[error("failed to write CMakeLists.txt: {0}")]
    WriteCmakeListsError(io::Error),
    #[error("failed to run `cmake` step: '{0}' command not found!")]
    #[diagnostic(help("run `lx debug toolchains` to check available build tools."))]
    CommandNotFound(String),
    #[error(transparent)]
    #[diagnostic(transparent)]
    VariableSubstitution(#[from] VariableSubstitutionError),
}

struct CMakeVariables;

impl HasVariables for CMakeVariables {
    fn get_variable(&self, input: &str) -> Result<Option<String>, GetVariableError> {
        Ok(match input {
            "CMAKE_MODULE_PATH" => Some(env::var("CMAKE_MODULE_PATH").unwrap_or("".into())),
            "CMAKE_LIBRARY_PATH" => Some(env::var("CMAKE_LIBRARY_PATH").unwrap_or("".into())),
            "CMAKE_INCLUDE_PATH" => Some(env::var("CMAKE_INCLUDE_PATH").unwrap_or("".into())),
            _ => None,
        })
    }
}

impl BuildBackend for CMakeBuildSpec {
    type Err = CMakeError;

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

        let mut args = Vec::new();
        if let Some(content) = self.cmake_lists_content {
            let cmakelists = build_dir.join("CMakeLists.txt");
            std::fs::write(&cmakelists, content).map_err(CMakeError::WriteCmakeListsError)?;
            args.push(format!("-G\"{}\"", cmakelists.display()));
        } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            // With msvc and x64, CMake does not select it by default so we need to be explicit.
            args.push("-DCMAKE_GENERATOR_PLATFORM=x64".into());
        }
        self.variables
            .into_iter()
            .map(|(key, value)| {
                let substituted_value = utils::substitute_variables(
                    &value,
                    output_paths,
                    lua,
                    external_dependencies,
                    config,
                )?;
                let substituted_value =
                    variables::substitute(&[&CMakeVariables], &substituted_value)?;
                Ok::<_, Self::Err>(format!("{key}={substituted_value}"))
            })
            .fold_ok((), |(), variable| args.push(format!("-D{variable}")))?;

        spawn_cmake_cmd(
            Command::new(config.cmake_cmd())
                .current_dir(build_dir)
                .arg("-H.")
                .arg(format!("-B{CMAKE_BUILD_FILE}"))
                .args(args)
                .env("PATH", &bin_path)
                .env("LUA_PATH", &lua_path)
                .env("LUA_CPATH", &lua_cpath),
            config,
        )
        .await?;

        if self.build_pass {
            spawn_cmake_cmd(
                Command::new(config.cmake_cmd())
                    .current_dir(build_dir)
                    .arg("--build")
                    .arg(CMAKE_BUILD_FILE)
                    .arg("--config")
                    .arg("Release")
                    .env("PATH", &bin_path)
                    .env("LUA_PATH", &lua_path)
                    .env("LUA_CPATH", &lua_cpath),
                config,
            )
            .await?
        }

        if self.install_pass && !no_install {
            spawn_cmake_cmd(
                Command::new(config.cmake_cmd())
                    .current_dir(build_dir)
                    .arg("--build")
                    .arg(CMAKE_BUILD_FILE)
                    .arg("--target")
                    .arg("install")
                    .arg("--config")
                    .arg("Release")
                    .env("PATH", &bin_path)
                    .env("LUA_PATH", &lua_path)
                    .env("LUA_CPATH", &lua_cpath),
                config,
            )
            .await?;
        }

        Ok(BuildInfo::default())
    }
}

async fn spawn_cmake_cmd(cmd: &mut Command, config: &Config) -> Result<(), CMakeError> {
    match cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(child) => match child.wait_with_output().await {
            Ok(output) if output.status.success() => utils::log_command_output(&output, config),
            Ok(output) => {
                return Err(CMakeError::CommandFailure {
                    name: config.cmake_cmd().clone(),
                    status: output.status,
                    stdout: String::from_utf8_lossy(&output.stdout).into(),
                    stderr: String::from_utf8_lossy(&output.stderr).into(),
                });
            }
            Err(err) => return Err(CMakeError::Io(err)),
        },
        Err(_) => return Err(CMakeError::CommandNotFound(config.cmake_cmd().clone())),
    }
    Ok(())
}
