use inquire::Confirm;
use lux_lib::{config::Config, lua_version::LuaVersion, tree::InstallTree};

use miette::{IntoDiagnostic, Result};
use path_slash::PathBufExt;
use tracing::Instrument;

/// Purge the user tree
pub async fn purge(config: Config) -> Result<()> {
    let tree = config.user_tree(LuaVersion::from(&config)?.clone())?;

    let len = tree.list()?.len();

    if !config.no_prompt()
        && Confirm::new(&format!("Are you sure you want to purge all {len} rocks?"))
            .with_default(false)
            .prompt()
            .into_diagnostic()?
    {
        let root_dir = tree.root();

        tokio::fs::remove_dir_all(tree.root())
            .instrument(tracing::info_span!(
                "Purging",
                tree = root_dir.to_slash_lossy().to_string()
            ))
            .await
            .into_diagnostic()?;
    }

    Ok(())
}
