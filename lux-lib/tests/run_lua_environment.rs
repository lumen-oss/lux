#![cfg(feature = "impure_tests")]

use assert_fs::prelude::*;
use lux_lib::{
    config::ConfigBuilder,
    lua_installation::LuaBinary,
    lua_version::LuaVersion,
    operations::{RunLua, RunLuaError},
    path::LuaEnvironment,
};
use serial_test::serial;
use which::which;

#[serial]
#[tokio::test]
async fn pure_environment_hides_inherited_lua_paths() {
    let Some(lua_cmd) = which("lua").ok().or_else(|| which("luajit").ok()) else {
        eprintln!("skipping: no Lua interpreter on PATH");
        return;
    };

    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("main.lua")
        .write_str(r#"os.exit(pcall(require, "external") and 0 or 1)"#)
        .unwrap();

    let external = temp.child("external");
    external.create_dir_all().unwrap();
    external
        .child("external.lua")
        .write_str("return true")
        .unwrap();

    let version = LuaVersion::Lua51;
    let external_path = external.child("?.lua");
    std::env::set_var("LUA_PATH_5_1", external_path.to_string_lossy().to_string());

    let config = ConfigBuilder::new()
        .unwrap()
        .user_tree(Some(temp.path().to_path_buf()))
        .data_dir(Some(temp.child("data").to_path_buf()))
        .build()
        .unwrap();
    let tree = config.user_tree(version.clone()).unwrap();
    let lua_cmd = LuaBinary::Custom(lua_cmd.to_string_lossy().to_string());
    let args = vec!["main.lua".to_string()];

    let pure = RunLua::new()
        .root(temp.path())
        .tree(&tree)
        .config(&config)
        .lua_cmd(lua_cmd.clone())
        .args(&args)
        .environment(LuaEnvironment::Pure)
        .run_lua()
        .await;
    assert!(
        matches!(pure, Err(RunLuaError::LuaCommandNonZeroExitCode { .. })),
        "pure environment should not see the inherited LUA_PATH: {pure:?}"
    );

    RunLua::new()
        .root(temp.path())
        .tree(&tree)
        .config(&config)
        .lua_cmd(lua_cmd)
        .args(&args)
        .environment(LuaEnvironment::Impure)
        .run_lua()
        .await
        .unwrap();

    std::env::remove_var("LUA_PATH_5_1");
}
