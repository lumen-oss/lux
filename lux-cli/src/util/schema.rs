use clap::ValueEnum;
use lux_lib::schema;
use miette::{IntoDiagnostic, Result};

use crate::args::OutputFormat;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum SchemaKind {
    /// The project manifest (`lux.toml`).
    LuxToml,
    /// The Lux configuration file (`config.toml`).
    Config,
}

#[derive(clap::Args)]
pub struct Schema {
    /// Which schema to generate.
    #[arg(long, value_enum, default_value = "lux-toml", ignore_case = true)]
    kind: SchemaKind,

    /// Output format: `json` (JSON Schema) or `text` (Markdown).
    #[arg(long, default_value = "json", value_enum, ignore_case = true)]
    output_format: OutputFormat,
}

pub fn schema(cmd: Schema) -> Result<()> {
    let schema = match cmd.kind {
        SchemaKind::LuxToml => schema::lux_toml(),
        SchemaKind::Config => schema::config(),
    };
    match cmd.output_format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&schema).into_diagnostic()?
            )
        }
        OutputFormat::Text => print!("{}", schema::to_markdown(&schema)),
    }
    Ok(())
}
