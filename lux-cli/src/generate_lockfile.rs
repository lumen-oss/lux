use lux_lib::{config::Config, operations, workspace::Workspace};
use miette::Result;

/// Generate a lockfile for the current project without installing its dependencies.
pub async fn generate_lockfile(config: Config) -> Result<()> {
    let workspace = Workspace::current_or_err()?;

    operations::GenerateLockfile::new(&config, &workspace)
        .generate()
        .await?;

    println!("Generated {}", workspace.lockfile_path().display());

    Ok(())
}
