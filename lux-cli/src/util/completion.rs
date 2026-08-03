use std::env;
use std::io;
use std::path::PathBuf;
use std::str::FromStr;

use clap::Args;
use clap::CommandFactory;
use clap_complete::generate as clap_generate;
use clap_complete::generate_to;
use clap_complete::Shell;
use miette::miette;
use miette::IntoDiagnostic;
use miette::Result;
use tokio::fs;

use crate::Cli;

#[derive(Args)]
pub struct Completion {
    /// The shell to generate the completion script for.{n}
    /// If not set, and no target directory is specified,{n}
    /// ux will try to detect the current shell.{n}
    /// If not set, and a target directory is specified,{n}
    /// Lux will generate completions for all supported shells.
    /// Possible values: "bash", "elvish", "fish", "powershell", "zsh"{n}
    #[arg(value_enum)]
    shell: Option<Shell>,

    /// The target directory in which to save completions.
    /// If not set, Lux will print the completions to stdout.
    #[arg(long)]
    target_dir: Option<PathBuf>,
}

pub async fn completion(args: Completion) -> Result<()> {
    let cmd = &mut Cli::command();
    match &args.target_dir {
        None => {
            let shell = match args.shell {
                Some(shell) => shell,
                None => {
                    let shell_var: PathBuf = env::var("SHELL")
                        .map_err(|_| {
                            miette!(
                                r#"could not auto-detect the shell
Please make sure the SHELL environment variable is set
or specify the shell for which to generate completions.

Example: `lx completion zsh`

Supported shells: "bash", "elvish", "fish", "powershell", "zsh"
"#
                            )
                        })?
                        .into();
                    let shell_name = shell_var
                        .file_name()
                        .unwrap_or_else(|| shell_var.as_os_str())
                        .to_string_lossy();
                    Shell::from_str(&shell_name).map_err(|_| {
                        miette!(
                            r#"unsupported shell: {}.
Please specify the shell for which to generate completions.

Example: `lx completion zsh`

Supported shells: "bash", "elvish", "fish", "powershell", "zsh"
"#,
                            &shell_name
                        )
                    })?
                }
            };
            clap_generate(shell, cmd, "lx", &mut std::io::stdout());
        }
        Some(target_dir) => {
            fs::create_dir_all(&target_dir)
                .await
                .map_err(|err| {
                    io::Error::other(format!(
                        "error creating directory '{}': {}",
                        target_dir.display(),
                        err
                    ))
                })
                .into_diagnostic()?;
            use clap::ValueEnum;
            let shells = match args.shell {
                Some(shell) => &[shell],
                None => Shell::value_variants(),
            };
            for shell in shells {
                let output = generate_to(*shell, cmd, "lx", target_dir).into_diagnostic()?;
                tracing::info!("generated {}", output.display());
            }
        }
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_completion() {
        for shell in [
            Shell::Bash,
            Shell::Zsh,
            Shell::Fish,
            Shell::Elvish,
            Shell::PowerShell,
        ] {
            completion(Completion {
                shell: Some(shell),
                target_dir: None,
            })
            .await
            .unwrap();
        }
    }
}
