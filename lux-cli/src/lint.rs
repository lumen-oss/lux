use clap::Args;
use eyre::Result;
use itertools::Itertools;
use lux_lib::{
    config::Config,
    operations::{Exec, Install, PackageInstallSpec},
    progress::MultiProgress,
    project::Project,
    tree,
};
use path_slash::PathBufExt;

use crate::project::top_level_ignored_files;

#[derive(Args)]
pub struct Lint {
    /// Arguments to pass to the luacheck command.{n}
    /// If you pass arguments to luacheck, Lux will not pass any default arguments.
    args: Option<Vec<String>>,
    /// By default, Lux will add top-level ignored files and directories{n}
    /// (like those in .gitignore) to luacheck's exclude files.{n}
    /// This flag disables that behaviour.{n}
    #[arg(long)]
    no_ignore: bool,
}

pub async fn lint(lint_args: Lint, config: Config) -> Result<()> {
    let project = Project::current_or_err()?;

    let luacheck =
        PackageInstallSpec::new("luacheck".parse()?, tree::EntryType::Entrypoint).build();

    Install::new(&config)
        .package(luacheck)
        .project(&project)?
        .progress(MultiProgress::new_arc(&config))
        .install()
        .await?;

    let check_args: Vec<String> = match lint_args.args {
        Some(args) => args,
        None if lint_args.no_ignore => Vec::new(),
        None => {
            let ignored_files = top_level_ignored_files(&project)
                .into_iter()
                .map(|file| file.to_slash_lossy().to_string());
            std::iter::once("--exclude-files".into())
                .chain(ignored_files)
                .collect_vec()
        }
    };

    Exec::new("luacheck", Some(&project), &config)
        .arg(project.root().to_slash_lossy())
        .args(check_args)
        .disable_loader(true)
        .exec()
        .await?;

    Ok(())
}
