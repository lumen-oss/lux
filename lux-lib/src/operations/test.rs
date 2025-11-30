use std::{io, ops::Deref, path::PathBuf, process::Command, sync::Arc};

use crate::{
    build::BuildBehaviour,
    config::{Config, ConfigError},
    lua_installation::{LuaBinary, LuaBinaryError},
    lua_rockspec::{LuaVersionError, TestSpecError, ValidatedTestSpec},
    package::{PackageName, PackageVersionReqError},
    path::{Paths, PathsError},
    progress::{MultiProgress, Progress},
    project::{
        project_toml::LocalProjectTomlValidationError, Project, ProjectError, ProjectTreeError,
    },
    rockspec::Rockspec,
    tree::{self, TreeError},
};
use bon::Builder;
use itertools::Itertools;
use path_slash::PathBufExt;
use thiserror::Error;

use super::{
    BuildProject, BuildProjectError, Install, InstallError, PackageInstallSpec, Sync, SyncError,
};

#[cfg(target_family = "unix")]
const BUSTED_EXE: &str = "busted";
#[cfg(target_family = "windows")]
const BUSTED_EXE: &str = "busted.bat";

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _run, vis = ""))]
pub struct Test<'a> {
    #[builder(start_fn)]
    project: Project,
    #[builder(start_fn)]
    config: &'a Config,

    #[builder(field)]
    args: Vec<String>,

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

#[derive(Error, Debug)]
pub enum RunTestsError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    InstallTestDependencies(#[from] InstallTestDependenciesError),
    #[error("error building project:\n{0}")]
    BuildProject(#[from] BuildProjectError),
    #[error("tests failed!")]
    TestFailure,
    #[error("failed to execute `{0}`: {1}")]
    RunCommandFailure(String, io::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error(transparent)]
    Paths(#[from] PathsError),
    #[error(transparent)]
    Tree(#[from] ProjectTreeError),
    #[error(transparent)]
    ProjectTomlValidation(#[from] LocalProjectTomlValidationError),
    #[error("failed to sync dependencies: {0}")]
    Sync(#[from] SyncError),
    #[error(transparent)]
    TestSpec(#[from] TestSpecError),
    #[error(transparent)]
    LuaVersion(#[from] LuaVersionError),
    #[error(transparent)]
    LuaBinary(#[from] LuaBinaryError),
}

async fn run_tests(test: Test<'_>) -> Result<(), RunTestsError> {
    let rocks = test.project.toml().into_local()?;

    let test_spec = rocks
        .test()
        .current_platform()
        .to_validated(&test.project)?;

    let config = test_spec.test_config(test.config)?;

    let no_lock = test.no_lock.unwrap_or(false);

    let progress = test
        .progress
        .unwrap_or_else(|| MultiProgress::new_arc(test.config));

    if no_lock {
        ensure_test_dependencies(&test.project, &rocks, &config, progress).await?;
    } else {
        Sync::new(&test.project, &config)
            .progress(progress.clone())
            .sync_test_dependencies()
            .await?;
    }

    BuildProject::new(&test.project, &config)
        .no_lock(no_lock)
        .only_deps(false)
        .build()
        .await?;

    let project_tree = test.project.tree(&config)?;
    let test_tree = test.project.test_tree(&config)?;
    let mut paths = Paths::new(&project_tree)?;
    let test_tree_paths = Paths::new(&test_tree)?;
    paths.prepend(&test_tree_paths);

    let test_executable = match &test_spec {
        ValidatedTestSpec::Busted(_) => BUSTED_EXE.to_string(),
        ValidatedTestSpec::BustedNlua(_) => BUSTED_EXE.to_string(),
        ValidatedTestSpec::Command(spec) => spec.command.to_string(),
        ValidatedTestSpec::LuaScript(_) => {
            let lua_version = test.project.lua_version(&config)?;
            let lua_binary = LuaBinary::new(lua_version, &config);
            let lua_bin_path: PathBuf = lua_binary.try_into()?;
            lua_bin_path.to_slash_lossy().to_string()
        }
    };
    let mut command = Command::new(&test_executable);
    let mut command = command
        .current_dir(test.project.root().deref())
        .args(test_spec.args())
        .args(test.args)
        .env("PATH", paths.path_prepended().joined())
        .env("LUA_PATH", paths.package_path().joined())
        .env("LUA_CPATH", paths.package_cpath().joined());
    if let TestEnv::Pure = test.env {
        // isolate the test runner from the user's own config/data files
        // by initialising empty HOME and XDG base directory paths
        let home = test_tree.root().join("home");
        let xdg = home.join("xdg");
        let _ = tokio::fs::remove_dir_all(&home).await;
        let xdg_config_home = xdg.join("config");
        std::fs::create_dir_all(&xdg_config_home)?;
        let xdg_state_home = xdg.join("local").join("state");
        std::fs::create_dir_all(&xdg_state_home)?;
        let xdg_data_home = xdg.join("local").join("share");
        std::fs::create_dir_all(&xdg_data_home)?;
        command = command
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", xdg_config_home)
            .env("XDG_STATE_HOME", xdg_state_home)
            .env("XDG_DATA_HOME", xdg_data_home);
    }
    let status = match command.status() {
        Ok(status) => Ok(status),
        Err(err) => Err(RunTestsError::RunCommandFailure(test_executable, err)),
    }?;
    if status.success() {
        Ok(())
    } else {
        Err(RunTestsError::TestFailure)
    }
}

#[derive(Error, Debug)]
#[error("error installing test dependencies: {0}")]
pub enum InstallTestDependenciesError {
    ProjectTree(#[from] ProjectTreeError),
    Tree(#[from] TreeError),
    Install(#[from] InstallError),
    PackageVersionReq(#[from] PackageVersionReqError),
}

/// Ensure test dependencies are installed
/// This defaults to the local project tree if cwd is a project root.
async fn ensure_test_dependencies(
    project: &Project,
    rockspec: &impl Rockspec,
    config: &Config,
    progress: Arc<Progress<MultiProgress>>,
) -> Result<(), InstallTestDependenciesError> {
    let test_tree = project.test_tree(config)?;
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
        config::{ConfigBuilder, LuaVersion},
        lua_installation::detect_installed_lua_version,
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
        let project_root = temp_dir.path();
        let project: Project = Project::from(project_root).unwrap().unwrap();
        let tree_root = project.root().to_path_buf().join(".lux");
        let _ = tokio::fs::remove_dir_all(&tree_root).await;

        let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

        let config = ConfigBuilder::new()
            .unwrap()
            .user_tree(Some(tree_root))
            .lua_version(lua_version)
            .build()
            .unwrap();

        Test::new(project, &config).run().await.unwrap();
    }
}
