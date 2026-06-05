mod bin;
mod tarball;

pub use bin::*;
pub use tarball::*;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum Dist {
    /// Distribute a tarball of a flat install tree which includes all dependencies.
    Tarball(Tarball),
    /// Build and distribute a standalone executable{n}
    /// which runs on systems that do not have Lua installed.
    Bin(Bin),
}
