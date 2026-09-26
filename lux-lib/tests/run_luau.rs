use assert_fs::prelude::PathCopy;
use flaky_test::flaky_test;
use lux_lib::{
    config::ConfigBuilder, lua_version::LuaVersion, operations::Run, workspace::Workspace,
};
use std::path::PathBuf;

#[flaky_test(tokio, times = 5)]
async fn test_luau_run_entrypoint() {
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

    let workspace = Workspace::from_exact(temp_dir.path()).unwrap().unwrap();

    Run::new()
        .workspace(&workspace)
        .config(&config)
        .args(&Vec::new())
        .run()
        .await
        .unwrap();
}
