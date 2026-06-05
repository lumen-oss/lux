mod bin;
mod flat_archive;

pub use bin::*;
pub use flat_archive::*;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum Dist {
    /// Distribute an archive of a flat install tree which includes all dependencies.{n}
    /// The resulting archive does not include the `etc` directory or build dependencies.{n}
    /// Unlike a Lux tree, dependency conflicts are not supported/handled.
    FlatArchive(FlatArchive),
    /// Build and distribute a standalone executable{n}
    /// which runs on systems that do not have Lua installed.
    Bin(Bin),
}
