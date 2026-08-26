#![recursion_limit = "256"]

use assert_fs::prelude::PathCopy;
use flaky_test::flaky_test;
use lux_lib::{
    config::ConfigBuilder, lua_installation::LuaInstallation, lua_version::LuaVersion,
    operations::Test, workspace::Workspace,
};
use std::path::PathBuf;

#[flaky_test(tokio, times = 5)]
async fn test_luau_tiniest() {
    let project_root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-projects/hello-luau");
    let temp_dir = assert_fs::TempDir::new().unwrap();
    temp_dir.copy_from(project_root, &["**/*"]).unwrap();
    let tree_root = temp_dir.path().join(".lux");

    let config = ConfigBuilder::new()
        .unwrap()
        .user_tree(Some(tree_root))
        .lua_version(Some(LuaVersion::Luau))
        .build()
        .unwrap();

    if which::which("luau").is_err() && which::which("lune").is_err() {
        LuaInstallation::install(&LuaVersion::Luau, &config)
            .await
            .unwrap();
    }

    let workspace = Workspace::from_exact(temp_dir.path()).unwrap().unwrap();

    Test::new(workspace, &config).run().await.unwrap();
}
