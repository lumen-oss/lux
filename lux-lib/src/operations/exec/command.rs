use crate::{
    build::BuildBehaviour,
    config::Config,
    drivers::{InstallBuildDependencies, InstallPackages, InstallPackagesError},
    lua,
    operations::install,
    package,
    package::db::PackageDB,
    transaction::Transaction,
    tree::{self},
};
use miette::Diagnostic;
use thiserror::Error;

#[derive(Error, Debug, Diagnostic)]
#[error(transparent)]
pub enum Error {
    Install(#[from] Box<InstallPackagesError>),
    PackageVersionReq(#[from] package::version::ReqError),
    PackageDB(#[from] package::db::Error),
    Tree(#[from] tree::Error),
    LuaVersion(#[from] lua::version::Error),
}

impl From<InstallPackagesError> for Error {
    fn from(source: InstallPackagesError) -> Self {
        Self::Install(Box::new(source))
    }
}

/// Ensure that a command is installed.
/// This defaults to the local project tree if cwd is a project root.
pub(super) async fn install(command: &str, config: &Config) -> Result<(), Error> {
    let package = package::Req::new(command.into(), None)?;
    let tree = config.user_tree(lua::Version::from(config)?.clone())?;
    let regular = tree.regular();

    if regular
        .match_rocks_and(&package, |package| regular.is_installed(package))
        .is_ok_and(|matches| matches.is_found())
    {
        return Ok(());
    }
    let install_spec = install::Spec::new(package, tree::EntryType::Entrypoint)
        .build_behaviour(BuildBehaviour::Force)
        .build();
    let package_db = PackageDB::from_config(config).await?;
    let build_tree = tree.build()?;
    let build_transaction = Transaction::new(&tree, &build_tree)?;
    let build_dependencies = InstallBuildDependencies::new(config, &build_transaction)
        .package(install_spec.clone())
        .package_db(package_db.clone())
        .install()
        .await?;
    build_transaction.commit()?;

    let transaction = Transaction::new(&tree, &regular)?;
    InstallPackages::new(config, &transaction)
        .package(install_spec)
        .package_db(package_db)
        .install(&build_dependencies.token)
        .await?;
    transaction.commit()?;
    Ok(())
}
