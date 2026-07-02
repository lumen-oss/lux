#[cfg(target_env = "msvc")]
use assert_fs::fixture::PathCopy;
#[cfg(target_env = "msvc")]
use assert_fs::TempDir;
#[cfg(target_env = "msvc")]
use lux_lib::lua_installation::detect_installed_lua_version;
#[cfg(target_env = "msvc")]
use lux_lib::operations::DistProjectBin;
#[cfg(target_env = "msvc")]
use lux_lib::project::Project;
#[cfg(target_env = "msvc")]
use lux_lib::{config::ConfigBuilder, lua_version::LuaVersion, tree::FlatDistTree};
#[cfg(target_env = "msvc")]
use std::path::PathBuf;

// This is a copy of the dist_bin tests, but as integration tests.
// The reason for copying them so that they are run with the
// Windows integration tests, in addition to the pure test suite we run with Nix.

#[tokio::test]
#[cfg(target_env = "msvc")]
async fn test_dist_bin_from_lua_source_compiles_and_runs() {
    let sample_project_path = "resources/test/sample-projects/only-src/";
    let expected_output = "1";
    let sample = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(sample_project_path);
    let project_dir = TempDir::new().unwrap();
    project_dir.copy_from(&sample, &["**"]).unwrap();

    let project = Project::from_exact(project_dir.path()).unwrap().unwrap();
    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));
    let config = ConfigBuilder::new()
        .unwrap()
        .lua_version(lua_version)
        .build()
        .unwrap();

    let staging = TempDir::new().unwrap();
    let tree = FlatDistTree::new(
        staging.to_path_buf(),
        config.lua_version().cloned().unwrap(),
        &config,
    )
    .unwrap();

    let out_dir = TempDir::new().unwrap();
    let binary = out_dir.path().join(if cfg!(target_env = "msvc") {
        "sample-project.exe"
    } else {
        "sample-project"
    });

    DistProjectBin::new()
        .project(&project)
        .config(&config)
        .tree(&tree)
        .output(binary.clone())
        .compile()
        .await
        .unwrap();

    assert!(binary.is_file(), "binary not produced");

    let out = tokio::process::Command::new(&binary)
        .output()
        .await
        .unwrap();

    assert!(out.status.success(), "binary exited non-zero:\n{:?}", out);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), expected_output);
}
