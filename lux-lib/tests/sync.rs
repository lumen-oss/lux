use assert_fs::{prelude::PathCopy, TempDir};
use flaky_test::flaky_test;
use lux_lib::{
    config::ConfigBuilder,
    operations::Sync,
    tree::{InstallTree, RockMatches},
    workspace::Workspace,
};
use std::path::PathBuf;

#[flaky_test(tokio, times = 5)]
async fn sync_test_dependencies_empty_project() {
    let sample_project_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources/test/sample-projects/busted-with-lockfile/");
    let _ = tokio::fs::remove_dir_all(sample_project_dir.join(".lux")).await;
    let temp_dir = TempDir::new().unwrap();
    temp_dir.copy_from(sample_project_dir, &["**"]).unwrap();
    let workspace = Workspace::from_exact(temp_dir.path()).unwrap().unwrap();
    let config = ConfigBuilder::new().unwrap().build().unwrap();

    let lockfile_before_sync =
        String::from_utf8(tokio::fs::read(workspace.lockfile_path()).await.unwrap());

    Sync::new(&workspace, &config)
        .validate_integrity(cfg!(not(target_os = "windows")))
        .fast(true)
        .sync_test_dependencies()
        .await
        .unwrap();

    let lockfile_after_sync =
        String::from_utf8(tokio::fs::read(workspace.lockfile_path()).await.unwrap());

    if cfg!(not(target_os = "windows")) {
        // Source hashes are different on Windows
        assert_eq!(lockfile_before_sync, lockfile_after_sync);
    }

    let test_tree = workspace.tree(&config).unwrap().test_tree(&config).unwrap();

    assert!(matches!(
        test_tree
            .match_rocks(&"busted@2.2.0-1".parse().unwrap())
            .unwrap(),
        RockMatches::Single { .. }
    ));
    assert!(matches!(
        test_tree
            .match_rocks(&"penlight@1.14.0-3".parse().unwrap())
            .unwrap(),
        RockMatches::Single { .. }
    ));
    assert!(matches!(
        test_tree
            .match_rocks(&"luafilesystem@1.8.0-1".parse().unwrap())
            .unwrap(),
        RockMatches::Single { .. }
    ));
}

/// non-regression for https://github.com/lumen-oss/lux/issues/1548
#[flaky_test(tokio, times = 5)]
async fn sync_multi_projects_same_dependencies() {
    let sample_project_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources/test/sample-projects/multi-project/");
    let _ = tokio::fs::remove_dir_all(sample_project_dir.join(".lux")).await;
    let temp_dir = TempDir::new().unwrap();
    temp_dir.copy_from(sample_project_dir, &["**"]).unwrap();
    let workspace = Workspace::from_exact(temp_dir.path()).unwrap().unwrap();
    let config = ConfigBuilder::new().unwrap().build().unwrap();

    Sync::new(&workspace, &config)
        .validate_integrity(cfg!(not(target_os = "windows")))
        .fast(true)
        .sync_test_dependencies()
        .await
        .unwrap();
}

#[cfg(not(target_os = "windows"))]
#[flaky_test(tokio, times = 5)]
async fn sync_clean_install_tree() {
    use lux_lib::{lua_installation::detect_installed_lua_version, lua_version::LuaVersion};

    let sample_project_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources/test/sample-projects/dependencies/");
    let _ = tokio::fs::remove_dir_all(sample_project_dir.join(".lux")).await;
    let project_dir = TempDir::new().unwrap();
    project_dir.copy_from(sample_project_dir, &["**"]).unwrap();
    tokio::fs::remove_file(project_dir.join("lux.toml"))
        .await
        .unwrap();
    tokio::fs::write(
        project_dir.join("lux.toml"),
        r#"
package = "sample-project"
version = "0.1.0"
lua = ">=5.1"

[source]
url = "https://github.com/lumen-oss/luarocks-stub"

[dependencies]
fallo = "2.3.0"
"#,
    )
    .await
    .unwrap();
    let workspace_tree = TempDir::new().unwrap();
    let workspace = Workspace::from_exact(project_dir.path()).unwrap().unwrap();
    let lua_version = detect_installed_lua_version().unwrap_or(LuaVersion::Lua51);
    let config = ConfigBuilder::new()
        .unwrap()
        .lua_version(Some(lua_version.clone()))
        .workspace_tree(Some(workspace_tree.to_path_buf()))
        .build()
        .unwrap();

    let workspace_lockfile = project_dir.join("lux.lock");

    Sync::new(&workspace, &config)
        .validate_integrity(cfg!(not(target_os = "windows")))
        .sync_dependencies()
        .await
        .unwrap();

    let workspace_lockfile_content_1 = tokio::fs::read_to_string(&workspace_lockfile)
        .await
        .unwrap();

    tokio::fs::remove_dir_all(workspace_tree).await.unwrap();

    Sync::new(&workspace, &config)
        .validate_integrity(cfg!(not(target_os = "windows")))
        .sync_dependencies()
        .await
        .unwrap();

    let workspace_lockfile_content_2 = tokio::fs::read_to_string(&workspace_lockfile)
        .await
        .unwrap();

    assert_eq!(workspace_lockfile_content_1, workspace_lockfile_content_2);
}
