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
    /// Compile a Lux project, including its dependencies, into a single static binary,{n}
    /// which runs on systems that do not have Lua installed.{n}
    /// As with flat-archive, dependency conflicts are not supported/handled.{n}
    /// {n}
    /// The entrypoint is specified via the lux.toml's [run] field, e.g.: {n}
    /// {n}
    /// ```toml{n}
    /// [run]{n}
    /// args = ["src/main.lua"]{n}
    /// ```
    Bin(Bin),
}
