use clap::Args;
use eyre::Result;
use itertools::Itertools;
use lux_lib::{config::Config, operations::Exec, project::Project};
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
    let project = Project::current()?;
    let root_dir = match &project {
        Some(project) => project.root().to_slash_lossy().to_string(),
        None => std::env::current_dir()?.to_slash_lossy().to_string(),
    };

    let check_args: Vec<String> = match lint_args.args {
        Some(args) => args,
        None if lint_args.no_ignore => Vec::new(),
        None => {
            let ignored_files = project.iter().flat_map(|project| {
                top_level_ignored_files(project)
                    .into_iter()
                    .map(|file| file.to_slash_lossy().to_string())
            });
            std::iter::once("--exclude-files".into())
                .chain(ignored_files)
                .collect_vec()
        }
    };

    Exec::new("luacheck", None, &config)
        .arg(root_dir)
        .args(check_args)
        .disable_loader(true)
        .exec()
        .await?;

    Ok(())
}
