use clap::Args;
use lux_lib::{config::Config, lua_rockspec::LuaModule, package::PackageReq, which};
use miette::Result;

#[derive(Args)]
pub struct Which {
    /// The module to search for.
    module: LuaModule,
    /// Only search in these packages.
    packages: Option<Vec<PackageReq>>,
}

pub fn which(args: Which, config: Config) -> Result<()> {
    let path = which::Which::new(args.module, &config)
        .packages(args.packages.unwrap_or_default())
        .search()?;
    print!("{}", path.display());
    Ok(())
}
