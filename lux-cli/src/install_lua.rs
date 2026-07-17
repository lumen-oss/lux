use lux_lib::{
    config::Config,
    lua_installation::LuaInstallation,
    lua_version::LuaVersion,
};

use miette::{miette, Result};

pub async fn install_lua(config: Config) -> Result<()> {
    let version_stringified = &LuaVersion::from(&config)?;

    // TODO: Detect when path already exists by checking `Lua::path()` and prompt the user
    // whether they'd like to forcefully reinstall.
    let lua = LuaInstallation::install(version_stringified, &config).await?;
    let _lua_root = lua
        .includes()
        .first()
        .and_then(|dir| dir.parent())
        .ok_or_else(|| miette!("error getting lua include parent directory"))?;

    Ok(())
}
