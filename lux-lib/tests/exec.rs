#[cfg(not(target_env = "msvc"))]
use lux_lib::{config::ConfigBuilder, operations::Exec};

#[cfg(not(target_env = "msvc"))]
#[tokio::test]
async fn run_nlua() {
    use lux_lib::{config::LuaVersion, lua_installation::detect_installed_lua_version};

    let dir = assert_fs::TempDir::new().unwrap();

    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

    let config = ConfigBuilder::new()
        .unwrap()
        .user_tree(Some(dir.to_path_buf()))
        .lua_version(lua_version)
        .build()
        .unwrap();
    Exec::new("nlua", None, &config)
        .arg("-v")
        .exec()
        .await
        .unwrap();
}
