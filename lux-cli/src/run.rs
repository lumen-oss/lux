use std::path::PathBuf;

use clap::Args;
use lux_lib::{build::BuildBehaviour, config::Config, operations, workspace::Workspace};
use miette::Result;

use crate::build::{self, Build};

#[derive(Args)]
pub struct Run {
    #[arg(trailing_var_arg = true)]
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
    pub(crate) build: Build,
}

pub async fn run(run_args: Run, config: Config) -> Result<()> {
    let workspace = Workspace::current_or_err()?;

    let package = run_args.build.package.clone();
    build::build_with_behaviour(run_args.build, config.clone(), BuildBehaviour::Ignore).await?;

    operations::Run::new()
        .workspace(&workspace)
        .maybe_package(package)
        .args(&run_args.args)
        .config(&config)
        .disable_loader(run_args.no_loader)
        .run()
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{Cli, Commands};
    use clap::error::ErrorKind;
    use clap::Parser;

    #[test]
    fn forwards_arguments_after_first_positional() {
        let cli = Cli::try_parse_from(["lx", "run", "script.lua", "--help", "-x"]).unwrap();
        let Commands::Run(run) = cli.command else {
            unreachable!()
        };
        assert_eq!(run.args, ["script.lua", "--help", "-x"]);
    }

    #[test]
    fn run_help_is_not_forwarded() {
        let err = match Cli::try_parse_from(["lx", "run", "--help"]) {
            Err(err) => err,
            Ok(_) => unreachable!(),
        };
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }
}
