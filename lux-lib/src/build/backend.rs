use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
};

use bon::Builder;

use crate::{
    build::external_dependency::ExternalDependencyInfo,
    config::Config,
    lua_installation::LuaInstallation,
    lua_rockspec::DeploySpec,
    tree::{InstallTree, RockLayout},
};

#[derive(Builder)]
#[builder(start_fn(name = "new"))]
pub(crate) struct RunBuildArgs<'a, T: InstallTree> {
    pub(crate) output_paths: &'a RockLayout,
    pub(crate) no_install: bool,
    pub(crate) lua: &'a LuaInstallation,
    pub(crate) external_dependencies: &'a HashMap<String, ExternalDependencyInfo>,
    pub(crate) deploy: &'a DeploySpec,
    pub(crate) config: &'a Config,
    pub(crate) tree: &'a T,
    pub(crate) build_dir: &'a Path,
}

pub(crate) trait BuildBackend {
    type Err: std::error::Error;

    fn run<T: InstallTree + Sync>(
        self,
        args: RunBuildArgs<'_, T>,
    ) -> impl Future<Output = Result<BuildInfo, Self::Err>> + Send;
}

#[derive(Default)]
pub(crate) struct BuildInfo {
    pub binaries: Vec<PathBuf>,
}
