use std::path::{Path, PathBuf};

use clap::Args;
use emmylua_check::OutputDestination;
use itertools::Itertools;
use lux_lib::{config::Config, lua_version::LuaVersion, workspace::Workspace};

use miette::{miette, Result};

use crate::{
    args::OutputFormat,
    utils::{
        self,
        path::{classify_path, PathTarget},
    },
    workspace::sync_dependencies_if_locked,
};

#[derive(Args)]
pub struct Check {
    /// Path to a workspace, directory, or Lua file to check. Defaults to the current workspace.
    #[arg(long)]
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
            sync_dependencies_if_locked(&workspace, &config).await?;

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
        PathTarget::Directory(dir) => {
            let rc = resolve_rc_files(&dir, &config)?;
            (vec![dir], rc)
        }
        PathTarget::File(file) => {
            let root = file
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf();

            let rc = resolve_rc_files(&root, &config)?;
            (vec![root], rc)
        }
    };

    if workspace_dirs.is_empty() {
        println!("Nothing to check!");
        return Ok(());
    }

    if config.lua_version().is_some_and(|v| v == &LuaVersion::Luau) {
        if args.ignore.is_some()
            || args.output_format == OutputFormat::Json
            || args.warnings_as_errors
        {
            tracing::warn!(
                "--ignore, --output-format and --warnings-as-errors are not supported by luau-analyze"
            );
        }
        utils::luau_analyze::run(
            &config,
            workspace_dirs
                .iter()
                .map(|dir| dir.to_string_lossy().to_string())
                .collect_vec(),
            Vec::new(),
        )
        .await
    } else {
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
            .map_err(|err| miette!("{err}"))?;
        Ok(())
    }
}

fn resolve_rc_files(root: &Path, config: &Config) -> Result<Option<Vec<PathBuf>>> {
    let rc_path = match Workspace::from(root)? {
        Some(workspace) => workspace.luarc_path(config),
        None => root.join(".luarc.json"),
    };

    if rc_path.is_file() {
        Ok(Some(vec![rc_path]))
    } else {
        Ok(None)
    }
}
