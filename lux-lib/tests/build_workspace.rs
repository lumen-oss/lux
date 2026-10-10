use std::path::PathBuf;

use assert_fs::prelude::PathCopy;
use assert_fs::TempDir;
use flaky_test::flaky_test;
use lux_lib::lua_version::LuaVersion;
use lux_lib::operations::BuildWorkspace;
use lux_lib::package::PackageName;
use lux_lib::tree::InstallTree;
use lux_lib::workspace::Workspace;
use lux_lib::{config::ConfigBuilder, lua_installation::detect_installed_lua_version};

#[flaky_test(tokio, times = 5)]
async fn test_build_multi_workspace_local_dependencies() {
    let sample_project = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources/test/sample-projects/multi-project-local-deps");
    let workspace_root = TempDir::new().unwrap();
    workspace_root.copy_from(&sample_project, &["**"]).unwrap();

    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

    let config = ConfigBuilder::new()
        .unwrap()
        .lua_version(lua_version.clone())
        .build()
        .unwrap();
    let workspace = Workspace::from_exact(&workspace_root).unwrap().unwrap();
    BuildWorkspace::new(&workspace, &config)
        .no_lock(false)
        .only_deps(false)
        .build()
        .await
        .unwrap();

    // Building an already-built workspace must be idempotent
    BuildWorkspace::new(&workspace, &config)
        .no_lock(false)
        .only_deps(false)
        .build()
        .await
        .unwrap();

    let lockfile = workspace.tree(&config).unwrap().lockfile().unwrap();

    let foo = lockfile
        .entrypoint(&PackageName::new("foo".into()))
        .expect("foo must be recorded as an entrypoint");
    let bar = lockfile
        .entrypoint(&PackageName::new("bar".into()))
        .expect("bar must be recorded as an entrypoint");

    assert!(
        foo.dependencies().is_empty(),
        "foo declares no dependencies and must not be linked to its sibling member bar"
    );
    assert!(
        bar.dependencies()
            .into_iter()
            .filter_map(|id| lockfile.get(id))
            .any(|pkg| pkg.name() == foo.name()),
        "bar's path dependency on foo must be recorded as a tree edge"
    );
}
