#![recursion_limit = "256"]

#[cfg(not(target_env = "msvc"))]
use flaky_test::flaky_test;
#[cfg(not(target_env = "msvc"))]
use lux_lib::{config, lua, operations::Exec};

#[cfg(not(target_env = "msvc"))]
#[flaky_test(tokio, times = 5)]
async fn run_nlua() {
    let dir = assert_fs::TempDir::new().unwrap();

    let lua_version = lua::detect::installed_version().or(Some(lua::Version::Lua51));

    let config = config::Builder::new()
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
