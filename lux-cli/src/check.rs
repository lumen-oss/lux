use std::path::{Path, PathBuf};

use clap::Args;
use emmylua_check::OutputDestination;
use eyre::{eyre, Result};
use itertools::Itertools;
use lux_lib::{config::Config, progress::MultiProgress, workspace::Workspace};

use crate::{
    args::OutputFormat,
    utils::path::{classify_path, PathTarget},
    workspace::{sync_dependencies_if_locked, sync_test_dependencies_if_locked},
};

#[derive(Args)]
pub struct Check {
    /// Path to a workspace, directory, or Lua file to check. Defaults to the current workspace.
    path: Option<PathBuf>,
    /// Comma-separated list of ignore patterns.
    /// Patterns must follow glob syntax.
    /// Lux will automatically add top-level ignored project files.
    #[arg(short, long, value_delimiter = ',')]
    ignore: Option<Vec<String>>,

    /// The output format.
    #[arg(long, default_value = "text", value_enum, ignore_case = true)]
    output_format: OutputFormat,

    /// Output destination.{n}
    /// (stdout or a file path, only used when the output format is json).
    #[arg(long, default_value = "stdout")]
    output: OutputDestination,

    /// Treat warnings as errors.
    #[arg(long)]
    warnings_as_errors: bool,
}

impl From<OutputFormat> for emmylua_check::OutputFormat {
    fn from(value: OutputFormat) -> Self {
        match value {
            OutputFormat::Json => emmylua_check::OutputFormat::Json,
            OutputFormat::Text => emmylua_check::OutputFormat::Text,
        }
    }
}

pub async fn check(args: Check, config: Config) -> Result<()> {
    let target = match args.path.as_deref() {
        None => PathTarget::Workspace(Box::new(Workspace::current_or_err()?)),
        Some(path) => classify_path(path)?,
    };

    let (workspace_dirs, rc_files) = match target {
        PathTarget::Workspace(workspace) => {
            let progress = MultiProgress::new_arc(&config);
            sync_dependencies_if_locked(&workspace, progress.clone(), &config).await?;
            sync_test_dependencies_if_locked(&workspace, progress, &config).await?;

            let dirs = workspace
                .members()
                .iter()
                .map(|project| project.root())
                .flat_map(|project_root| {
                    vec![
                        project_root.join("src"),
                        project_root.join("lua"),
                        // For now, we don't include tests
                        // because they require LLS_Addons definitions for busted

                        // project_root.join("test"),
                        // project_root.join("tests"),
                        // project_root.join("spec"),
                    ]
                })
                .filter(|dir| dir.is_dir())
                .collect_vec();

            let luarc_path = workspace.luarc_path(&config);

            let rc = if luarc_path.is_file() {
                Some(vec![luarc_path])
            } else {
                None
            };

            (dirs, rc)
        }
        PathTarget::Directory(dir) => (vec![dir], None),
        PathTarget::File(file) => {
            let root = file
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf();
            (vec![root], None)
        }
    };

    if workspace_dirs.is_empty() {
        println!("Nothing to check!");
        return Ok(());
    }

    let emmylua_check_args = emmylua_check::CmdArgs {
        config: rc_files,
        workspace: workspace_dirs,
        ignore: args.ignore,
        output_format: args.output_format.into(),
        output: args.output,
        warnings_as_errors: args.warnings_as_errors,
        verbose: config.verbose(),
    };

    emmylua_check::run_check(emmylua_check_args)
        .await
        .map_err(|err| eyre!(err.to_string()))?;
    Ok(())
}
