use crate::{
    project::DebugProject,
    unpack::{Unpack, UnpackRemote},
};
use clap::Subcommand;

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
}
