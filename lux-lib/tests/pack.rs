use std::{fs::File, io::Read, path::PathBuf};

use assert_fs::{prelude::PathCopy, TempDir};
use lux_lib::{
    config::{ConfigBuilder, LuaVersion},
    lua_installation::detect_installed_lua_version,
    operations::{BuildProject, Pack},
    project::Project,
};
use mlua::Lua;
use zip::ZipArchive;

#[tokio::test]
async fn pack_project_with_etc_directories() {
    let project_root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-projects/init/");
    let temp_dir = TempDir::new().unwrap();
    temp_dir.copy_from(&project_root, &["**"]).unwrap();
    let project_root = temp_dir.path();
    let foo_module_dir = project_root.join("src").join("foo");
    tokio::fs::create_dir_all(&foo_module_dir).await.unwrap();
    let bar_module_file = foo_module_dir.join("bar.lua");
    tokio::fs::write(&bar_module_file, "print('foo.bar')")
        .await
        .unwrap();
    let plugin_dir = project_root.join("plugin");
    tokio::fs::create_dir_all(&plugin_dir).await.unwrap();
    let plugin_script = plugin_dir.join("foo.lua");
    tokio::fs::write(&plugin_script, "print('foo')")
        .await
        .unwrap();
    let config_file = project_root.join("cfg.toml");
    tokio::fs::write(&config_file, "enable = true")
        .await
        .unwrap();
    let project_toml_file = project_root.join("lux.toml");
    let project_toml_content = tokio::fs::read_to_string(&project_toml_file).await.unwrap();
    let project_toml_content = format!(
        r#"{project_toml_content}
copy_directories = [ "plugin" ]

[source]
url = "https://github.com/lumen-oss/luarocks-stub"

[build.install.conf]
"cfg.toml" = "cfg.toml"
"#
    );
    tokio::fs::write(&project_toml_file, project_toml_content)
        .await
        .unwrap();

    let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));
    let config = ConfigBuilder::new()
        .unwrap()
        .lua_version(lua_version)
        .build()
        .unwrap();
    let project = Project::from_exact(project_root).unwrap().unwrap();
    let tree = project.tree(&config).unwrap();
    let temp_dir = TempDir::new().unwrap();
    let dest_dir = temp_dir.to_path_buf();

    let package = BuildProject::new(&project, &config)
        .no_lock(false)
        .only_deps(false)
        .build()
        .await
        .unwrap()
        .unwrap();
    let archive_path = Pack::new(dest_dir, tree, package).pack().await.unwrap();
    let archive_file = File::open(&archive_path).unwrap();
    let mut archive = ZipArchive::new(archive_file).unwrap();
    let mut rock_manifest_entry = archive.by_name("rock_manifest").unwrap();
    let mut rock_manifest_content = String::new();
    rock_manifest_entry
        .read_to_string(&mut rock_manifest_content)
        .unwrap();

    let lua = Lua::new();
    lua.load(rock_manifest_content).exec().unwrap();
    let globals = lua.globals();
    let rock_manifest: mlua::Table = globals.get("rock_manifest").unwrap();
    let lua_mod: mlua::Table = rock_manifest.get("lua").unwrap();
    assert!(lua_mod.contains_key("foo").unwrap());
    let foo: mlua::Table = lua_mod.get("foo").unwrap();
    assert!(foo.contains_key("bar.lua").unwrap());
    let conf: mlua::Table = rock_manifest.get("conf").unwrap();
    assert!(conf.contains_key("cfg.toml").unwrap());
    let plugin: mlua::Table = rock_manifest.get("plugin").unwrap();
    assert!(plugin.contains_key("foo.lua").unwrap());
    assert!(plugin.contains_key("foo.lua").unwrap());
    assert!(rock_manifest
        .contains_key("sample-project-0.1.0-1.rockspec")
        .unwrap());
}
