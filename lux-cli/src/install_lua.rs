use std::path::Path;

use lux_lib::{config::Config, lua_installation::LuaInstallation, lua_version::LuaVersion};

use miette::{miette, Result};

pub async fn install_lua(config: Config) -> Result<()> {
    let version = LuaVersion::from(&config)?;

    // TODO: Detect when path already exists by checking `Lua::path()` and prompt the user
    // whether they'd like to forcefully reinstall.
    let lua = LuaInstallation::install(version, &config).await?;
    let lua_root = match version {
        LuaVersion::Luau => lua
            .bin()
            .as_ref()
            .and_then(|bin| bin.parent())
            .map(Path::to_path_buf),
        _ => lua
            .includes()
            .first()
            .and_then(|dir| dir.parent())
            .map(Path::to_path_buf),
    }
    .ok_or_else(|| {
        miette!(
            help = "ensure that pkg-config is installed and configured, and that you have Lua installed",
            "error getting the lua installation root directory"
        )
    })?;

    tracing::info!("Installed Lua ({}) to {}", version, lua_root.display());

    Ok(())
}
