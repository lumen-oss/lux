use clap::Subcommand;
use lux_lib::config::Config;

use crate::util::{completion::Completion, man::Man};
use miette::Result;

mod completion;
mod man;

#[derive(Subcommand)]
pub enum Util {
    /// Generate autocompletion scripts for the shell.{n}
    /// Example: `lx completion zsh > ~/.zsh/completions/_lx`
    Completion(Completion),
    /// Generate manpages.
    Man(Man),
}

pub async fn util(util: Util, _config: Config) -> Result<()> {
    match util {
        Util::Completion(completion) => completion::completion(completion).await,
        Util::Man(man) => man::man(man).await,
    }
}
