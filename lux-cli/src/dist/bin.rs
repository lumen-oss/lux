use std::path::PathBuf;

use clap::Args;
use eyre::Result;
use lux_lib::{
    config::{Config, ConfigBuilder},
    operations::DistProjectBin,
    package::PackageName,
    tree::FlatDistTree,
    workspace::Workspace,
};
use tempfile::tempdir;

#[derive(Args)]
pub struct Bin {
    /// Output path for the compiled binary.{n}
    /// Defaults to `<package>[.exe]` in the current directory.
    #[arg(short, long, visible_short_alias = 'o')]
    pub output: Option<PathBuf>,

    /// Package to compile.{n}
    /// Prioritises local projects if in a workspace.{n}
    /// If not set, lux will attempt to compile the current project.{n}
    /// Must be set in multi-project workspaces.
    #[arg(short, long, visible_short_alias = 'p')]
    package: Option<PackageName>,

    /// Output a JSON path.
    #[arg(long)]
    pub porcelain: bool,
}

pub async fn bin(data: Bin, config: Config) -> Result<()> {
    let staging_dir = tempdir()?;
    let config = ConfigBuilder::from(config)
        .wrap_bin_scripts(Some(false))
        .user_tree(Some(staging_dir.path().to_path_buf()))
        .build()?;

    let workspace = Workspace::current_or_err()?;
    let project = match &data.package {
        None => workspace.single_member()?,
        Some(package) => workspace.select_member(package)?,
    };

    let lua_version = project.lua_version(&config)?;
    let tree = FlatDistTree::new(staging_dir.path().to_path_buf(), lua_version, &config)?;

    let out = DistProjectBin::new()
        .project(project)
        .config(&config)
        .tree(&tree)
        .maybe_output(data.output)
        .compile()
        .await?;

    if data.porcelain {
        println!("{}", serde_json::to_string(&out)?);
    } else {
        println!("Binary written to {}", out.display());
    }

    Ok(())
}
