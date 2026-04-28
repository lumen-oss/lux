use assert_fs::{prelude::PathCopy, TempDir};
use lux_lib::{config::ConfigBuilder, operations::Sync, tree::RockMatches, workspace::Workspace};
use std::path::PathBuf;

#[tokio::test]
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
