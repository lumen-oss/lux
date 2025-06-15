use eyre::Result;
use lux_lib::{
    config::{Config, LuaVersion},
    lockfile::PinnedState,
    operations,
    package::PackageReq,
    progress::MultiProgress,
};

use crate::utils::install::apply_build_behaviour;

#[derive(clap::Args)]
pub struct Install {
    /// Package or list of packages to install.
    package_req: Vec<PackageReq>,

    /// Pin the packages so that they don't get updated.
    #[arg(long)]
    pin: bool,

    /// Reinstall without prompt if a package is already installed.
    #[arg(long)]
    force: bool,
}

/// Install a rock into the user tree.
pub async fn install(data: Install, config: Config) -> Result<()> {
    let pin = PinnedState::from(data.pin);

    let lua_version = LuaVersion::from_config(&config)?.clone();
    let tree = config.user_tree(lua_version)?;

    let packages = apply_build_behaviour(data.package_req, pin, data.force, &tree)?;

    // TODO(vhyrro): If the tree doesn't exist then error out.
    operations::Install::new(&config)
        .packages(packages)
        .tree(tree)
        .progress(MultiProgress::new_arc(&config))
        .install()
        .await?;

    Ok(())
}
