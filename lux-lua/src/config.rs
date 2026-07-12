use lux_lib::config::ConfigBuilder;
use mlua::ExternalResult;
use mlua_extras::typed::{Type, Typed, TypedDataMethods, TypedUserData};

use crate::lua_impls::{ConfigBuilderLua, ConfigLua};

const DEFAULT_USER_AGENT: &str = concat!("lux-lua/", env!("CARGO_PKG_VERSION"));

#[derive(Clone)]
pub(crate) struct ConfigModule;

impl Typed for ConfigModule {
    fn ty() -> Type {
        Type::named("ConfigModule")
    }
}

impl TypedUserData for ConfigModule {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("Create a config builder that builds the default `Config`");
        methods.add_function("default", |_, ()| {
            ConfigBuilder::default()
                .user_agent(Some(DEFAULT_USER_AGENT.into()))
                .build()
                .map(ConfigLua)
                .into_lua_err()
        });

        methods.document("Create a new config builder, starting with a blank slate");
        methods.add_function("builder", |_, ()| {
            Ok(ConfigBuilderLua(ConfigBuilder::default()))
        });

        methods.document(
            r#"Create a new config builder by deserializing from a config file
if present, or otherwise by instantiating the default config"#,
        );
        methods.add_function("new", |_, ()| {
            ConfigBuilder::new().map(ConfigBuilderLua).into_lua_err()
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Module for building a Lux `Config`");
    }
}

impl mlua::UserData for ConfigModule {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[cfg(feature = "definitions")]
mod definitions_registry {
    use mlua_extras::typed::{Type, TypedClassBuilder};

    use super::ConfigModule;
    use crate::definitions::LuxDefinition;

    inventory::submit! {
        LuxDefinition {
            name: "ConfigModule",
            build: || Type::class(TypedClassBuilder::new::<ConfigModule>().build()),
        }
    }
}

#[cfg(test)]
mod tests {
    use mlua::prelude::*;

    #[test]
    fn lua_api_test_config() {
        let lua = Lua::new();

        lua.globals().set("lux", crate::lux(&lua).unwrap()).unwrap();

        lua.load(
            r#"
            local config = lux.config
            local default = config.default()
            assert(default, "default config should not be nil")
            "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_config_builder() {
        let lua = Lua::new();
        let tree = assert_fs::TempDir::new().unwrap();
        let cache = assert_fs::TempDir::new().unwrap();
        let data = assert_fs::TempDir::new().unwrap();

        lua.globals().set("lux", crate::lux(&lua).unwrap()).unwrap();
        lua.globals().set("tree", tree.path()).unwrap();
        lua.globals().set("cache", cache.path()).unwrap();
        lua.globals().set("data", data.path()).unwrap();

        lua.load(
            r#"
            local config = lux.config
            local full_config = config.builder()
                :dev(true)
                :server("https://example.com")
                :extra_servers({"https://example.com", "https://example2.com"})
                :namespace("example")
                :lua_dir("lua")
                :lua_version("5.1")
                :user_tree(tree)
                :verbose(true)
                :timeout(10)
                :cache_dir(cache)
                :data_dir(data)
                :entrypoint_layout({ layout = "nvim" })
                :build()

            assert(full_config, "default config should not be nil")
            assert(#full_config:enabled_dev_servers() > 0, "enabled_dev_servers should not be empty")
            assert(full_config:server() == "https://example.com/", "server should be https://example.com")
            assert(#full_config:extra_servers() == 2, "extra_servers should have 2 elements")
            assert(full_config:extra_servers()[1] == "https://example.com/", "first extra server should be https://example.com")
            assert(full_config:extra_servers()[2] == "https://example2.com/", "second extra server should be https://example2.com")
            assert(full_config:namespace() == "example", "namespace should be example")
            assert(full_config:lua_dir() == "lua", "lua_dir should be lua")
            assert(full_config:user_tree("5.1"), "tree should be not nil")
            assert(full_config:verbose(), "verbose should be true")
            assert(full_config:timeout() == 10, "timeout should be 10")
            assert(full_config:cache_dir() == cache, "cache_dir should be /cache")
            assert(full_config:data_dir() == data, "data_dir should be /data")
            assert(full_config:entrypoint_layout(), "entrypoint_layout should be not nil")
            "#,
        )
        .exec()
        .unwrap();
    }

    #[tokio::test]
    async fn lua_api_test_tree_lockfile_api() {
        let temp = assert_fs::TempDir::new().unwrap();
        let lua = Lua::new();
        lua.globals().set("user_tree", temp.path()).unwrap();
        lua.globals().set("lux", crate::lux(&lua).unwrap()).unwrap();
        lua.load(
            r#"
        local config = lux.config
        local full_config = config.builder()
            :user_tree(user_tree)
            :build()
        local tree = full_config:user_tree("5.5")
        local lockfile = tree:lockfile()
        print(lockfile:version())
    "#,
        )
        .exec()
        .unwrap();
    }
}
