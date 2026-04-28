use std::path::PathBuf;

use clap::Args;
use eyre::Result;
use lux_lib::{config::Config, operations, package::PackageName, workspace::Workspace};

use crate::build::{self, Build};

#[derive(Args)]
pub struct Run {
    args: Vec<String>,

    /// Do not add `require('lux').loader()` to `LUA_INIT`.{n}
    /// If a rock has conflicting transitive dependencies,{n}
    /// disabling the Lux loader may result in the wrong modules being loaded.
    #[clap(default_value_t = false)]
    #[arg(long)]
    no_loader: bool,

    /// Path in which to run the command.{n}
    /// Defaults to the project root.
    #[arg(long)]
    dir: Option<PathBuf>,

    #[clap(flatten)]
    build: Build,

    /// Package with the target to run.
    #[arg(short, long, visible_short_alias = 'p')]
    package: Option<PackageName>,
}

pub async fn run(run_args: Run, config: Config) -> Result<()> {
    let workspace = Workspace::current_or_err()?;

    build::build(run_args.build, config.clone()).await?;

    operations::Run::new()
        .workspace(&workspace)
        .maybe_package(run_args.package)
        .args(&run_args.args)
        .config(&config)
        .disable_loader(run_args.no_loader)
        .run()
        .await?;

    Ok(())
}
