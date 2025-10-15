use std::path::PathBuf;

use assert_fs::prelude::PathCopy;
use lux_lib::{
    config::{ConfigBuilder, LuaVersion},
    lua_installation::detect_installed_lua_version,
    operations::Test,
    project::Project,
};
use tokio::fs::remove_dir_all;

#[tokio::test]
async fn run_busted_test() {
    let project_root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-projects/busted/");
    let temp_dir = assert_fs::TempDir::new().unwrap();
    temp_dir.copy_from(&project_root, &["**"]).unwrap();
    let project_root = temp_dir.path();
    let project: Project = Project::from(project_root).unwrap().unwrap();
    let tree_root = project.root().to_path_buf().join(".lux");
    let _ = remove_dir_all(&tree_root).await;

    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

    let config = ConfigBuilder::new()
        .unwrap()
        .user_tree(Some(tree_root))
        .lua_version(lua_version)
        .build()
        .unwrap();

    Test::new(project, &config).run().await.unwrap();
}

#[tokio::test]
async fn run_busted_test_no_lock() {
    let project_root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-projects/busted/");
    let temp_dir = assert_fs::TempDir::new().unwrap();
    temp_dir.copy_from(&project_root, &["**"]).unwrap();
    let project_root = temp_dir.path();
    let project: Project = Project::from(project_root).unwrap().unwrap();
    let tree_root = project.root().to_path_buf().join(".lux");
    let _ = remove_dir_all(&tree_root).await;

    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

    let config = ConfigBuilder::new()
        .unwrap()
        .user_tree(Some(tree_root))
        .lua_version(lua_version)
        .build()
        .unwrap();

    Test::new(project, &config)
        .no_lock(true)
        .run()
        .await
        .unwrap();
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn non_regression_lockfile_corruption() {
    let sample_project_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources/test/sample-projects/busted-with-lockfile/");
    let _ = remove_dir_all(sample_project_dir.join(".lux")).await;
    let temp_dir = assert_fs::TempDir::new().unwrap();
    temp_dir.copy_from(sample_project_dir, &["**"]).unwrap();
    let project = Project::from_exact(temp_dir.path()).unwrap().unwrap();
    let config = ConfigBuilder::new().unwrap().build().unwrap();

    let lockfile_before_test =
        String::from_utf8(tokio::fs::read(project.lockfile_path()).await.unwrap());

    Test::new(project, &config)
        .no_lock(true)
        .run()
        .await
        .unwrap();

    let project = Project::from_exact(temp_dir.path()).unwrap().unwrap();
    let lockfile_after_test =
        String::from_utf8(tokio::fs::read(project.lockfile_path()).await.unwrap());
    assert_eq!(lockfile_before_test, lockfile_after_test);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn run_busted_nlua_test() {
    run_busted_nlua_test_impl(false).await
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn run_busted_nlua_test_no_lock() {
    run_busted_nlua_test_impl(true).await
}

// NOTE: The busted-nlua test backend is currently broken on macOS and Windows.
// On macOS, it appears that Neovim segfaults when `require`ing `lfs` (luafilesystem).
// Investigation is needed on Windows.
#[cfg(target_os = "linux")]
async fn run_busted_nlua_test_impl(no_lock: bool) {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources/test/sample-projects/busted-nlua/");
    let temp_dir = assert_fs::TempDir::new().unwrap();
    temp_dir.copy_from(&project_root, &["**"]).unwrap();
    let project_root = temp_dir.path();
    let project: Project = Project::from(project_root).unwrap().unwrap();
    let tree_root = project.root().to_path_buf().join(".lux");
    let _ = remove_dir_all(&tree_root).await;

    let config = ConfigBuilder::new()
        .unwrap()
        .user_tree(Some(tree_root))
        .build()
        .unwrap();
    Test::new(project, &config)
        .no_lock(no_lock)
        .run()
        .await
        .unwrap();
}
