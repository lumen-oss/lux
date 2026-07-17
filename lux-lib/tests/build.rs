use std::path::PathBuf;

use assert_fs::prelude::PathCopy;
use assert_fs::TempDir;
use lux_lib::lua_version::LuaVersion;
use lux_lib::rockspec::Rockspec;
use lux_lib::tree::InstallTree;
use lux_lib::workspace::Workspace;
use lux_lib::{
    build::{Build, BuildBehaviour::Force},
    config::ConfigBuilder,
    lua_installation::{detect_installed_lua_version, LuaInstallation},
    lua_rockspec::RemoteLuaRockspec,
    tree,
};
use tokio::runtime::Builder;

#[cfg(not(target_env = "msvc"))]
use lux_lib::build::BuildBehaviour;

#[tokio::test]
async fn builtin_build() {
    let dir = TempDir::new().unwrap();

    let content = String::from_utf8(
        std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("resources/test/lua-cjson-2.1.0-1.rockspec"))
        .unwrap())
    .unwrap();
    let rockspec = RemoteLuaRockspec::new(&content).unwrap();

    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

    let config = ConfigBuilder::new()
        .unwrap()
        .user_tree(Some(dir.to_path_buf()))
        .lua_version(lua_version)
        .no_progress(Some(true))
        .build()
        .unwrap();

    let lua = LuaInstallation::new_from_config(&config)
        .await
        .unwrap();

    let tree = config.user_tree(lua.version.clone()).unwrap();

    Build::new()
        .rockspec(&rockspec)
        .lua(&lua)
        .tree(&tree)
        .entry_type(tree::EntryType::Entrypoint)
        .config(&config)
        .behaviour(Force)
        .build()
        .await
        .unwrap();
}

#[tokio::test]
async fn make_build() {
    let dir = TempDir::new().unwrap();

    let content = String::from_utf8(
        std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("resources/test/make-project/make-project-scm-1.rockspec"))
        .unwrap())
    .unwrap();
    let rockspec = RemoteLuaRockspec::new(&content).unwrap();

    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

    let config = ConfigBuilder::new()
        .unwrap()
        .user_tree(Some(dir.to_path_buf()))
        .lua_version(lua_version)
        .build()
        .unwrap();

    let lua = LuaInstallation::new_from_config(&config)
        .await
        .unwrap();

    let tree = config.user_tree(lua.version.clone()).unwrap();

    Build::new()
        .rockspec(&rockspec)
        .lua(&lua)
        .tree(&tree)
        .entry_type(tree::EntryType::Entrypoint)
        .config(&config)
        .behaviour(Force)
        .build()
        .await
        .unwrap();
}

#[tokio::test]
async fn cmake_build() {
    let rockspec =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/luv-1.48.0-2.rockspec");
    test_build_rockspec(rockspec).await
}

#[cfg(not(target_env = "msvc"))] // luaposix does not build on msvc
#[tokio::test]
async fn command_build() {
    // The rockspec appears to be broken when using luajit headers on macos
    let config = ConfigBuilder::new().unwrap().build().unwrap();
    let lua_version = LuaVersion::from(&config).unwrap_or(&LuaVersion::Lua51);
    if cfg!(target_os = "macos") && *lua_version == LuaVersion::LuaJIT {
        println!("luaposix is broken on macos/luajit! Skipping...");
        return;
    }
    let rockspec =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/luaposix-35.1-1.rockspec");
    test_build_rockspec(rockspec).await
}

#[cfg(test)]
async fn test_build_rockspec(rockspec_path: PathBuf) {
    let dir = TempDir::new().unwrap();

    let content = String::from_utf8(std::fs::read(rockspec_path).unwrap()).unwrap();
    let rockspec = RemoteLuaRockspec::new(&content).unwrap();

    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

    let config = ConfigBuilder::new()
        .unwrap()
        .user_tree(Some(dir.to_path_buf()))
        .lua_version(lua_version)
        .build()
        .unwrap();

    let lua = LuaInstallation::new_from_config(&config)
        .await
        .unwrap();

    let tree = config.user_tree(lua.version.clone()).unwrap();

    Build::new()
        .rockspec(&rockspec)
        .lua(&lua)
        .tree(&tree)
        .entry_type(tree::EntryType::Entrypoint)
        .config(&config)
        .behaviour(Force)
        .build()
        .await
        .unwrap();
}

#[tokio::test]
async fn treesitter_parser_build() {
    if cfg!(target_env = "msvc") {
        println!("Skipping test that is flaky on Windows/MSVC");
        return;
    }

    let dir = TempDir::new().unwrap();

    let rockspec = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources/test/tree-sitter-rust-0.0.43.rockspec");
    let content = String::from_utf8(std::fs::read(rockspec).unwrap()).unwrap();
    let rockspec = RemoteLuaRockspec::new(&content).unwrap();

    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

    let config = ConfigBuilder::new()
        .unwrap()
        .user_tree(Some(dir.to_path_buf()))
        .lua_version(lua_version)
        .build()
        .unwrap();

    let lua = LuaInstallation::new_from_config(&config)
        .await
        .unwrap();

    let tree = config
        .user_tree(LuaVersion::from(&config).unwrap().clone())
        .unwrap();

    let package = Build::new()
        .rockspec(&rockspec)
        .lua(&lua)
        .tree(&tree)
        .entry_type(tree::EntryType::Entrypoint)
        .config(&config)
        .behaviour(Force)
        .build()
        .await
        .unwrap();

    let rock_layout = tree.installed_rock_layout(&package).unwrap();

    let folds_query = rock_layout
        .etc
        .join("queries")
        .join("rust")
        .join("folds.scm");
    assert!(folds_query.is_file());
}

#[tokio::test]
async fn treesitter_parser_build_source_queries() {
    if cfg!(target_env = "msvc") {
        println!("Skipping test that is flaky on Windows/MSVC");
        return;
    }

    let dir = TempDir::new().unwrap();

    let rockspec = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources/test/tree-sitter-tmux-scm-1.rockspec");
    let content = String::from_utf8(std::fs::read(rockspec).unwrap()).unwrap();
    let rockspec = RemoteLuaRockspec::new(&content).unwrap();

    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

    let config = ConfigBuilder::new()
        .unwrap()
        .user_tree(Some(dir.to_path_buf()))
        .lua_version(lua_version)
        .build()
        .unwrap();

    let lua = LuaInstallation::new_from_config(&config)
        .await
        .unwrap();

    let tree = config
        .user_tree(LuaVersion::from(&config).unwrap().clone())
        .unwrap();

    let package = Build::new()
        .rockspec(&rockspec)
        .lua(&lua)
        .tree(&tree)
        .entry_type(tree::EntryType::Entrypoint)
        .config(&config)
        .behaviour(Force)
        .build()
        .await
        .unwrap();

    let rock_layout = tree.installed_rock_layout(&package).unwrap();

    let highlights_query = rock_layout
        .etc
        .join("queries")
        .join("tmux")
        .join("highlights.scm");
    assert!(highlights_query.is_file());

    let injections_query = rock_layout
        .etc
        .join("queries")
        .join("tmux")
        .join("injections.scm");
    assert!(injections_query.is_file());

    let top_level_queries_dir = rock_layout.etc.join("queries");
    assert!(top_level_queries_dir.is_dir());
    let mut top_level_scm_files = Vec::new();
    let mut entries = tokio::fs::read_dir(&top_level_queries_dir).await.unwrap();
    while let Some(entry) = entries.next_entry().await.unwrap() {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "scm") {
            top_level_scm_files.push(path);
        }
    }
    assert!(
        top_level_scm_files.is_empty(),
        "query files should not be installed at the top level: {:?}",
        top_level_scm_files);
}

#[tokio::test]
async fn test_build_local_project_no_source() {
    let sample_project =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-projects/no-source/");
    let workspace_root = TempDir::new().unwrap();
    workspace_root.copy_from(&sample_project, &["**"]).unwrap();

    let workspace = Workspace::from_exact(&workspace_root).unwrap().unwrap();
    let project_toml = workspace.members().first().toml().into_local().unwrap();

    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

    let config = ConfigBuilder::new()
        .unwrap()
        .lua_version(lua_version)
        .build()
        .unwrap();

    let tree = workspace.tree(&config).unwrap();

    let lua = LuaInstallation::new_from_config(&config)
        .await
        .unwrap();

    let package = Build::new()
        .rockspec(&project_toml)
        .lua(&lua)
        .tree(&tree)
        .entry_type(tree::EntryType::Entrypoint)
        .config(&config)
        .behaviour(Force)
        .build()
        .await
        .unwrap();

    let rock_layout = tree.installed_rock_layout(&package).unwrap();
    let conf_file = rock_layout.conf.join("foo").join("bar.toml");
    assert!(conf_file.is_file());

    let plugin_file = rock_layout.etc.join("plugin").join("foo.lua");
    assert!(plugin_file.is_file());
}

#[tokio::test]
async fn test_build_local_project_only_src() {
    let sample_project =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-projects/only-src/");
    let workspace_root = assert_fs::TempDir::new().unwrap();
    workspace_root.copy_from(&sample_project, &["**"]).unwrap();

    let workspace = Workspace::from_exact(&workspace_root).unwrap().unwrap();
    let project_toml = workspace.members().first().toml().into_local().unwrap();

    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

    let config = ConfigBuilder::new()
        .unwrap()
        .lua_version(lua_version)
        .build()
        .unwrap();

    let tree = workspace.tree(&config).unwrap();

    let lua = LuaInstallation::new_from_config(&config)
        .await
        .unwrap();

    let pkg = Build::new()
        .rockspec(&project_toml)
        .lua(&lua)
        .tree(&tree)
        .entry_type(tree::EntryType::Entrypoint)
        .config(&config)
        .behaviour(Force)
        .build()
        .await
        .unwrap();

    let layout = tree.installed_rock_layout(&pkg).unwrap();
    assert!(layout.src.is_dir());
    assert!(layout.src.join("main.lua").is_file());
    assert!(layout.src.join("foo.lua").is_file());
}

#[test]
fn test_build_multiple_treesitter_parsers() {
    let dir = TempDir::new().unwrap();

    let content = String::from_utf8(
        std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("resources/test/tree-sitter-rust-0.0.43.rockspec"))
        .unwrap())
    .unwrap();
    let rockspec = RemoteLuaRockspec::new(&content).unwrap();

    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

    let runtime = Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    let mut handles = vec![];

    for i in 0..4 {
        let config = ConfigBuilder::new()
            .unwrap()
            .user_tree(Some(dir.join(format!("{i}"))))
            .lua_version(lua_version.clone())
            .build()
            .unwrap();

        let tree = config
            .user_tree(LuaVersion::from(&config).unwrap().clone())
            .unwrap();

        let config = config.clone();
        let tree = tree.clone();
        let rockspec = rockspec.clone();

        handles.push(runtime.spawn(async move {
            let lua = LuaInstallation::new_from_config(&config)
                .await
                .unwrap();

            Build::new()
                .rockspec(&rockspec)
                .lua(&lua)
                .tree(&tree)
                .entry_type(tree::EntryType::Entrypoint)
                .config(&config)
                .behaviour(Force)
                .build()
                .await
                .unwrap()
        }));
    }

    runtime.block_on(futures::future::join_all(handles));
}

#[tokio::test]
async fn build_project_with_git_dependency() {
    let sample_project = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources/test/sample-projects/git-dependency/");
    let workspace_root = assert_fs::TempDir::new().unwrap();
    workspace_root.copy_from(&sample_project, &["**"]).unwrap();

    let workspace = Workspace::from_exact(&workspace_root).unwrap().unwrap();
    let project_toml = workspace.members().first().toml().into_local().unwrap();

    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

    let config = ConfigBuilder::new()
        .unwrap()
        .lua_version(lua_version)
        .build()
        .unwrap();

    let tree = workspace.tree(&config).unwrap();

    let lua = LuaInstallation::new_from_config(&config)
        .await
        .unwrap();

    Build::new()
        .rockspec(&project_toml)
        .lua(&lua)
        .tree(&tree)
        .entry_type(tree::EntryType::Entrypoint)
        .config(&config)
        .behaviour(Force)
        .build()
        .await
        .unwrap();
}

#[cfg(not(target_env = "msvc"))]
#[tokio::test]
async fn test_multiline_command_build() {
    let sample_project = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources/test/sample-projects/command-build/");
    let workspace_root = TempDir::new().unwrap();
    workspace_root.copy_from(&sample_project, &["**"]).unwrap();
    let workspace = Workspace::from_exact(&workspace_root).unwrap().unwrap();
    let project_toml = workspace.members().first().toml().into_local().unwrap();

    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

    let config = ConfigBuilder::new()
        .unwrap()
        .lua_version(lua_version)
        .build()
        .unwrap();

    let tree = workspace.tree(&config).unwrap();

    let lua = LuaInstallation::new_from_config(&config)
        .await
        .unwrap();

    let package = Build::new()
        .rockspec(&project_toml)
        .lua(&lua)
        .tree(&tree)
        .entry_type(tree::EntryType::Entrypoint)
        .config(&config)
        .behaviour(BuildBehaviour::Force)
        .build()
        .await
        .unwrap();

    let rock_layout = tree.installed_rock_layout(&package).unwrap();
    let success_dir = rock_layout.src.join("success");
    assert!(success_dir.is_dir());
}

#[tokio::test]
async fn builtin_build_install_include() {
    let dir = TempDir::new().unwrap();

    let content = String::from_utf8(
        std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/luyoga-1.3-3.rockspec"))
        .unwrap())
    .unwrap();
    let rockspec = RemoteLuaRockspec::new(&content).unwrap();

    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));

    let config = ConfigBuilder::new()
        .unwrap()
        .user_tree(Some(dir.to_path_buf()))
        .lua_version(lua_version.clone())
        .build()
        .unwrap();

    let lua = LuaInstallation::new_from_config(&config)
        .await
        .unwrap();

    let tree = config.user_tree(lua.version.clone()).unwrap();

    Build::new()
        .rockspec(&rockspec)
        .lua(&lua)
        .tree(&tree)
        .entry_type(tree::EntryType::Entrypoint)
        .config(&config)
        .behaviour(Force)
        .build()
        .await
        .unwrap();

    let install_path = dir.path().join(lua_version.unwrap().to_string());
    let mut install_dir_entries = tokio::fs::read_dir(install_path).await.unwrap();
    let mut luyoga_path = None;
    while let Some(entry) = install_dir_entries.next_entry().await.unwrap() {
        if entry.file_type().await.unwrap().is_dir()
            && entry.file_name().into_string().unwrap().contains("luyoga")
        {
            luyoga_path = Some(entry.path());
        }
    }

    let install_spec = &rockspec.build().current_platform().install;
    for target in install_spec.lib.keys() {
        let full_path = &luyoga_path.as_ref().unwrap().join("lib").join(target);
        tokio::fs::try_exists(full_path).await.unwrap();
    }
}
