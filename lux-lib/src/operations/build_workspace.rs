use crate::{
    build::BuildBehaviour,
    config::Config,
    drivers::sync::{Sync, SyncError, SyncMode, TargetSet},
    lockfile::LockedPackage,
    package::PackageName,
    workspace::Workspace,
};
use bon::Builder;
use thiserror::Error;
use tracing::{info_span, Instrument};

#[derive(Debug, Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum BuildWorkspaceError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Sync(#[from] SyncError),
}

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct BuildWorkspace<'a> {
    #[builder(start_fn)]
    workspace: &'a Workspace,

    #[builder(start_fn)]
    config: &'a Config,

    /// Package to build
    package: Option<PackageName>,

    /// Ignore the project's lockfile and don't create one
    no_lock: bool,

    /// Build only the dependencies
    only_deps: bool,

    #[builder(default = BuildBehaviour::Force)]
    behaviour: BuildBehaviour,
}

impl<State: build_workspace_builder::State + build_workspace_builder::IsComplete>
    BuildWorkspaceBuilder<'_, State>
{
    pub async fn build(self) -> Result<Vec<LockedPackage>, BuildWorkspaceError> {
        let build = self._build();
        let span = match &build.package {
            Some(package) => info_span!("Building workspace", package = package.to_string()),
            None => info_span!("Building workspace"),
        };
        async move {
            let report = Sync::new(build.workspace, build.config)
                .mode(SyncMode::Open)
                .targets(TargetSet {
                    test: false,
                    members: build.package.map(|package| vec![package]),
                })
                .behaviour(build.behaviour)
                .only_deps(build.only_deps)
                .no_lock(build.no_lock)
                .sync()
                .await?;
            Ok(report.added().to_vec())
        }
        .instrument(span)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{
        config::ConfigBuilder, fs, lua_installation::detect_installed_lua_version,
        lua_version::LuaVersion, tree::InstallTree,
    };
    use assert_fs::prelude::PathCopy;
    use std::path::PathBuf;

    #[tokio::test]
    /// Non-regression for #980
    async fn builtin_build_autodetect_bin_scripts() {
        let project_root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-projects/init/");
        let data_dir: PathBuf = assert_fs::TempDir::new().unwrap().path().into();
        let temp_dir = assert_fs::TempDir::new().unwrap();
        temp_dir.copy_from(&project_root, &["**"]).unwrap();
        let project_root = temp_dir.path();
        let foo_bin_dir = project_root.join("src").join("bin");
        fs::tokio::create_dir_all(&foo_bin_dir).await.unwrap();
        let foo_bin_file = foo_bin_dir.join("foo");
        fs::tokio::write(&foo_bin_file, "print('hello')")
            .await
            .unwrap();
        let bar_bin_dir = project_root.join("bin");
        fs::tokio::create_dir_all(&bar_bin_dir).await.unwrap();
        let bar_bin_file = bar_bin_dir.join("bar");
        fs::tokio::write(&bar_bin_file, "print('hello')")
            .await
            .unwrap();
        let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));
        let config = ConfigBuilder::new()
            .unwrap()
            .data_dir(Some(data_dir))
            .lua_version(lua_version)
            .build()
            .unwrap();
        let workspace = Workspace::from_exact(project_root).unwrap().unwrap();
        let tree = workspace.tree(&config).unwrap();
        BuildWorkspace::new(&workspace, &config)
            .no_lock(false)
            .only_deps(false)
            .build()
            .await
            .unwrap();
        let bin_dir = tree.bin();
        assert!(bin_dir.join("foo").is_file());
        assert!(bin_dir.join("bar").is_file());
    }

    #[tokio::test]
    /// Non-regression for #1563
    async fn builtin_build_support_src_init_lua() {
        let data_dir: PathBuf = assert_fs::TempDir::new().unwrap().path().into();
        let project_root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-projects/init/");
        let temp_dir = assert_fs::TempDir::new().unwrap();
        temp_dir.copy_from(&project_root, &["**"]).unwrap();
        let project_root = temp_dir.path();
        let src_dir = project_root.join("src");
        fs::tokio::create_dir_all(&src_dir).await.unwrap();
        let init_lua_file = src_dir.join("init.lua");
        fs::tokio::write(&init_lua_file, "print('hello')")
            .await
            .unwrap();
        let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));
        let config = ConfigBuilder::new()
            .unwrap()
            .data_dir(Some(data_dir))
            .lua_version(lua_version)
            .build()
            .unwrap();
        let workspace = Workspace::from_exact(project_root).unwrap().unwrap();
        let package = BuildWorkspace::new(&workspace, &config)
            .no_lock(false)
            .only_deps(false)
            .build()
            .await
            .unwrap();
        let package = package.first().unwrap();
        let tree = workspace.tree(&config).unwrap();
        let src_dir = tree.layout_for(&package.spec).src;
        assert!(src_dir.join("init.lua").is_file());
    }
}
