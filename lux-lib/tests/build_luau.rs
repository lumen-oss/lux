use assert_fs::{assert::PathAssert, prelude::PathChild};
use flaky_test::flaky_test;
use lux_lib::lua_version::LuaVersion;
use lux_lib::{config::ConfigBuilder, operations::BuildLua};
use predicates::prelude::predicate;

#[flaky_test(tokio, times = 5)]
async fn test_build_luau() {
    let target_dir = assert_fs::TempDir::new().unwrap();
    let target_path = target_dir.to_path_buf();
    let user_tree = assert_fs::TempDir::new().unwrap();
    let config = ConfigBuilder::new()
        .unwrap()
        .user_tree(Some(user_tree.to_path_buf()))
        .lua_version(Some(LuaVersion::Luau))
        .build()
        .unwrap();
    BuildLua::new()
        .lua_version(&LuaVersion::Luau)
        .install_dir(&target_path)
        .config(&config)
        .build()
        .await
        .unwrap();
    let bin_dir = target_dir.child("bin");
    bin_dir.assert(predicate::path::is_dir());
    let luau = if cfg!(target_env = "msvc") {
        bin_dir.child("luau.exe")
    } else {
        bin_dir.child("luau")
    };
    luau.assert(predicate::path::is_file());
    let luau_analyze = if cfg!(target_env = "msvc") {
        bin_dir.child("luau-analyze.exe")
    } else {
        bin_dir.child("luau-analyze")
    };
    luau_analyze.assert(predicate::path::is_file());
}
