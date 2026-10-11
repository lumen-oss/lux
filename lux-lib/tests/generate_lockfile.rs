// Skipped on Windows: keygen tests fail because no known cipher is available.
#![cfg(not(target_os = "windows"))]
// Helper functions here aren't detected as test code by clippy's
// `allow-unwrap-in-tests`, so unwraps must be allowed file-wide.
#![allow(clippy::unwrap_used)]

use assert_fs::TempDir;
use flaky_test::flaky_test;
use lux_lib::{
    config::{Config, ConfigBuilder},
    drivers::sync::{Sync, SyncMode, TargetSet},
    lua_installation::detect_installed_lua_version,
    lua_version::LuaVersion,
    operations::GenerateLockfile,
    workspace::Workspace,
};

fn write_project(temp: &TempDir) {
    std::fs::write(
        temp.path().join("lux.toml"),
        r#"package = "generate-lockfile-test"
version = "0.1.0"
lua = ">=5.1"

[source]
url = "https://github.com/lumen-oss/luarocks-stub"

[dependencies]
luassert = "1.9.0"

[build]
type = "builtin"
"#,
    )
    .unwrap();
    std::fs::create_dir_all(temp.path().join("lua")).unwrap();
    std::fs::write(temp.path().join("lua/init.lua"), "return {}\n").unwrap();
}

fn test_config() -> Config {
    ConfigBuilder::new()
        .unwrap()
        .lua_version(detect_installed_lua_version().or(Some(LuaVersion::Lua51)))
        .build()
        .unwrap()
}

async fn read_lockfile(workspace: &Workspace) -> String {
    tokio::fs::read_to_string(workspace.lockfile_path())
        .await
        .unwrap()
}

/// A generated lockfile must be identical to the one produced by actually
/// installing the project's dependencies.
#[flaky_test(tokio, times = 5)]
async fn generated_lockfile_matches_install() {
    let generated = {
        let temp = TempDir::new().unwrap();
        write_project(&temp);
        let workspace = Workspace::from_exact(temp.path()).unwrap().unwrap();
        let config = test_config();
        GenerateLockfile::new(&config, &workspace)
            .generate()
            .await
            .unwrap();
        read_lockfile(&workspace).await
    };

    let installed = {
        let temp = TempDir::new().unwrap();
        write_project(&temp);
        let workspace = Workspace::from_exact(temp.path()).unwrap().unwrap();
        let config = test_config();
        Sync::new(&workspace, &config)
            .mode(SyncMode::Open)
            .targets(TargetSet::all(&workspace, true))
            .only_deps(true)
            .sync()
            .await
            .unwrap();
        read_lockfile(&workspace).await
    };

    assert_eq!(generated, installed);
}

/// The generated lockfile must list the project's direct dependencies as
/// entrypoints.
#[flaky_test(tokio, times = 5)]
async fn generated_lockfile_contains_entrypoints() {
    let temp = TempDir::new().unwrap();
    write_project(&temp);
    let workspace = Workspace::from_exact(temp.path()).unwrap().unwrap();
    let config = test_config();

    GenerateLockfile::new(&config, &workspace)
        .generate()
        .await
        .unwrap();

    let lockfile = read_lockfile(&workspace).await;
    assert!(lockfile.contains("luassert"), "{lockfile}");
}
