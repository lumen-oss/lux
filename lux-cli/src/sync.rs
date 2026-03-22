use clap::Args;
use eyre::{OptionExt, Result};
use lux_lib::{
    config::Config, lockfile::LocalPackage, operations::Sync, progress::MultiProgress,
    project::Project,
};

#[derive(Args)]
pub struct SyncProject {
    /// Skip the integrity checks for installed rocks when syncing the project lockfile.
    #[arg(long)]
    no_integrity_check: bool,
}

/// Sync the current project's installed packages with its lux.toml.
pub async fn sync(args: SyncProject, config: Config) -> Result<()> {
    let project = Project::current()?.ok_or_eyre("No project found")?;
    let progress = MultiProgress::new_arc(&config);

    let dep_report = Sync::new(&project, &config)
        .progress(progress.clone())
        .validate_integrity(!args.no_integrity_check)
        .sync_dependencies()
        .await?;

    let build_report = Sync::new(&project, &config)
        .progress(progress.clone())
        .validate_integrity(false)
        .sync_build_dependencies()
        .await?;

    let test_report = Sync::new(&project, &config)
        .progress(progress.clone())
        .validate_integrity(false)
        .sync_test_dependencies()
        .await?;

    let added: Vec<&LocalPackage> = dep_report
        .added()
        .iter()
        .chain(build_report.added().iter())
        .chain(test_report.added().iter())
        .collect();

    let removed: Vec<&LocalPackage> = dep_report
        .removed()
        .iter()
        .chain(build_report.removed().iter())
        .chain(test_report.removed().iter())
        .collect();

    if added.is_empty() && removed.is_empty() {
        println!("Already in sync.");
        return Ok(());
    }

    for pkg in added {
        println!("+ {} {}", pkg.name(), pkg.version());
    }
    for pkg in removed {
        println!("- {} {}", pkg.name(), pkg.version());
    }

    Ok(())
}
