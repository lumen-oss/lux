use clap::Subcommand;
use lux_lib::config::Config;

use crate::util::{completion::Completion, man::Man};
use miette::Result;

mod completion;
mod man;
mod schema;

#[derive(Subcommand)]
pub enum Util {
    /// Generate autocompletion scripts for the shell.{n}
    /// Example: `lx completion zsh > ~/.zsh/completions/_lx`
    Completion(Completion),
    /// Generate manpages.
    Man(Man),
    /// Generate the JSON Schema for `lux.toml` or `config.toml`.
    Schema(schema::Schema),
}

pub async fn util(util: Util, _config: Config) -> Result<()> {
    match util {
        Util::Completion(completion) => completion::completion(completion).await,
        Util::Man(man) => man::man(man).await,
        Util::Schema(schema) => schema::schema(schema),
    }
}
