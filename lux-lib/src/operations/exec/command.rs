use crate::{
    build::BuildBehaviour,
    config::Config,
    drivers::{InstallPackages, InstallPackagesError},
    lua,
    operations::PackageInstallSpec,
    package,
    package::db::PackageDB,
    tree::{self, InstallTree},
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

    if tree
        .match_rocks(&package)
        .is_ok_and(|matches| matches.is_found())
    {
        return Ok(());
    }
    let install_spec = PackageInstallSpec::new(package, tree::EntryType::Entrypoint)
        .build_behaviour(BuildBehaviour::Force)
        .build();
    InstallPackages::new(config, &tree)
        .package(install_spec)
        .package_db(PackageDB::from_config(config).await?)
        .install()
        .await?;
    Ok(())
}
