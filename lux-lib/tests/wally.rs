use std::path::PathBuf;

use assert_fs::{prelude::PathCopy, TempDir};
use flaky_test::flaky_test;
use lux_lib::{
    config::ConfigBuilder,
    lua_version::LuaVersion,
    operations::{BuildWorkspace, Run},
    workspace::Workspace,
};

#[flaky_test(tokio, times = 5)]
async fn test_install_wally_dependency() {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources/test/sample-projects/wally-signal");
    let project = TempDir::new().unwrap();
    project.copy_from(&project_root, &["**/*"]).unwrap();

    let config = ConfigBuilder::new()
        .unwrap()
        .cache_dir(Some(project.path().join(".cache")))
        .lua_version(Some(LuaVersion::Luau))
        .build()
        .unwrap();

    let workspace = Workspace::from_exact(project.path()).unwrap().unwrap();
    BuildWorkspace::new(&workspace, &config)
        .no_lock(false)
        .only_deps(false)
        .build()
        .await
        .unwrap();

    Run::new()
        .workspace(&workspace)
        .config(&config)
        .args(&Vec::new())
        .run()
        .await
        .unwrap();
}
