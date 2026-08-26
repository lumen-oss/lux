use std::{io, ops::Deref, process::Command};

use super::{
    BuildWorkspace, BuildWorkspaceError, Install, InstallError, PackageInstallSpec, Sync, SyncError,
};
pub(crate) mod tiniest;
use crate::fs;
use crate::tree::InstallTree;
use crate::workspace::{WorkspaceError, WorkspaceTreeError};
use crate::{
    build::BuildBehaviour,
    config::{Config, ConfigError},
    lua_installation::LuaBinaryError,
    lua_rockspec::{LuaVersionError, TestSpecError, ValidatedTestSpec},
    lua_version::LuaVersion,
    package::{PackageName, PackageVersionReqError},
    path::{Paths, PathsError},
    project::{project_toml::LocalProjectTomlValidationError, Project, ProjectError},
    rockspec::Rockspec,
    tree::{self, TreeError},
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
    package: Option<PackageName>,

    no_lock: Option<bool>,

    #[builder(default)]
    env: TestEnv,
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

    pub async fn run(self) -> Result<(), RunTestsError>
    where
        State: test_builder::IsComplete,
    {
        run_tests(self._run()).await
    }
}

#[derive(Default)]
pub enum TestEnv {
    /// An environment that is isolated from `HOME` and `XDG` base directories (default).
    #[default]
    Pure,
    /// An impure environment in which `HOME` and `XDG` base directories can influence
    /// the test results.
    Impure,
}

#[derive(Error, Debug, Diagnostic)]
pub enum RunTestsError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    InstallTestDependencies(#[from] Box<InstallTestDependenciesError>),
    #[error("build failed")]
    #[diagnostic(forward(0))]
    BuildWorkspace(#[from] Box<BuildWorkspaceError>),
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
    Fs(#[from] fs::FsError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Project(#[from] ProjectError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Paths(#[from] PathsError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] WorkspaceTreeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    ProjectTomlValidation(#[from] LocalProjectTomlValidationError),
    #[error("failed to sync dependencies")]
    #[diagnostic(forward(0))]
    Sync(#[from] Box<SyncError>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    TestSpec(#[from] TestSpecError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaVersion(#[from] LuaVersionError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaBinary(#[from] LuaBinaryError),
    #[error("failed to set up tiniest")]
    #[diagnostic(forward(0))]
    Tiniest(#[from] tiniest::TiniestError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    RunLua(#[from] super::run_lua::RunLuaError),
}

impl From<InstallTestDependenciesError> for RunTestsError {
    fn from(source: InstallTestDependenciesError) -> Self {
        Self::InstallTestDependencies(Box::new(source))
    }
}

impl From<BuildWorkspaceError> for RunTestsError {
    fn from(source: BuildWorkspaceError) -> Self {
        Self::BuildWorkspace(Box::new(source))
    }
}

impl From<SyncError> for RunTestsError {
    fn from(source: SyncError) -> Self {
        Self::Sync(Box::new(source))
    }
}

#[tracing::instrument(name = "🧪 Running tests", skip_all)]

async fn run_tests(test: Test<'_>) -> Result<(), RunTestsError> {
    let workspace = test.workspace;
    let config = test.config;
    let no_lock = test.no_lock.unwrap_or(false);

    if let Some(package) = test.package {
        let project = workspace.select_member(&package)?;
        run_project_tests(&workspace, project, no_lock, &test.args, &test.env, config).await
    } else {
        for project in workspace.members() {
            run_project_tests(&workspace, project, no_lock, &test.args, &test.env, config).await?;
        }
        Ok(())
    }
}

async fn run_project_tests(
    workspace: &Workspace,
    project: &Project,
    no_lock: bool,
    test_args: &[String],
    test_env: &TestEnv,
    config: &Config,
) -> Result<(), RunTestsError> {
    let rocks = project.toml().into_local()?;
    let test_spec = match rocks.test().current_platform().to_validated(project) {
        Ok(test_spec) => test_spec,
        Err(TestSpecError::NoTestSpecDetected)
            if project.lua_version(config)? == LuaVersion::Luau =>
        {
            // tiniest is the default test framework for Luau projects
            ValidatedTestSpec::Tiniest
        }
        Err(err) => return Err(err.into()),
    };
    let test_config = test_spec.test_config(config)?;

    if no_lock {
        let rockspec = project.toml().into_local()?;
        ensure_test_dependencies(workspace, project, rockspec, &test_config).await?;
    } else {
        Sync::new(workspace, &test_config).test(true).sync().await?;
    }

    BuildWorkspace::new(workspace, &test_config)
        .package(project.toml().package().clone())
        .no_lock(no_lock)
        .only_deps(false)
        .build()
        .await?;

    let lua_version = project.lua_version(&test_config)?.clone();
    let project_tree = workspace.lua_version_tree(lua_version.clone(), &test_config)?;
    let test_tree = workspace.test_tree(&test_config)?;
    let mut paths = Paths::new(&project_tree)?;
    let test_tree_paths = Paths::new(&test_tree)?;
    paths.prepend(&test_tree_paths);

    let collector_path = match &test_spec {
        ValidatedTestSpec::Tiniest => {
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
        ValidatedTestSpec::Busted { .. } => BUSTED_EXE.to_string(),
        ValidatedTestSpec::BustedNlua { .. } => BUSTED_EXE.to_string(),
        ValidatedTestSpec::Command(spec) => spec.command.to_string(),
        ValidatedTestSpec::Tiniest | ValidatedTestSpec::LuaScript(_) => {
            let runtime =
                super::run_lua::resolve_lua_runtime(&lua_version.clone(), &test_config).await?;
            runtime.to_string_lossy().to_string()
        }
    };
    let mut command = Command::new(&test_executable);
    let mut command = command
        .current_dir(project.root().deref())
        .args(runner_args)
        .args(test_args)
        .env("PATH", paths.path_prepended().joined())
        .env("LUA_PATH", paths.package_path().joined())
        .env("LUA_CPATH", paths.package_cpath().joined());
    if let TestEnv::Pure = test_env {
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
            Err(RunTestsError::RunCommandFailure {
                cmd: test_executable,
                source: err,
                help,
            })
        }
    }?;
    if !status.success() {
        Err(RunTestsError::TestFailure)
    } else {
        Ok(())
    }
}

#[derive(Error, Debug, Diagnostic)]
#[error("error installing test dependencies")]
#[diagnostic(forward(0))]
pub enum InstallTestDependenciesError {
    WorkspaceTree(#[from] WorkspaceTreeError),
    Tree(#[from] TreeError),
    Install(#[from] Box<InstallError>),
    PackageVersionReq(#[from] PackageVersionReqError),
}

impl From<InstallError> for InstallTestDependenciesError {
    fn from(source: InstallError) -> Self {
        Self::Install(Box::new(source))
    }
}

/// Ensure test dependencies are installed
/// This defaults to the local project tree if cwd is a project root.
async fn ensure_test_dependencies(
    workspace: &Workspace,
    project: &Project,
    rockspec: impl Rockspec,
    config: &Config,
) -> Result<(), InstallTestDependenciesError> {
    let test_tree = workspace.test_tree(config)?;
    let rockspec_dependencies = rockspec.test_dependencies().current_platform();
    let test_dependencies = rockspec
        .test()
        .current_platform()
        .test_dependencies(project)
        .iter()
        .filter(|test_dep| {
            !rockspec_dependencies
                .iter()
                .any(|dep| dep.name() == test_dep.name())
        })
        .filter_map(|dep| {
            let build_behaviour = if test_tree
                .match_rocks(dep)
                .is_ok_and(|matches| matches.is_found())
            {
                Some(BuildBehaviour::NoForce)
            } else {
                Some(BuildBehaviour::Force)
            };
            build_behaviour.map(|build_behaviour| {
                PackageInstallSpec::new(dep.clone(), tree::EntryType::Entrypoint)
                    .build_behaviour(build_behaviour)
                    .build()
            })
        })
        .chain(
            rockspec_dependencies
                .iter()
                .filter(|req| !req.name().eq(&PackageName::new("lua".into())))
                .filter_map(|dep| {
                    let build_behaviour = if test_tree
                        .match_rocks(dep.package_req())
                        .is_ok_and(|matches| matches.is_found())
                    {
                        Some(BuildBehaviour::NoForce)
                    } else {
                        Some(BuildBehaviour::Force)
                    };
                    build_behaviour.map(|build_behaviour| {
                        PackageInstallSpec::new(
                            dep.package_req().clone(),
                            tree::EntryType::Entrypoint,
                        )
                        .build_behaviour(build_behaviour)
                        .pin(*dep.pin())
                        .opt(*dep.opt())
                        .maybe_source(dep.source.clone())
                        .build()
                    })
                }),
        )
        .collect();

    Install::new(config)
        .packages(test_dependencies)
        .tree(test_tree)
        .install()
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::{
        config::ConfigBuilder, fs, lua_installation::detect_installed_lua_version,
        lua_version::LuaVersion,
    };

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

        let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

        let config = ConfigBuilder::new()
            .unwrap()
            .user_tree(Some(tree_root))
            .lua_version(lua_version)
            .build()
            .unwrap();

        Test::new(workspace, &config).run().await.unwrap();
    }
}
