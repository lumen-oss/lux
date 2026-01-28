#![cfg_attr(feature = "test", allow(unused_imports, dead_code))]

use mlua::prelude::*;

mod config;
mod loader;
mod operations;
mod project;

#[cfg_attr(not(feature = "test"), mlua::lua_module)]
fn lux(lua: &Lua) -> LuaResult<LuaTable> {
    #[cfg(not(any(
        feature = "lua51",
        feature = "lua52",
        feature = "lua53",
        feature = "lua54",
        feature = "lua55",
        feature = "luajit",
        feature = "test"
    )))]
    compile_error!(
        "
        At least one Lua version feature must be enabled. \
        Please enable one of the following features: \
        lua51, lua52, lua53, lua54, lua55, luajit."
    );

    let exports = lua.create_table()?;

    exports.set(
        "loader",
        lua.create_function(|lua, ()| loader::load_loader(lua))?,
    )?;
    exports.set("config", config::config(lua)?)?;
    exports.set("project", project::project(lua)?)?;
    exports.set("operations", operations::operations(lua)?)?;

    Ok(exports)
}
