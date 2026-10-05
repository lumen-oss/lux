use std::path::PathBuf;

use assert_fs::{
    assert::PathAssert,
    prelude::{PathChild, PathCopy},
};
use flaky_test::flaky_test;
use lux_lib::{
    config::ConfigBuilder,
    lua_rockspec::RemoteLuaRockspec,
    operations::{Vendor, VendorTarget},
    workspace::Workspace,
};
use predicates::prelude::predicate;

#[flaky_test(tokio, times = 5)]
async fn vendor_dependencies() {
    let sample_project_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources/test/sample-projects/busted-with-lockfile/");
    let _ = tokio::fs::remove_dir_all(sample_project_dir.join(".lux")).await;
    let temp_dir = assert_fs::TempDir::new().unwrap();
    temp_dir.copy_from(sample_project_dir, &["**"]).unwrap();
    let workspace = Workspace::from_exact(temp_dir.path()).unwrap().unwrap();

    // Every rock in the sample project's committed lockfile must be vendored.
    // (These are the `test_dependencies` of `busted-with-lockfile`.)
    let expected_packages = [
        "luassert@1.9.0-1",
        "lua_cliargs@3.0.2-1",
        "penlight@1.15.0-1",
        "lua-term@0.8-1",
        "dkjson@2.11-1",
        "mediator_lua@1.1.2-0",
        "luafilesystem@1.9.0-1",
        "busted@2.3.0-1",
        "luasystem@0.7.1-1",
        "say@1.4.1-3",
    ];

    let config = ConfigBuilder::new().unwrap().build().unwrap();
    let vendor_dir = assert_fs::TempDir::new().unwrap();

    Vendor::new()
        .target(VendorTarget::Workspace(workspace))
        .vendor_dir(vendor_dir.to_path_buf())
        .config(&config)
        .vendor_dependencies()
        .await
        .unwrap();

    for package in expected_packages {
        let (name, version) = package.split_once('@').unwrap();

        // The rockspec must be vendored and parse back into a valid rockspec.
        let rockspec_file = vendor_dir
            .to_path_buf()
            .join(format!("{name}-{version}.rockspec"));
        assert!(rockspec_file.is_file(), "missing rockspec for {package}");
        let rockspec_content = std::fs::read_to_string(&rockspec_file).unwrap();
        RemoteLuaRockspec::new(&rockspec_content)
            .unwrap_or_else(|err| panic!("vendored rockspec for {package} failed to parse: {err}"));

        let source_path = vendor_dir.to_path_buf().join(package);
        assert!(
            source_path.is_dir() || source_path.is_file(),
            "missing vendored source for {package}"
        );
    }

    // The sample lockfile's dependencies are all `rockspec` sources, so the
    // vendored sources must be non-empty directories containing their contents.
    let busted_dir = vendor_dir.child("busted@2.3.0-1");
    busted_dir.assert(predicate::path::is_dir());
    assert!(
        std::fs::read_dir(busted_dir.path())
            .unwrap()
            .next()
            .is_some(),
        "vendored source tree for busted is empty"
    );
}
