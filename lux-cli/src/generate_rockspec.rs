use clap::Args;
use eyre::Result;
use lux_lib::{package::PackageName, rockspec::Rockspec, workspace::Workspace};

#[derive(Args)]
pub struct GenerateRockspec {
    /// Package to generate the rockspec for.
    #[arg(short, long, visible_short_alias = 'p')]
    package: Option<PackageName>,
}

pub async fn generate_rockspec(data: GenerateRockspec) -> Result<()> {
    let workspace = Workspace::current_or_err()?;

    for project in workspace.try_members(&data.package)? {
        let toml = project.toml().into_remote(None)?;
        let rockspec = toml.to_lua_remote_rockspec_string()?;

        let path = project
            .root()
            .join(format!("{}-{}.rockspec", toml.package(), toml.version()));

        tokio::fs::write(&path, rockspec).await?;

        println!("Wrote rockspec to {}", path.display());
    }

    Ok(())
}
