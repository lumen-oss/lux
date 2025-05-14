use std::path::PathBuf;

use assert_fs::assert::PathAssert;
use assert_fs::prelude::{PathChild, PathCopy};
use assert_fs::TempDir;
use lux_lib::lua_installation::detect_installed_lua_version;
use lux_lib::operations::LuaBinary;
use lux_lib::progress::{MultiProgress, Progress, ProgressBar};
use lux_lib::{
    config::{ConfigBuilder, LuaVersion},
    lua_installation::LuaInstallation,
    luarocks::luarocks_installation::LuaRocksInstallation,
};
use predicates::prelude::predicate;

#[tokio::test]
async fn luarocks_make() {
    let dir = TempDir::new().unwrap();

    let lua_version = detect_installed_lua_version(LuaBinary::default())
        .ok()
        .and_then(|version| LuaVersion::from_version(version).ok())
        .or(Some(LuaVersion::Lua51));

    let config = ConfigBuilder::new()
        .unwrap()
        .user_tree(Some(dir.path().into()))
        .lua_version(lua_version.clone())
        .build()
        .unwrap();
    let tree = config.user_tree(lua_version.unwrap()).unwrap();
    let luarocks = LuaRocksInstallation::new(&config, tree).unwrap();
    let progress = Progress::Progress(MultiProgress::new());
    let bar = progress.map(|p| p.add(ProgressBar::from("Installing luarocks".to_string())));
    luarocks.ensure_installed(&bar).await.unwrap();
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources/test/sample-project-no-build-spec");
    let rockspec_path = project_root.join("foo-1.0.0-1.rockspec");
    let build_dir = TempDir::new().unwrap();
    build_dir.copy_from(&project_root, &["**"]).unwrap();
    let dest_dir = TempDir::new().unwrap();
    let lua_version = LuaVersion::from(&config).unwrap_or(&LuaVersion::Lua51);
    let lua = LuaInstallation::new(lua_version, &config);
    luarocks
        .make(&rockspec_path, build_dir.path(), dest_dir.path(), &lua)
        .unwrap();
    let foo_dir = dest_dir
        .child("share")
        .child("lua")
        .child(lua_version.version_compatibility_str())
        .child("foo");
    foo_dir.assert(predicate::path::is_dir());
    let foo_init = foo_dir.child("init.lua");
    foo_init.assert(predicate::path::is_file());
    foo_init.assert(predicate::str::contains("return true"));
}
