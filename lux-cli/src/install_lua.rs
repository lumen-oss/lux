use eyre::Result;
use lux_lib::{
    config::{Config, LuaVersion},
    lua_installation::LuaInstallation,
    progress::{MultiProgress, ProgressBar},
};

pub async fn install_lua(config: Config) -> Result<()> {
    let version_stringified = &LuaVersion::from_current_project_or_config(&config)?;

    let progress = MultiProgress::new(&config);

    let bar = progress.map(|progress| {
        progress.add(ProgressBar::from(format!(
            "🌔 Installing Lua ({version_stringified})",
        )))
    });

    // TODO: Detect when path already exists by checking `Lua::path()` and prompt the user
    // whether they'd like to forcefully reinstall.
    let lua = LuaInstallation::install(version_stringified, &config, &bar).await?;
    let lua_root = lua
        .includes()
        .first()
        .and_then(|dir| dir.parent())
        .expect("error getting parent directory");

    bar.map(|bar| {
        bar.finish_with_message(format!(
            "🌔 Installed Lua ({}) to {}",
            version_stringified,
            lua_root.display()
        ))
    });

    Ok(())
}
