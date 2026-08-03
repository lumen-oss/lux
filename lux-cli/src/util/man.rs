use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use clap::Args;
use clap::CommandFactory;
use miette::IntoDiagnostic;
use miette::Result;
use std::fs::File;
use tokio::fs;

use crate::Cli;

#[derive(Args)]
pub struct Man {
    /// The target directory in which to save man pages.
    #[arg(long)]
    target_dir: PathBuf,
}

pub async fn man(args: Man) -> Result<()> {
    fs::create_dir_all(&args.target_dir)
        .await
        .map_err(|err| {
            io::Error::other(format!(
                "error creating directory '{}': {}",
                args.target_dir.display(),
                err
            ))
        })
        .into_diagnostic()?;

    fn dist_manpage(dir: &Path, app: &clap::Command) -> Result<()> {
        let name = app.get_display_name().unwrap_or_else(|| app.get_name());
        let out_path = dir.join(format!("{name}.1"));
        let mut out = File::create(&out_path).into_diagnostic()?;

        clap_mangen::Man::new(app.clone())
            .render(&mut out)
            .into_diagnostic()?;
        out.flush().into_diagnostic()?;
        tracing::info!("generated {}", out_path.display());

        for sub in app.get_subcommands() {
            dist_manpage(dir, sub)?;
        }

        // So that `man lx` brings up `lux-cli.1`
        Ok(())
    }

    let mut cmd = Cli::command();
    cmd.build();
    dist_manpage(&args.target_dir, &cmd)?;
    let src = args.target_dir.join("lux-cli.1");
    let dest = args.target_dir.join("lx.1");
    fs::copy(&src, &dest)
        .await
        .map_err(|err| {
            io::Error::other(format!(
                "error copying '{}' to '{}':\n{}",
                src.display(),
                dest.display(),
                err
            ))
        })
        .into_diagnostic()?;
    Ok(())
}
