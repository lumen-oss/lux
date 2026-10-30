use assert_fs::TempDir;
use lux_lib::{config, lua};
use lux_lua::lua_impls::TreeLua;
use mlua::{IntoLua, Lua};

#[tokio::test]
async fn tree_userdata() {
    let temp = TempDir::new().unwrap();

    let lua = Lua::new();

    let lua_version = lua::detect::installed_version().or(Some(lua::Version::Lua51));

    let config = config::Builder::new()
        .unwrap()
        .user_tree(Some(temp.to_path_buf()))
        .lua_version(lua_version)
        .build()
        .unwrap();
    let t = config.user_tree(lua::Version::Lua51).unwrap();
    let tree = TreeLua(t).into_lua(&lua).unwrap();
    lua.globals().set("tree", tree).unwrap();

    lua.load(
        r#"
        print(tree:bin())
    "#,
    )
    .exec()
    .unwrap();
}
