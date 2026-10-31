use crate::package;
use crate::rockspec;
use std::{io, process::Command};

use crate::drivers::sync::{self, Sync, SyncMode, TargetSet};

pub(crate) mod tiniest;
use crate::fs;
use crate::tree::InstallTree;
use crate::workspace::{self, TreeError};
use crate::{
    build::BuildBehaviour,
    config::{self, Config},
    lua,
    path::{self, Paths},
    project::{self, Project},
    rockspec::Rockspec,
    workspace::Workspace,
};
use bon::Builder;
use itertools::Itertools;
use miette::Diagnostic;
use path_slash::PathBufExt;
use thiserror::Error;

#[cfg(target_family = "unix")]
const BUSTED_EXE: &str = "busted";
#[cfg(target_family = "windows")]
const BUSTED_EXE: &str = "busted.bat";

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _run, vis = ""))]
pub struct Test<'a> {
    #[builder(start_fn)]
    workspace: Workspace,
    #[builder(start_fn)]
    config: &'a Config,

    #[builder(field)]
    args: Vec<String>,

    /// Package to run tests for
    package: Option<package::Name>,

    no_lock: Option<bool>,

    #[builder(default)]
    env: Env,
}

impl<State: test_builder::State> TestBuilder<'_, State> {
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args(mut self, args: impl IntoIterator<Item: Into<String>>) -> Self {
        self.args.extend(args.into_iter().map_into());
        self
    }

    pub async fn run(self) -> Result<(), Error>
    where
        State: test_builder::IsComplete,
    {
        run_tests(self._run()).await
    }
}

#[derive(Default)]
pub enum Env {
    /// An environment that is isolated from `HOME` and `XDG` base directories (default).
    #[default]
    Pure,
    /// An impure environment in which `HOME` and `XDG` base directories can influence
    /// the test results.
    Impure,
}

#[derive(Error, Debug, Diagnostic)]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] config::Error),
    #[error("build failed")]
    #[diagnostic(forward(0))]
    Sync(#[from] Box<sync::Error>),
    #[error("tests failed!")]
    #[diagnostic(help("see the test runner's output for details"))]
    TestFailure,
    #[error("failed to execute '{cmd}'")]
    RunCommandFailure {
        cmd: String,
        source: io::Error,
        #[help]
        help: Option<String>,
    },
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Project(#[from] project::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Paths(#[from] path::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Workspace(#[from] workspace::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    ProjectTomlValidation(#[from] rockspec::toml::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Test(#[from] rockspec::test::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaVersion(#[from] rockspec::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaBinary(#[from] lua::installation::Error),
    #[error("failed to set up tiniest")]
    #[diagnostic(forward(0))]
    Tiniest(#[from] tiniest::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    RunLua(#[from] super::run_lua::Error),
}

impl From<sync::Error> for Error {
    fn from(source: sync::Error) -> Self {
        Self::Sync(Box::new(source))
    }
}

#[tracing::instrument(name = "🧪 Running tests", skip_all)]

async fn run_tests(test: Test<'_>) -> Result<(), Error> {
    let workspace = test.workspace;
    let config = test.config;
    let no_lock = test.no_lock.unwrap_or(false);

    if let Some(package) = test.package {
        let project = workspace.select_member(&package)?;
        Box::pin(run_project_tests(
            &workspace, project, no_lock, &test.args, &test.env, config,
        ))
        .await
    } else {
        for project in workspace.members() {
            Box::pin(run_project_tests(
                &workspace, project, no_lock, &test.args, &test.env, config,
            ))
            .await?;
        }
        Ok(())
    }
}

async fn run_project_tests(
    workspace: &Workspace,
    project: &Project,
    no_lock: bool,
    test_args: &[String],
    test_env: &Env,
    config: &Config,
) -> Result<(), Error> {
    let rocks = project.toml().into_local()?;
    let test_spec = match rocks.test().current_platform().to_validated(project) {
        Ok(test_spec) => test_spec,
        Err(rockspec::test::Error::NoTestSpecDetected)
            if project.lua_version(config)? == lua::Version::Luau =>
        {
            // tiniest is the default test framework for Luau projects
            rockspec::test::internal::Validated::Tiniest
        }
        Err(err) => return Err(err.into()),
    };
    let test_config = test_spec.test_config(config)?;

    Sync::new(workspace, &test_config)
        .mode(SyncMode::Open)
        .targets(TargetSet::member(project.toml().package().clone(), true))
        .no_lock(no_lock)
        .behaviour(BuildBehaviour::Ignore)
        .sync()
        .await?;

    let lua_version = project.lua_version(&test_config)?.clone();
    let project_tree = workspace.lua_version_tree(lua_version.clone(), &test_config)?;
    let test_tree = workspace.test_tree(&test_config)?;
    let mut paths = Paths::new(&project_tree)?;
    let test_tree_paths = Paths::new(&test_tree)?;
    paths.prepend(&test_tree_paths);

    let collector_path = match &test_spec {
        rockspec::test::internal::Validated::Tiniest => {
            let spec_files = tiniest::discover_spec_files(project.root());
            if spec_files.is_empty() {
                tracing::warn!("no *.spec.luau or *.spec.lua files found in test/ or tests/");
            }
            let tiniest_src = tiniest::ensure_installed(&test_tree, &test_config).await?;
            Some(tiniest::write_collector(workspace.root(), &spec_files, &tiniest_src).await?)
        }
        _ => None,
    };
    let mut runner_args = test_spec.args();
    if let Some(collector_path) = &collector_path {
        runner_args.push(collector_path.to_slash_lossy().to_string());
    }

    let test_executable = match &test_spec {
        rockspec::test::internal::Validated::Busted { .. } => BUSTED_EXE.to_string(),
        rockspec::test::internal::Validated::BustedNlua { .. } => BUSTED_EXE.to_string(),
        rockspec::test::internal::Validated::Command(spec) => spec.command.to_string(),
        rockspec::test::internal::Validated::Tiniest
        | rockspec::test::internal::Validated::LuaScript(_) => {
            let runtime =
                super::run_lua::resolve_runtime(&lua_version.clone(), &test_config).await?;
            runtime.to_string_lossy().to_string()
        }
    };
    let mut command = Command::new(&test_executable);
    let mut command = command
        .current_dir(project.root())
        .args(runner_args)
        .args(test_args)
        .env("PATH", paths.path_prepended().joined())
        .env("LUA_PATH", paths.package_path().joined())
        .env("LUA_CPATH", paths.package_cpath().joined());
    if matches!(
        &test_spec,
        rockspec::test::internal::Validated::Busted { .. }
            | rockspec::test::internal::Validated::BustedNlua { .. }
    ) && !test_spec
        .args()
        .iter()
        .chain(test_args.iter())
        .any(|arg| arg == "--directory" || arg == "-C" || arg.starts_with("--directory="))
    {
        let directory = fs::sync::absolute(project.root())?;
        command = command
            .arg("--directory")
            .arg(directory.to_slash_lossy().to_string());
    }
    if let Env::Pure = test_env {
        // isolate the test runner from the user's own config/data files
        // by initialising empty HOME and XDG base directory paths
        let home = test_tree.root().join("home");
        let xdg = home.join("xdg");
        let _ = fs::tokio::remove_dir_all(&home).await;
        let xdg_config_home = xdg.join("config");
        fs::tokio::create_dir_all(&xdg_config_home).await?;
        let xdg_state_home = xdg.join("local").join("state");
        fs::tokio::create_dir_all(&xdg_state_home).await?;
        let xdg_data_home = xdg.join("local").join("share");
        fs::tokio::create_dir_all(&xdg_data_home).await?;
        command = command
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", xdg_config_home)
            .env("XDG_STATE_HOME", xdg_state_home)
            .env("XDG_DATA_HOME", xdg_data_home);
    }
    let status = match command.status() {
        Ok(status) => Ok(status),
        Err(err) => {
            let help = if err.to_string().starts_with("No such file") {
                Some(format!(
                    "make sure '{}' is available on your PATH",
                    test_executable
                ))
            } else {
                None
            };
            Err(Error::RunCommandFailure {
                cmd: test_executable,
                source: err,
                help,
            })
        }
    }?;
    if !status.success() {
        Err(Error::TestFailure)
    } else {
        Ok(())
    }
}

#[cfg(all(test, feature = "impure_tests"))]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::{config, fs, lua};

    use super::*;
    use assert_fs::{prelude::PathCopy, TempDir};

    #[tokio::test]
    async fn test_command_spec() {
        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample-projects/command-test/");
        run_test(&project_root).await
    }

    #[tokio::test]
    async fn test_lua_script_spec() {
        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample-projects/lua-script-test/");
        run_test(&project_root).await
    }

    async fn run_test(project_root: &Path) {
        let temp_dir = TempDir::new().unwrap();
        temp_dir.copy_from(project_root, &["**"]).unwrap();
        let workspace_root = temp_dir.path();
        let workspace = Workspace::from(workspace_root).unwrap().unwrap();
        let tree_root = workspace.root().to_path_buf().join(".lux");
        let _ = fs::tokio::remove_dir_all(&tree_root).await;

        let lua_version = lua::detect::installed_version().or(Some(lua::Version::Lua51));

        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(tree_root))
            .lua_version(lua_version)
            .build()
            .unwrap();

        Test::new(workspace, &config).run().await.unwrap();
    }
}
