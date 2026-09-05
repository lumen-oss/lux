use std::path::PathBuf;

use lux_lib::{
    config::Config,
    lockfile::PinnedState,
    lua_version::LuaVersion,
    operations,
    package::{PackageName, PackageReq},
    workspace::{Workspace, WorkspaceError},
};

use miette::Result;

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

    /// Install a local project from this directory, rather than a remote package.
    #[arg(long, value_name = "path")]
    path: Option<PathBuf>,

    /// Package to install when installing from a multi-project workspace.
    /// Ignored if the `--path` option is unset.
    #[arg(short, long, visible_short_alias = 'p')]
    package: Option<PackageName>,
}

/// Install a rock into the user tree.
pub async fn install(data: Install, config: Config) -> Result<()> {
    if let Some(path) = data.path {
        install_from_path(path, data.package, config).await
    } else {
        install_remote(data, config).await
    }
}

async fn install_from_path(
    path: PathBuf,
    package: Option<PackageName>,
    config: Config,
) -> Result<()> {
    let workspace =
        Workspace::from(&path)?.ok_or_else(|| WorkspaceError::NoWorkspaceOrProject(path))?;

    let project = workspace.single_member_or_select(&package)?;

    let lua_version = project.lua_version(&config)?;
    let tree = config.user_tree(lua_version)?;

    operations::InstallProject::new()
        .project(project)
        .config(&config)
        .tree(&tree)
        .build()
        .await?;

    Ok(())
}

async fn install_remote(data: Install, config: Config) -> Result<()> {
    let pin = PinnedState::from(data.pin);

    let lua_version = LuaVersion::from(&config)?.clone();
    let tree = config.user_tree(lua_version)?;

    let packages = apply_build_behaviour(data.package_req, pin, data.force, &tree, &config)?;

    // TODO(vhyrro): If the tree doesn't exist then error out.
    operations::Install::new(&config)
        .packages(packages)
        .tree(tree)
        .install()
        .await?;

    Ok(())
}
