use assert_fs::TempDir;
use itertools::Itertools;
use lux_lib::{
    config::{ConfigBuilder, LuaVersion},
    git::GitSource,
    lua_installation::detect_installed_lua_version,
    lua_rockspec::RockSourceSpec,
    operations::{Install, PackageInstallSpec},
    tree::EntryType,
};
use std::path::PathBuf;
use walkdir::WalkDir;

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
#[cfg(not(target_env = "msvc"))] // http has dependencies that are not supported on Windows
async fn install_http_package() {
    let install_spec =
        PackageInstallSpec::new("http@0.4-0".parse().unwrap(), EntryType::Entrypoint).build();
    test_install(install_spec).await
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
        .user_tree(LuaVersion::from_config(&config).unwrap().clone())
        .unwrap();
    let installed = Install::new(&config)
        .package(install_spec)
        .tree(tree)
        .install()
        .await
        .unwrap();
    assert!(!installed.is_empty());
}
