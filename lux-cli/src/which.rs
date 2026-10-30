use clap::Args;
use lux_lib::package;
use lux_lib::{config::Config, rockspec, which};
use miette::Result;

#[derive(Args)]
pub struct Which {
    /// The module to search for.
    module: rockspec::build::backend::Module,
    /// Only search in these packages.
    packages: Option<Vec<package::Req>>,
}

pub fn which(args: Which, config: Config) -> Result<()> {
    let path = which::Which::new(args.module, &config)
        .packages(args.packages.unwrap_or_default())
        .search()?;
    print!("{}", path.display());
    Ok(())
}
