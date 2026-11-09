use std::path::PathBuf;

use assert_fs::prelude::PathCopy;
use lux_lib::{config, lua, operations, workspace::Workspace};

#[tokio::test]
async fn build_rockspec_format_3_1() {
    let sample_project =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-projects/format-3-1");
    let workspace_root = assert_fs::TempDir::new().unwrap();
    workspace_root.copy_from(&sample_project, &["**"]).unwrap();
    let user_tree = assert_fs::TempDir::new().unwrap();

    let lua_version = lua::detect::installed_version().or(Some(lua::Version::Lua51));
    let config = config::Builder::new()
        .unwrap()
        .user_tree(Some(user_tree.to_path_buf()))
        .lua_version(lua_version.clone())
        .build()
        .unwrap();
    std::fs::write(
        workspace_root
            .path()
            .join("uses_3_1")
            .join("expected-lua-version.txt"),
        lua_version.unwrap().version_compatibility_str(),
    )
    .unwrap();

    let workspace = Workspace::from_exact(workspace_root.path())
        .unwrap()
        .unwrap();
    operations::BuildWorkspace::new(&workspace, &config)
        .no_lock(false)
        .only_deps(false)
        .build()
        .await
        .unwrap();
}
