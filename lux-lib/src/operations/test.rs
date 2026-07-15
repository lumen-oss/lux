use std::{io, ops::Deref, path::PathBuf, process::Command, sync::Arc};

use super::{
    BuildWorkspace, BuildWorkspaceError, Install, InstallError, PackageInstallSpec, Sync, SyncError,
};
use crate::tree::InstallTree;
use crate::workspace::{WorkspaceError, WorkspaceTreeError};
use crate::{
    build::BuildBehaviour,
    config::{Config, ConfigError},
    lua_installation::{LuaBinary, LuaBinaryError},
    lua_rockspec::{LuaVersionError, TestSpecError, ValidatedTestSpec},
    package::{PackageName, PackageVersionReqError},
    path::{Paths, PathsError},
    progress::{MultiProgress, Progress},
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
    progress: Option<Arc<Progress<MultiProgress>>>,
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
    InstallTestDependencies(#[from] InstallTestDependenciesError),
    #[error("build failed")]
    #[diagnostic(forward(0))]
    BuildWorkspace(#[from] BuildWorkspaceError),
    #[error("tests failed!")]
    TestFailure,
    #[error("failed to execute '{cmd}'")]
    RunCommandFailure {
        cmd: String,
        source: io::Error,
        #[help]
        help: Option<String>,
    },
    #[error(transparent)]
    Io(#[from] io::Error),
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
    Sync(#[from] SyncError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    TestSpec(#[from] TestSpecError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaVersion(#[from] LuaVersionError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaBinary(#[from] LuaBinaryError),
}

async fn run_tests(test: Test<'_>) -> Result<(), RunTestsError> {
    let workspace = test.workspace;
    let config = test.config;
    let progress = test
        .progress
        .unwrap_or_else(|| MultiProgress::new_arc(config));
    let no_lock = test.no_lock.unwrap_or(false);

    if let Some(package) = test.package {
        let project = workspace.select_member(&package)?;
        let progress = Arc::clone(&progress);
        run_project_tests(
            &workspace, project, no_lock, &test.args, &test.env, progress, config,
        )
        .await
    } else {
        for project in workspace.members() {
            let progress = Arc::clone(&progress);
            run_project_tests(
                &workspace, project, no_lock, &test.args, &test.env, progress, config,
            )
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
    test_env: &TestEnv,
    progress: Arc<Progress<MultiProgress>>,
    config: &Config,
) -> Result<(), RunTestsError> {
    let rocks = project.toml().into_local()?;
    let test_spec = rocks.test().current_platform().to_validated(project)?;
    let test_config = test_spec.test_config(config)?;

    if no_lock {
        let rockspec = project.toml().into_local()?;
        ensure_test_dependencies(workspace, project, rockspec, &test_config, progress.clone())
            .await?;
    } else {
        Sync::new(workspace, &test_config)
            .progress(progress.clone())
            .sync_test_dependencies()
            .await?;
    }

    BuildWorkspace::new(workspace, &test_config)
        .package(project.toml().package().clone())
        .no_lock(no_lock)
        .only_deps(false)
        .build()
        .await?;

    let lua_version = project.lua_version(&test_config)?;
    let project_tree = workspace.lua_version_tree(lua_version, &test_config)?;
    let test_tree = workspace.test_tree(&test_config)?;
    let mut paths = Paths::new(&project_tree)?;
    let test_tree_paths = Paths::new(&test_tree)?;
    paths.prepend(&test_tree_paths);

    let test_executable = match &test_spec {
        ValidatedTestSpec::Busted { .. } => BUSTED_EXE.to_string(),
        ValidatedTestSpec::BustedNlua { .. } => BUSTED_EXE.to_string(),
        ValidatedTestSpec::Command(spec) => spec.command.to_string(),
        ValidatedTestSpec::LuaScript(_) => {
            let lua_version = project.lua_version(&test_config)?;
            let lua_binary = LuaBinary::new(lua_version, &test_config);
            let lua_bin_path: PathBuf = lua_binary.try_into()?;
            lua_bin_path.to_slash_lossy().to_string()
        }
    };
    let mut command = Command::new(&test_executable);
    let mut command = command
        .current_dir(project.root().deref())
        .args(test_spec.args())
        .args(test_args)
        .env("PATH", paths.path_prepended().joined())
        .env("LUA_PATH", paths.package_path().joined())
        .env("LUA_CPATH", paths.package_cpath().joined());
    if let TestEnv::Pure = test_env {
        // isolate the test runner from the user's own config/data files
        // by initialising empty HOME and XDG base directory paths
        let home = test_tree.root().join("home");
        let xdg = home.join("xdg");
        let _ = tokio::fs::remove_dir_all(&home).await;
        let xdg_config_home = xdg.join("config");
        tokio::fs::create_dir_all(&xdg_config_home).await?;
        let xdg_state_home = xdg.join("local").join("state");
        tokio::fs::create_dir_all(&xdg_state_home).await?;
        let xdg_data_home = xdg.join("local").join("share");
        tokio::fs::create_dir_all(&xdg_data_home).await?;
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
#[error("error installing test dependencies: {0}")]
#[diagnostic(forward(0))]
pub enum InstallTestDependenciesError {
    WorkspaceTree(#[from] WorkspaceTreeError),
    Tree(#[from] TreeError),
    Install(#[from] InstallError),
    PackageVersionReq(#[from] PackageVersionReqError),
}

/// Ensure test dependencies are installed
/// This defaults to the local project tree if cwd is a project root.
async fn ensure_test_dependencies(
    workspace: &Workspace,
    project: &Project,
    rockspec: impl Rockspec,
    config: &Config,
    progress: Arc<Progress<MultiProgress>>,
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
        .progress(progress.clone())
        .install()
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::{
        config::ConfigBuilder, lua_installation::detect_installed_lua_version,
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
        let _ = tokio::fs::remove_dir_all(&tree_root).await;

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
