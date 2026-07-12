use clap::Args;
use lux_lib::{package::PackageName, project::Project, rockspec::Rockspec, workspace::Workspace};
use miette::{IntoDiagnostic, Result};
use std::path::PathBuf;

use crate::args::OutputFormat;

#[derive(Args)]
pub struct GenerateRockspec {
    /// Package to generate the rockspec for.
    #[arg(short, long, visible_short_alias = 'p')]
    package: Option<PackageName>,

    #[arg(long, default_value = "text", value_enum, ignore_case = true)]
    output_format: OutputFormat,
}

pub async fn generate_rockspec(data: GenerateRockspec) -> Result<()> {
    let workspace = Workspace::current_or_err()?;

    let targets: Vec<&Project> = match &data.package {
        Some(package) => vec![workspace.select_member(package)?],
        None => workspace.members().into_iter().collect(),
    };

    let mut generated_paths = Vec::new();

    for project in targets {
        let path = generate_project_rockspec(project).await?;

        if data.output_format == OutputFormat::Text {
            println!("Wrote rockspec to {}", path.display());
        }

        generated_paths.push(path.to_string_lossy().into_owned());
    }

    if data.output_format == OutputFormat::Json {
        println!(
            "{}",
            serde_json::to_string(&generated_paths).into_diagnostic()?
        );
    }

    Ok(())
}

async fn generate_project_rockspec(project: &Project) -> Result<PathBuf> {
    let toml = project.toml().into_remote(None)?;
    let rockspec = toml.to_lua_remote_rockspec_string()?;

    let path = project
        .root()
        .join(format!("{}-{}.rockspec", toml.package(), toml.version()));

    tokio::fs::write(&path, rockspec).await.into_diagnostic()?;

    Ok(path)
}
