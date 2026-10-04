use clap::Args;
use lux_lib::{
    config::Config,
    operations::{pipeline::install_workspace::InstallWorkspaceDependencies, GenLuaRc},
    workspace::Workspace,
};

use miette::Result;

#[derive(Args)]
pub struct SyncProject {
    /// Skip the integrity checks for installed rocks when syncing the project lockfile.
    #[arg(long)]
    no_integrity_check: bool,
}

/// Sync the current project's installed packages with its lux.toml.
pub async fn sync(args: SyncProject, config: Config) -> Result<()> {
    // FIXME(vhyrro): reimplement
    let _ = args.no_integrity_check;
    let workspace = Workspace::current_or_err()?;

    InstallWorkspaceDependencies::new(&config, &workspace)
        .test(true)
        .install()
        .await?;

    GenLuaRc::new()
        .config(&config)
        .workspace(&workspace)
        .generate_luarc()
        .await?;

    // FIXME(vhyrro): Readd report publishing

    Ok(())
}
