use assert_fs::prelude::PathCopy;
use flaky_test::flaky_test;
use lux_lib::{
    config, lua, operations, package,
    path::{LuaEnvironment, Paths},
    tree::InstallTree,
    workspace::Workspace,
};
use std::path::PathBuf;
use tokio::process::Command;

#[flaky_test(tokio, times = 3)]
async fn test_build_teal() {
    let manifest_dir: PathBuf = env!("CARGO_MANIFEST_DIR").into();
    let sample_projects = manifest_dir
        .join("resources")
        .join("test")
        .join("sample-projects");
    let project_root = sample_projects.join("hello-teal");
    let temp_dir = assert_fs::TempDir::new().unwrap();
    temp_dir.copy_from(&project_root, &["**"]).unwrap();
    let tree_dir = assert_fs::TempDir::new().unwrap();
    let lua_version = lua::detect::installed_version().or(Some(lua::Version::Lua51));
    if lua_version == Some(lua::Version::Lua55) {
        println!("teal 0.24.8 is not supported on Lua 5.5 yet! Skipping...");
        // FIXME: teal 0.24.8 predates the Lua 5.5 fix.
        // See https://github.com/teal-language/tl/pull/1157
        return;
    }
    let config = config::Builder::new()
        .unwrap()
        .user_tree(Some(tree_dir.to_path_buf()))
        .lua_version(lua_version)
        .build()
        .unwrap();
    let workspace = Workspace::from_exact(temp_dir.path()).unwrap().unwrap();

    let packages = operations::BuildWorkspace::new(&workspace, &config)
        .no_lock(false)
        .only_deps(false)
        .build()
        .await
        .unwrap();
    let name: package::Name = "hello-teal".into();
    let package = packages
        .iter()
        .find(|package| package.name() == &name)
        .unwrap();
    let tree = workspace.tree(&config).unwrap();
    let layout = tree.layout_for(package.spec());

    assert!(layout.src.join("greeting.tl").is_file());
    assert!(layout.src.join("hello.d.tl").is_file());
    assert!(layout.src.join("main.tl").is_file());

    let lua = lua::Installation::new_from_config(&config).await.unwrap();
    let lua_bin = lua.bin().as_ref().expect("expected a Lua executable");
    let paths = Paths::new(&tree).unwrap();
    let output = Command::new(lua_bin)
        .arg("-e")
        .arg("require('main')")
        .envs(paths.lua_env_vars(LuaEnvironment::Pure, ""))
        .env("PATH", paths.path_prepended().joined())
        .output()
        .await
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello teal");
}
