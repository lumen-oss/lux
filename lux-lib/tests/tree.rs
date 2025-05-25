use assert_fs::TempDir;
use lux_lib::{
    config::{ConfigBuilder, LuaVersion},
    lua_installation::detect_installed_lua_version,
    operations::LuaBinary,
};
use mlua::{IntoLua, Lua};

#[tokio::test]
async fn tree_userdata() {
    let temp = TempDir::new().unwrap();

    let lua = Lua::new();

    let lua_version = detect_installed_lua_version(LuaBinary::default())
        .await
        .ok()
        .and_then(|version| LuaVersion::from_version(version).ok())
        .or(Some(LuaVersion::Lua51));

    let config = ConfigBuilder::new()
        .unwrap()
        .user_tree(Some(temp.to_path_buf()))
        .lua_version(lua_version)
        .build()
        .unwrap();
    let t = config.user_tree(LuaVersion::Lua51).unwrap();
    let tree = t.into_lua(&lua).unwrap();
    lua.globals().set("tree", tree).unwrap();

    lua.load(
        r#"
        print(tree:bin())
    "#,
    )
    .exec()
    .unwrap();
}
