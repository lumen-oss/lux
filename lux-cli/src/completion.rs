use std::env;
use std::path::PathBuf;
use std::str::FromStr;

use clap::Args;
use clap::CommandFactory;
use clap_complete::generate as clap_generate;
use clap_complete::Shell;
use miette::miette;
use miette::Result;

use crate::Cli;

#[derive(Args)]
pub struct Completion {
    /// The shell to generate the completion script for.{n}
    /// If not set, Lux will try to detect the current shell.{n}
    /// Possible values: "bash", "elvish", "fish", "powershell", "zsh"{n}
    #[arg(value_enum)]
    shell: Option<Shell>,
}

pub async fn completion(args: Completion) -> Result<()> {
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
    clap_generate(shell, &mut Cli::command(), "lx", &mut std::io::stdout());
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
            completion(Completion { shell: Some(shell) }).await.unwrap();
        }
    }
}
