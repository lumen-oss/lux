use clap::Args;
use eyre::Result;
use lux_lib::{package::PackageName, project::Project, rockspec::Rockspec, workspace::Workspace};
use std::path::PathBuf;

#[derive(Args)]
pub struct GenerateRockspec {
    /// Package to generate the rockspec for.
    #[arg(short, long, visible_short_alias = 'p')]
    package: Option<PackageName>,

    /// Control the command's output to emit a JSON list of paths.
    #[arg(long)]
    porcelain: bool,
}

pub async fn generate_rockspec(data: GenerateRockspec) -> Result<()> {
    let workspace = Workspace::current_or_err()?;
    let mut generated_paths = Vec::new();

    if let Some(package) = data.package {
        let path =
            generate_project_rockspec(workspace.select_member(&package)?, data.porcelain).await?;
        generated_paths.push(path.to_string_lossy().into_owned());
    } else {
        for project in workspace.members() {
            let path = generate_project_rockspec(project, data.porcelain).await?;
            generated_paths.push(path.to_string_lossy().into_owned());
        }
    }

    // If porcelain mode is active, print all gathered paths as a single JSON array
    if data.porcelain {
        println!("{}", serde_json::to_string(&generated_paths)?);
    }

    Ok(())
}

async fn generate_project_rockspec(project: &Project, porcelain: bool) -> Result<PathBuf> {
    let toml = project.toml().into_remote(None)?;
    let rockspec = toml.to_lua_remote_rockspec_string()?;

    let path = project
        .root()
        .join(format!("{}-{}.rockspec", toml.package(), toml.version()));

    tokio::fs::write(&path, rockspec).await?;

    if !porcelain {
        println!("Wrote rockspec to {}", path.display());
    }

    Ok(path)
}
