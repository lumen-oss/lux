use std::path::PathBuf;

use assert_fs::prelude::PathCopy;
use assert_fs::TempDir;
use lux_lib::lua_version::LuaVersion;
use lux_lib::operations::BuildWorkspace;
use lux_lib::workspace::Workspace;
use lux_lib::{config::ConfigBuilder, lua_installation::detect_installed_lua_version};

#[tokio::test]
async fn test_build_multi_workspace_local_dependencies() {
    let sample_project = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources/test/sample-projects/multi-project-local-deps");
    let workspace_root = TempDir::new().unwrap();
    workspace_root.copy_from(&sample_project, &["**"]).unwrap();

    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

    let config = ConfigBuilder::new()
        .unwrap()
        .lua_version(lua_version.clone())
        .build()
        .unwrap();
    let workspace = Workspace::from_exact(&workspace_root).unwrap().unwrap();
    BuildWorkspace::new(&workspace, &config)
        .no_lock(false)
        .only_deps(false)
        .build()
        .await
        .unwrap();
}
