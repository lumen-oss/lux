use crate::{
    args::OutputFormat,
    project::DebugProject,
    unpack::{Unpack, UnpackRemote},
};
use clap::{Args, Subcommand};

#[derive(Args)]
pub struct Toolchains {
    #[arg(long, default_value = "text")]
    pub format: OutputFormat,
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
    /// Check for available toolchains.
    Toolchains(Toolchains),
}
