use std::path::PathBuf;

use assert_fs::{
    assert::PathAssert,
    prelude::{PathChild, PathCopy},
};
use lux_lib::{
    config::ConfigBuilder,
    operations::{Vendor, VendorTarget},
    project::Project,
};
use predicates::prelude::predicate;

#[tokio::test]
async fn vendor_dependencies() {
    let sample_project_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources/test/sample-projects/busted-with-lockfile/");
    let _ = tokio::fs::remove_dir_all(sample_project_dir.join(".lux")).await;
    let temp_dir = assert_fs::TempDir::new().unwrap();
    temp_dir.copy_from(sample_project_dir, &["**"]).unwrap();
    let project = Project::from_exact(temp_dir.path()).unwrap().unwrap();
    let config = ConfigBuilder::new().unwrap().build().unwrap();
    let vendor_dir = assert_fs::TempDir::new().unwrap();

    Vendor::new()
        .target(VendorTarget::Project(project))
        .vendor_dir(vendor_dir.to_path_buf())
        .config(&config)
        .vendor_dependencies()
        .await
        .unwrap();

    let busted_rockspec = vendor_dir.child("busted-2.2.0-1.rockspec");
    busted_rockspec.assert(predicate::path::is_file());
    let busted_dir = vendor_dir.child("busted@2.2.0-1");
    busted_dir.assert(predicate::path::is_dir());
    let luasystem_rockspec = vendor_dir.child("luasystem-0.6.2-1.rockspec");
    luasystem_rockspec.assert(predicate::path::is_file());
    let luasystem_dir = vendor_dir.child("luasystem@0.6.2-1");
    luasystem_dir.assert(predicate::path::is_dir());
    let say_rockspec = vendor_dir.child("say-1.4.1-3.rockspec");
    say_rockspec.assert(predicate::path::is_file());
    let say_dir = vendor_dir.child("say@1.4.1-3");
    say_dir.assert(predicate::path::is_dir());
}
