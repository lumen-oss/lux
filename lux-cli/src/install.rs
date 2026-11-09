use lux_lib::package;
use std::path::PathBuf;

use lux_lib::{
    config::Config,
    drivers::{InstallBuildDependencies, InstallPackages},
    lockfile::PinnedState,
    lua, operations,
    package::db::PackageDB,
    transaction::Transaction,
    workspace::{self, Workspace},
};

use miette::Result;

use crate::utils::install::apply_build_behaviour;

#[derive(clap::Args)]
pub struct Install {
    /// Package or list of packages to install.
    package_req: Vec<package::Req>,

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
    package: Option<package::Name>,
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
    package: Option<package::Name>,
    config: Config,
) -> Result<()> {
    let workspace =
        Workspace::from(&path)?.ok_or_else(|| workspace::Error::NoWorkspaceOrProject(path))?;

    let project = workspace.single_member_or_select(&package)?;

    let lua_version = project.lua_version(&config)?;
    let tree = config.user_tree(lua_version)?;
    let regular = tree.regular();

    let transaction = Transaction::new(&tree, &regular)?;
    operations::InstallProject::new()
        .project(project)
        .config(&config)
        .target(&transaction)
        .workspace(&workspace)
        .build()
        .await?;
    transaction.commit()?;

    Ok(())
}

async fn install_remote(data: Install, config: Config) -> Result<()> {
    let pin = PinnedState::from(data.pin);

    let lua_version = lua::Version::from(&config)?.clone();
    let tree = config.user_tree(lua_version)?;
    let build_tree = tree.build()?;
    let regular = tree.regular();

    let packages = apply_build_behaviour(data.package_req, pin, data.force, &regular, &config)?;

    // TODO(vhyrro): If the tree doesn't exist then error out.
    let package_db = PackageDB::from_config(&config).await?;
    let build_transaction = Transaction::new(&tree, &build_tree)?;
    let build_dependencies = InstallBuildDependencies::new(&config, &build_transaction)
        .packages(packages.clone())
        .package_db(package_db.clone())
        .install()
        .await?;
    build_transaction.commit()?;

    let transaction = Transaction::new(&tree, &regular)?;
    InstallPackages::new(&config, &transaction)
        .packages(packages)
        .package_db(package_db)
        .install(&build_dependencies.token)
        .await?;
    transaction.commit()?;

    Ok(())
}
