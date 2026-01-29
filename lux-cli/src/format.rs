use std::path::PathBuf;

use clap::Args;
use eyre::{Context, OptionExt, Result};
use lux_lib::project::Project;
use path_slash::PathExt;
use stylua_lib::Config;
use walkdir::WalkDir;

#[derive(Args)]
pub struct Fmt {
    /// Optional path to a workspace or Lua file to format.
    workspace_or_file: Option<PathBuf>,

    #[clap(default_value = "stylua")]
    #[arg(long)]
    backend: FmtBackend,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum FmtBackend {
    Stylua,
    EmmyluaCodestyle,
}

// TODO: Add `PathBuf` parameter that describes what directory or file to format here.
pub fn format(args: Fmt) -> Result<()> {
    let project = Project::current()?.ok_or_eyre(
        "`lx fmt` can only be executed in a lux project! Run `lx new` to create one.",
    )?;

    let config: Config = std::fs::read_to_string("stylua.toml")
        .or_else(|_| std::fs::read_to_string(".stylua.toml"))
        .map(|config: String| toml::from_str(&config).unwrap_or_default())
        .unwrap_or_default();

    WalkDir::new(project.root().join("src"))
        .into_iter()
        .chain(WalkDir::new(project.root().join("lua")))
        .chain(WalkDir::new(project.root().join("lib")))
        .chain(WalkDir::new(project.root().join("spec")))
        .chain(WalkDir::new(project.root().join("test")))
        .chain(WalkDir::new(project.root().join("tests")))
        .filter_map(Result::ok)
        .filter(|file| {
            args.workspace_or_file
                .as_ref()
                .is_none_or(|workspace_or_file| {
                    file.path().to_path_buf().starts_with(workspace_or_file)
                })
        })
        .try_for_each(|file| {
            if PathBuf::from(file.file_name())
                .extension()
                .is_some_and(|ext| ext == "lua")
            {
                let file = file.path();
                let unformatted_code = std::fs::read_to_string(file)?;
                let formatted_code = match args.backend {
                    FmtBackend::Stylua => stylua_lib::format_code(
                        &unformatted_code,
                        config,
                        None,
                        stylua_lib::OutputVerification::Full,
                    )
                    .context(format!("error formatting {} with stylua.", file.display()))?,
                    FmtBackend::EmmyluaCodestyle => {
                        let uri = file.to_slash_lossy().to_string();
                        emmylua_codestyle::reformat_code(
                            &unformatted_code,
                            &uri,
                            emmylua_codestyle::FormattingOptions::default(),
                        )
                    }
                };

                std::fs::write(file, formatted_code)
                    .context(format!("error writing formatted file {}.", file.display()))?
            };
            Ok::<_, eyre::Report>(())
        })?;

    // Format the rockspec

    let rockspec = project.root().join("extra.rockspec");

    if rockspec.exists() {
        let unformatted_code = std::fs::read_to_string(&rockspec)?;
        let formatted_code = match args.backend {
            FmtBackend::Stylua => stylua_lib::format_code(
                &unformatted_code,
                config,
                None,
                stylua_lib::OutputVerification::Full,
            )?,
            FmtBackend::EmmyluaCodestyle => {
                let uri = rockspec.to_slash_lossy().to_string();
                emmylua_codestyle::reformat_code(
                    &unformatted_code,
                    &uri,
                    emmylua_codestyle::FormattingOptions::default(),
                )
            }
        };

        std::fs::write(rockspec, formatted_code)?;
    }

    Ok(())
}
