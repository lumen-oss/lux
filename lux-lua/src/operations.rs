//! Functions for interacting with global state (currently installed packages user-wide,
//! getting all packages from the manifest, etc.)

use std::collections::HashMap;

use lux_lib::{
    config::Config,
    lua::lua_runtime,
    package::{PackageName, PackageVersion},
    progress::Progress,
    remote_package_db::PackageDB,
};
use mlua::prelude::*;

pub fn operations(lua: &Lua) -> mlua::Result<LuaTable> {
    let table = lua.create_table()?;

    table.set(
        "search",
        lua.create_async_function(|_, (query, config)| async move {
            let _runtime = lua_runtime().enter();

            search(query, &config).await
        })?,
    )?;

    Ok(table)
}

async fn search(
    query: String,
    config: &Config,
) -> mlua::Result<HashMap<PackageName, Vec<PackageVersion>>> {
    let remote_db = PackageDB::from_config(config, &Progress::no_progress())
        .await
        .into_lua_err()?;

    Ok(remote_db
        .search(&query.parse().into_lua_err()?)
        .into_iter()
        .map(|(name, versions)| (name.clone(), versions.into_iter().cloned().collect()))
        .collect())
}
