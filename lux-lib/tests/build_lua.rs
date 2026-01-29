use assert_fs::{assert::PathAssert, prelude::PathChild};
use lux_lib::{
    config::{ConfigBuilder, LuaVersion},
    operations::BuildLua,
    progress::MultiProgress,
};
use predicates::prelude::predicate;

#[tokio::test]
async fn test_build_lua() {
    for lua_version in [
        LuaVersion::Lua51,
        LuaVersion::Lua52,
        LuaVersion::Lua53,
        LuaVersion::Lua54,
        LuaVersion::Lua55,
    ] {
        let target_dir = assert_fs::TempDir::new().unwrap();
        let target_path = target_dir.to_path_buf();
        let user_tree = assert_fs::TempDir::new().unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .user_tree(Some(user_tree.to_path_buf()))
            .lua_version(Some(lua_version.clone()))
            .build()
            .unwrap();
        let progress = MultiProgress::new(&config);
        let bar = progress.map(MultiProgress::new_bar);
        BuildLua::new()
            .lua_version(&lua_version)
            .progress(&bar)
            .install_dir(&target_path)
            .config(&config)
            .build()
            .await
            .unwrap();
        let lua_bin_dir = target_dir.child("bin");
        lua_bin_dir.assert(predicate::path::is_dir());
        let lua_bin = if cfg!(target_env = "msvc") {
            lua_bin_dir.child("lua.exe")
        } else {
            lua_bin_dir.child("lua")
        };
        lua_bin.assert(predicate::path::is_file());
        let lua_include_dir = target_dir.child("include");
        lua_include_dir.assert(predicate::path::is_dir());
        let lua_header = lua_include_dir.child("lua.h");
        lua_header.assert(predicate::path::is_file());
        let lua_lib_dir = target_dir.child("lib");
        lua_lib_dir.assert(predicate::path::is_dir());
        let lua_lib = if cfg!(target_env = "msvc") {
            let lib_name = match lua_version {
                LuaVersion::Lua51 => "lua5.1.lib",
                LuaVersion::Lua52 => "lua5.2.lib",
                LuaVersion::Lua53 => "lua5.3.lib",
                LuaVersion::Lua54 => "lua5.4.lib",
                LuaVersion::Lua55 => "lua5.5.lib",
                LuaVersion::LuaJIT | LuaVersion::LuaJIT52 => unreachable!(),
            };
            lua_lib_dir.child(lib_name)
        } else {
            lua_lib_dir.child("liblua.a")
        };
        lua_lib.assert(predicate::path::is_file());
    }
}

#[tokio::test]
async fn test_build_luajit() {
    for lua_version in [LuaVersion::LuaJIT, LuaVersion::LuaJIT52] {
        let target_dir = assert_fs::TempDir::new().unwrap();
        let target_path = target_dir.to_path_buf();
        let user_tree = assert_fs::TempDir::new().unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .user_tree(Some(user_tree.to_path_buf()))
            .lua_version(Some(lua_version.clone()))
            .build()
            .unwrap();
        let progress = MultiProgress::new(&config);
        let bar = progress.map(MultiProgress::new_bar);
        BuildLua::new()
            .lua_version(&lua_version)
            .progress(&bar)
            .install_dir(&target_path)
            .config(&config)
            .build()
            .await
            .unwrap();
        let luajit_bin_dir = target_dir.child("bin");
        luajit_bin_dir.assert(predicate::path::is_dir());
        let luajit_bin = if cfg!(target_env = "msvc") {
            luajit_bin_dir.child("luajit.exe")
        } else {
            luajit_bin_dir.child("luajit")
        };
        luajit_bin.assert(predicate::path::exists()); // May be a symlink
        let luajit_include_dir = target_dir.child("include");
        luajit_include_dir.assert(predicate::path::is_dir());
        let lua_header = luajit_include_dir.child("lua.h");
        lua_header.assert(predicate::path::is_file());
        let luajit_lib_dir = target_dir.child("lib");
        luajit_lib_dir.assert(predicate::path::is_dir());
        let luajit_lib = if cfg!(target_env = "msvc") {
            luajit_lib_dir.child("luajit.lib")
        } else {
            // NOTE: luajit builds a libluajit-5.1 lib even if 5.2 compatibility is enabled.
            luajit_lib_dir.child("libluajit-5.1.a")
        };
        luajit_lib.assert(predicate::path::is_file());
    }
}
