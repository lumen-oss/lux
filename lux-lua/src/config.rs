use lux_lib::config::ConfigBuilder;
use mlua::{ExternalResult, Lua, Table};

use crate::lua_impls::{ConfigBuilderLua, ConfigLua};

pub fn config(lua: &Lua) -> mlua::Result<Table> {
    let table = lua.create_table()?;

    table.set(
        "default",
        lua.create_function(|_, ()| {
            ConfigBuilder::default()
                .build()
                .map(ConfigLua)
                .into_lua_err()
        })?,
    )?;

    table.set(
        "builder",
        lua.create_function(|_, ()| Ok(ConfigBuilderLua(ConfigBuilder::default())))?,
    )?;

    Ok(table)
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
                :only_sources("example")
                :namespace("example")
                :lua_dir("lua")
                :lua_version("5.1")
                :user_tree(tree)
                :verbose(true)
                :timeout(10)
                :cache_dir(cache)
                :data_dir(data)
                -- :entrypoint_layout("rockspec")
                :build()

            assert(full_config, "default config should not be nil")
            assert(#full_config:enabled_dev_servers() > 0, "enabled_dev_servers should not be empty")
            assert(full_config:server() == "https://example.com/", "server should be https://example.com")
            assert(#full_config:extra_servers() == 2, "extra_servers should have 2 elements")
            assert(full_config:extra_servers()[1] == "https://example.com/", "first extra server should be https://example.com")
            assert(full_config:extra_servers()[2] == "https://example2.com/", "second extra server should be https://example2.com")
            assert(full_config:only_sources() == "example", "only_sources should be https://example.com")
            assert(full_config:namespace() == "example", "namespace should be example")
            assert(full_config:lua_dir() == "lua", "lua_dir should be lua")
            assert(full_config:user_tree("5.1"), "tree should be not nil")
            assert(full_config:verbose(), "verbose should be true")
            assert(full_config:timeout() == 10, "timeout should be 10")
            assert(full_config:cache_dir() == cache, "cache_dir should be /cache")
            assert(full_config:data_dir() == data, "data_dir should be /data")
            -- assert(full_config.entrypoint_layout() == ...)
            "#,
        )
        .exec()
        .unwrap();
    }
}
