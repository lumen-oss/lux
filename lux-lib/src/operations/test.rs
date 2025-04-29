use std::{io, ops::Deref, process::Command, sync::Arc};

use crate::{
    build::BuildBehaviour,
    config::Config,
    package::{PackageName, PackageReq, PackageVersionReqError},
    path::{Paths, PathsError},
    progress::{MultiProgress, Progress},
    project::{
        project_toml::LocalProjectTomlValidationError, Project, ProjectError, ProjectTreeError,
    },
    rockspec::Rockspec,
    tree::{self, Tree, TreeError},
};
use bon::Builder;
use itertools::Itertools;
use thiserror::Error;

use super::{Install, InstallError, PackageInstallSpec, Sync, SyncError};

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
    #[builder(default = MultiProgress::new_arc())]
    progress: Arc<Progress<MultiProgress>>,
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

pub enum TestEnv {
    /// An environment that is isolated from `HOME` and `XDG` base directories (default).
    Pure,
    /// An impure environment in which `HOME` and `XDG` base directories can influence
    /// the test results.
    Impure,
}

impl Default for TestEnv {
    fn default() -> Self {
        Self::Pure
    }
}

#[derive(Error, Debug)]
pub enum RunTestsError {
    #[error(transparent)]
    InstallTestDependencies(#[from] InstallTestDependenciesError),
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
}

async fn run_tests(test: Test<'_>) -> Result<(), RunTestsError> {
    let rocks = test.project.toml().into_local()?;
    let project_tree = test.project.tree(test.config)?;
    let test_tree = test.project.test_tree(test.config)?;
    std::fs::create_dir_all(test_tree.root())?;
    // TODO(#204): Only ensure busted if running with busted (e.g. a .busted directory exists)
    if test.no_lock.unwrap_or(false) {
        ensure_dependencies(
            &rocks,
            &project_tree,
            &test_tree,
            test.config,
            test.progress,
        )
        .await?;
    } else {
        let mut lockfile = test.project.lockfile()?.write_guard();

        let test_dependencies = rocks
            .test_dependencies()
            .current_platform()
            .iter()
            .cloned()
            .collect_vec();

        Sync::new(&test_tree, &mut lockfile, test.config)
            .progress(test.progress.clone())
            .packages(test_dependencies)
            .sync_test_dependencies()
            .await?;

        let dependencies = rocks
            .dependencies()
            .current_platform()
            .iter()
            .filter(|req| !req.name().eq(&PackageName::new("lua".into())))
            .cloned()
            .collect_vec();

        Sync::new(&project_tree, &mut lockfile, test.config)
            .progress(test.progress.clone())
            .packages(dependencies)
            .sync_dependencies()
            .await?;
    }
    let test_tree_root = &test_tree.root().clone();
    let mut paths = Paths::new(&project_tree)?;
    let test_tree_paths = Paths::new(&test_tree)?;
    paths.prepend(&test_tree_paths);

    let mut command = Command::new(BUSTED_EXE);
    let mut command = command
        .current_dir(test.project.root().deref())
        .args(test.args)
        .env("PATH", paths.path_prepended().joined())
        .env("LUA_PATH", paths.package_path().joined())
        .env("LUA_CPATH", paths.package_cpath().joined());
    if let TestEnv::Pure = test.env {
        // isolate the test runner from the user's own config/data files
        // by initialising empty HOME and XDG base directory paths
        let home = test_tree_root.join("home");
        let xdg = home.join("xdg");
        let _ = std::fs::remove_dir_all(&home);
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
        Err(err) => Err(RunTestsError::RunCommandFailure("busted".into(), err)),
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
    Tree(#[from] TreeError),
    Install(#[from] InstallError),
    PackageVersionReq(#[from] PackageVersionReqError),
}

/// Ensure that busted is installed.
/// This defaults to the local project tree if cwd is a project root.
pub async fn ensure_busted(
    tree: &Tree,
    config: &Config,
    progress: Arc<Progress<MultiProgress>>,
) -> Result<(), InstallTestDependenciesError> {
    let busted_req = PackageReq::new("busted".into(), None)?;

    if !tree.match_rocks(&busted_req)?.is_found() {
        let install_spec = PackageInstallSpec::new(busted_req, tree::EntryType::Entrypoint).build();
        Install::new(tree, config)
            .package(install_spec)
            .progress(progress)
            .install()
            .await?;
    }

    Ok(())
}

/// Ensure dependencies and test dependencies are installed
/// This defaults to the local project tree if cwd is a project root.
async fn ensure_dependencies(
    rockspec: &impl Rockspec,
    project_tree: &Tree,
    test_tree: &Tree,
    config: &Config,
    progress: Arc<Progress<MultiProgress>>,
) -> Result<(), InstallTestDependenciesError> {
    ensure_busted(test_tree, config, progress.clone()).await?;
    let test_dependencies = rockspec
        .test_dependencies()
        .current_platform()
        .iter()
        .filter(|req| !req.name().eq(&PackageName::new("lua".into())))
        .filter_map(|dep| {
            let build_behaviour = if test_tree
                .match_rocks(dep.package_req())
                .is_ok_and(|matches| matches.is_found())
            {
                Some(BuildBehaviour::Force)
            } else {
                None
            };
            build_behaviour.map(|build_behaviour| {
                PackageInstallSpec::new(dep.package_req().clone(), tree::EntryType::Entrypoint)
                    .build_behaviour(build_behaviour)
                    .pin(*dep.pin())
                    .opt(*dep.opt())
                    .build()
            })
        });

    Install::new(test_tree, config)
        .packages(test_dependencies)
        .progress(progress.clone())
        .install()
        .await?;

    let dependencies = rockspec
        .dependencies()
        .current_platform()
        .iter()
        .filter(|req| !req.name().eq(&PackageName::new("lua".into())))
        .filter_map(|dep| {
            let build_behaviour = if project_tree
                .match_rocks(dep.package_req())
                .is_ok_and(|matches| matches.is_found())
            {
                Some(BuildBehaviour::Force)
            } else {
                None
            };
            build_behaviour.map(|build_behaviour| {
                PackageInstallSpec::new(dep.package_req().clone(), tree::EntryType::Entrypoint)
                    .build_behaviour(build_behaviour)
                    .pin(*dep.pin())
                    .opt(*dep.opt())
                    .build()
            })
        });

    Install::new(project_tree, config)
        .packages(dependencies)
        .progress(progress)
        .install()
        .await?;

    Ok(())
}
