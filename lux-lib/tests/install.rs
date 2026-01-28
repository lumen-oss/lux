use assert_fs::TempDir;
use itertools::Itertools;
use lux_lib::{
    config::{ConfigBuilder, LuaVersion},
    git::GitSource,
    lua_installation::detect_installed_lua_version,
    lua_rockspec::RockSourceSpec,
    operations::{Exec, Install, PackageInstallSpec},
    tree::EntryType,
};
use std::path::PathBuf;
use walkdir::WalkDir;

#[cfg(not(target_env = "msvc"))]
use serial_test::serial;

#[tokio::test]
async fn install_git_package() {
    let install_spec =
        PackageInstallSpec::new("rustaceanvim@6.0.3".parse().unwrap(), EntryType::Entrypoint)
            .source(RockSourceSpec::Git(GitSource {
                url: "https://github.com/mrcjkb/rustaceanvim.git"
                    .parse()
                    .unwrap(),
                checkout_ref: Some("v6.0.3".into()),
            }))
            .build();
    test_install(install_spec).await
}

// http 0.4 has an http-0.4-0.all.rock packed rock on luarocks.org
#[tokio::test]
#[serial]
#[cfg(not(target_env = "msvc"))] // http has dependencies that are not supported on Windows
async fn install_http_package() {
    let cflags = std::env::var("CFLAGS").unwrap_or_default();
    // See https://github.com/wahern/luaossl/issues/220#issuecomment-3401472124
    std::env::set_var("CFLAGS", "-Wno-error=incompatible-pointer-types");
    let install_spec =
        PackageInstallSpec::new("http@0.4-0".parse().unwrap(), EntryType::Entrypoint).build();
    test_install(install_spec).await;
    std::env::set_var("CFLAGS", cflags);
}

#[tokio::test]
async fn install_and_use_luafilesystem() {
    let install_spec = PackageInstallSpec::new(
        "luafilesystem@1.9.0".parse().unwrap(),
        EntryType::Entrypoint,
    )
    .build();
    let dir = TempDir::new().unwrap();
    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

    let config = ConfigBuilder::new()
        .unwrap()
        .user_tree(Some(dir.to_path_buf()))
        .lua_version(lua_version)
        .build()
        .unwrap();

    let tree = config
        .user_tree(LuaVersion::from(&config).unwrap().clone())
        .unwrap();
    let installed = Install::new(&config)
        .package(install_spec)
        .tree(tree)
        .install()
        .await
        .unwrap();
    assert!(!installed.is_empty());

    Exec::new("lua", None, &config)
        .arg("-e")
        .arg("require('lfs')")
        .exec()
        .await
        .unwrap()
}

// See https://github.com/lumen-oss/lux/issues/1106
#[tokio::test]
async fn no_build_artifacts_in_cwd() {
    let cwd = std::env::current_dir().unwrap();
    let cwd_content_before_install = WalkDir::new(&cwd)
        .into_iter()
        .filter_map(Result::ok)
        .map(|entry| entry.into_path())
        .collect_vec();
    let install_spec =
        PackageInstallSpec::new("bit32@5.3.5".parse().unwrap(), EntryType::Entrypoint).build();
    test_install(install_spec).await;
    let build_artifacts = WalkDir::new(&cwd)
        .into_iter()
        .filter_map(Result::ok)
        .map(|entry| entry.into_path())
        .filter(|file| !cwd_content_before_install.contains(file))
        .collect_vec();
    assert_eq!(build_artifacts, vec![] as Vec<PathBuf>)
}

#[cfg(test)]
async fn test_install(install_spec: PackageInstallSpec) {
    let dir = TempDir::new().unwrap();
    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

    let config = ConfigBuilder::new()
        .unwrap()
        .user_tree(Some(dir.to_path_buf()))
        .lua_version(lua_version)
        .build()
        .unwrap();

    let tree = config
        .user_tree(LuaVersion::from(&config).unwrap().clone())
        .unwrap();
    let installed = Install::new(&config)
        .package(install_spec)
        .tree(tree)
        .install()
        .await
        .unwrap();
    assert!(!installed.is_empty());
}
