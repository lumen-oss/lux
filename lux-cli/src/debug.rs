use crate::{
    project::DebugProject,
    unpack::{Unpack, UnpackRemote},
};
use clap::{Args, Subcommand};

#[derive(Args)]
pub struct Dependencies {
    /// Output format: human-readable or json
    #[arg(long, default_value = "human")]
    pub format: OutputFormat,
}

#[derive(Clone, clap::ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
}

#[derive(Subcommand)]
pub enum Debug {
    /// Unpack the contents of a rock.
    Unpack(Unpack),
    /// Fetch a remote rock from its RockSpec source.
    FetchRemote(UnpackRemote),
    /// Download a .src.rock from luarocks.org and unpack it.
    UnpackRemote(UnpackRemote),
    /// View information about the current project.
    Project(DebugProject),
    /// Check for required dependencies (C compiler, make, cmake, cargo, pkg-config).
    Dependencies(Dependencies),
}
