use clap::Args;
use lux_lib::{config::Config, operations::Sync, workspace::Workspace};

use miette::Result;

#[derive(Args)]
pub struct SyncProject {
    /// Skip the integrity checks for installed rocks when syncing the project lockfile.
    #[arg(long)]
    no_integrity_check: bool,
}

/// Sync the current project's installed packages with its lux.toml.
pub async fn sync(args: SyncProject, config: Config) -> Result<()> {
    let workspace = Workspace::current_or_err()?;

    let report = Sync::new(&workspace, &config)
        .validate_integrity(!args.no_integrity_check)
        .test(true)
        .sync()
        .await?;

    if report.added().is_empty() && report.removed().is_empty() {
        println!("Already in sync.");
        return Ok(());
    }

    for pkg in report.added() {
        println!("+ {} {}", pkg.name(), pkg.version());
    }
    for pkg in report.removed() {
        println!("- {} {}", pkg.name(), pkg.version());
    }

    Ok(())
}
