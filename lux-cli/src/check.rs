use clap::{Args, ValueEnum};
use emmylua_check::OutputDestination;
use eyre::{eyre, Result};
use itertools::Itertools;
use lux_lib::{config::Config, progress::MultiProgress, project::Project};

use crate::utils::project::{sync_dependencies_if_locked, sync_test_dependencies_if_locked};

#[derive(Args)]
pub struct Check {
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

#[derive(Debug, Clone, PartialEq, ValueEnum)]
enum OutputFormat {
    Json,
    Text,
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
    let project = Project::current_or_err()?;

    let progress = MultiProgress::new_arc(&config);
    sync_dependencies_if_locked(&project, progress.clone(), &config).await?;
    sync_test_dependencies_if_locked(&project, progress, &config).await?;

    let project_root = project.root();
    let workspace = vec![
        project_root.join("src"),
        project_root.join("lua"),
        project_root.join("test"),
        project_root.join("tests"),
        project_root.join("spec"),
    ]
    .into_iter()
    .filter(|dir| dir.is_dir())
    .collect_vec();

    if workspace.is_empty() {
        println!("Nothing to check!");
        return Ok(());
    }

    let luarc_path = project.luarc_path();
    let rc_files = if luarc_path.is_file() {
        Some(vec![luarc_path])
    } else {
        None
    };
    let emmylua_check_args = emmylua_check::CmdArgs {
        config: rc_files,
        workspace,
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
