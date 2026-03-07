//! Functions for interacting with global state (currently installed packages user-wide,
//! getting all packages from the manifest, etc.)

use std::collections::HashMap;

use lux_lib::{
    lua::lua_runtime,
    progress::{Progress, ProgressBar},
    remote_package_db::RemotePackageDB,
};
use mlua::prelude::*;

use crate::lua_impls::ConfigLua;

pub fn operations(lua: &Lua) -> mlua::Result<LuaTable> {
    let table = lua.create_table()?;

    table.set(
        "search",
        lua.create_async_function(|_, (query, config): (String, ConfigLua)| async move {
            let _runtime = lua_runtime().enter();

            search(query, config).await
        })?,
    )?;

    Ok(table)
}

async fn search(query: String, config: ConfigLua) -> mlua::Result<HashMap<String, Vec<String>>> {
    let remote_db =
        RemotePackageDB::from_config(&config.0, &Progress::<ProgressBar>::no_progress())
            .await
            .into_lua_err()?;

    Ok(remote_db
        .search(&query.parse().into_lua_err()?)
        .into_iter()
        .map(|(name, versions)| {
            (
                name.to_string(),
                versions.into_iter().map(|v| v.to_string()).collect(),
            )
        })
        .collect())
}
