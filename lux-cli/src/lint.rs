use std::path::PathBuf;

use clap::Args;
use itertools::Itertools;
use lux_lib::{config::Config, operations::Exec, workspace::Workspace};
use path_slash::PathBufExt;

use crate::utils::path::{classify_path, PathTarget};
use crate::workspace::top_level_ignored_files;
use miette::{IntoDiagnostic, Result};

#[derive(Args)]
pub struct Lint {
    /// Path to a workspace, directory, or Lua file to lint. Defaults to the current workspace.
    #[arg(long)]
    path: Option<PathBuf>,
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
    let target = match lint_args.path.as_deref() {
        Some(path) => classify_path(path)?,
        None => match Workspace::current()? {
            Some(workspace) => PathTarget::Workspace(Box::new(workspace)),
            None => PathTarget::Directory(std::env::current_dir().into_diagnostic()?),
        },
    };

    let (target_path, workspace) = match target {
        PathTarget::Workspace(ws) => (ws.root().to_slash_lossy().to_string(), Some(*ws)),
        PathTarget::Directory(dir) => {
            let ws = Workspace::from(&dir)?;
            (dir.to_slash_lossy().to_string(), ws)
        }
        PathTarget::File(file) => {
            let ws = match file.parent() {
                Some(parent) => Workspace::from(parent)?,
                None => None,
            };
            (file.to_slash_lossy().to_string(), ws)
        }
    };

    let check_args: Vec<String> = match lint_args.args {
        Some(args) => args,
        None if lint_args.no_ignore => Vec::new(),
        None => {
            let ignored_files = workspace.iter().flat_map(|workspace| {
                top_level_ignored_files(workspace)
                    .into_iter()
                    .map(|file| file.to_slash_lossy().to_string())
            });
            std::iter::once("--exclude-files".into())
                .chain(ignored_files)
                .collect_vec()
        }
    };

    Exec::new("luacheck", None, &config)
        .arg(target_path)
        .args(check_args)
        .disable_loader(true)
        .exec()
        .await?;

    Ok(())
}
