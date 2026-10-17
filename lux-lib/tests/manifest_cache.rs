use assert_fs::TempDir;
use flaky_test::flaky_test;
use lux_lib::{
    build::BuildBehaviour,
    config::ConfigBuilder,
    drivers::install_packages::InstallPackages,
    lua_installation::detect_installed_lua_version,
    lua_version::LuaVersion,
    operations::PackageInstallSpec,
    package::PackageReq,
    package_db::PackageDB,
    tree::{EntryType, InstallTree},
};
use std::str::FromStr;

#[cfg(test)]
async fn install_rock_lockfile(rock: &str, manifest_cache: bool) -> String {
    let dir = TempDir::new().unwrap();
    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

    let mut config_builder = ConfigBuilder::new()
        .unwrap()
        .user_tree(Some(dir.to_path_buf()))
        .lua_version(lua_version);
    if !manifest_cache {
        config_builder = config_builder.no_manifest_cache(Some(true));
    }
    let config = config_builder.build().unwrap();

    let tree = config
        .user_tree(LuaVersion::from(&config).unwrap().clone())
        .unwrap();

    let spec = PackageInstallSpec::new(PackageReq::from_str(rock).unwrap(), EntryType::Entrypoint)
        .build_behaviour(BuildBehaviour::Force)
        .build();

    InstallPackages::new(&config, &tree)
        .package(spec)
        .package_db(PackageDB::from_config(&config).await.unwrap())
        .install()
        .await
        .unwrap();

    String::from_utf8(tokio::fs::read(tree.lockfile_path()).await.unwrap()).unwrap()
}

#[flaky_test(tokio, times = 5)]
async fn lockfile_is_identical_with_and_without_manifest_cache() {
    let rock = "luassert";

    let with_cache = install_rock_lockfile(rock, true).await;
    let without_cache = install_rock_lockfile(rock, false).await;

    assert_eq!(with_cache, without_cache);
}
