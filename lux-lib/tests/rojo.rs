use assert_fs::prelude::PathCopy;
use flaky_test::flaky_test;
use lux_lib::{
    config::ConfigBuilder, lua_version::LuaVersion, operations::BuildWorkspace, tree::RojoLayout,
    workspace::Workspace,
};
use std::path::PathBuf;

#[flaky_test(tokio, times = 5)]
async fn test_rojo_package_layout() {
    if std::env::var("LUX_SKIP_IMPURE_TESTS").unwrap_or("0".into()) == "1" {
        println!("Skipping impure test");
        return;
    }

    let project_root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-projects/hello-rojo");
    let temp_dir = assert_fs::TempDir::new().unwrap();
    temp_dir.copy_from(project_root, &["**/*"]).unwrap();

    let config = ConfigBuilder::new()
        .unwrap()
        .lua_version(Some(LuaVersion::Luau))
        .entrypoint_layout(RojoLayout)
        .build()
        .unwrap();

    let workspace = Workspace::from_exact(temp_dir.path()).unwrap().unwrap();
    BuildWorkspace::new(&workspace, &config)
        .no_lock(false)
        .only_deps(false)
        .build()
        .await
        .unwrap();

    let packages = temp_dir.path().join("Packages");
    assert!(packages
        .join("_Index/hello-rojo@0.1.0/hello-rojo/init.luau")
        .is_file());
    assert_eq!(
        std::fs::read_to_string(packages.join("hello-rojo.lua")).unwrap(),
        "return require(script.Parent._Index[\"hello-rojo@0.1.0\"][\"hello-rojo\"])\n"
    );
    assert!(packages.join("_Index/foo@1.0.0/foo/init.lua").is_file());
    assert_eq!(
        std::fs::read_to_string(packages.join("foo.lua")).unwrap(),
        "return require(script.Parent._Index[\"foo@1.0.0\"][\"foo\"])\n"
    );

    if which::which("rojo").is_err() {
        println!("Skipping rojo build: rojo is not installed");
        return;
    }

    let output = temp_dir.path().join("hello-rojo.rbxlx");
    let status = std::process::Command::new("rojo")
        .arg("build")
        .arg("-o")
        .arg(&output)
        .current_dir(temp_dir.path())
        .status()
        .unwrap();
    assert!(status.success());

    let place = std::fs::read_to_string(&output).unwrap();
    assert!(place.contains("hello-rojo@0.1.0"));
    assert!(place.contains("foo@1.0.0"));
    assert!(place.contains("hello from"));
}
