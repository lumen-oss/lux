use crate::package;
use bon::Builder;
use miette::Diagnostic;
use thiserror::Error;

use crate::{
    build::{self, BuildBehaviour},
    config::Config,
    drivers::{InstallBuildDependencies, InstallPackages, InstallPackagesError},
    hash::HasIntegrity,
    lockfile::{self, LockedPackage},
    operations::install,
    package::db::PackageDB,
    pipeline::discover::FoundPackage,
    project::{self, IntoLocalRockspecError, Project},
    rockspec::Rockspec,
    tree::{self, InstallSession},
    workspace::{self, Workspace},
};

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Project(#[from] project::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LocalRockspec(#[from] IntoLocalRockspecError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    PackageDB(#[from] package::db::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] tree::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Install(#[from] Box<InstallPackagesError>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Build(#[from] Box<build::Error>),
    #[error("package '{0}' was not installed")]
    PackageNotInstalled(package::Name),
    #[error("failed to hash the project's sources")]
    Hash(#[source] std::io::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Workspace(#[from] workspace::Error),
}

impl From<InstallPackagesError> for Error {
    fn from(source: InstallPackagesError) -> Self {
        Self::Install(Box::new(source))
    }
}

impl From<build::Error> for Error {
    fn from(source: build::Error) -> Self {
        Self::Build(Box::new(source))
    }
}

/// Installs a project into a [`Slot`](tree::Slot).
/// Typically, you will want to use [`crate::operations::BuildWorkspace`].
/// Useful for installing a project and its dependencies outside of a workspace tree.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct InstallProject<'a, T>
where
    T: InstallSession,
{
    project: &'a Project,

    config: &'a Config,

    target: &'a T,

    #[builder(default = BuildBehaviour::Force)]
    behaviour: BuildBehaviour,

    /// Workspace whose lockfile is preferred during dependency resolution.
    workspace: Option<&'a Workspace>,

    /// Locked packages to prefer during dependency resolution.
    #[builder(setters(vis = "pub(crate)"))]
    locked_packages: Option<Vec<lockfile::PackageLock>>,
}

impl<
        T: InstallSession + Sync + Send,
        State: install_project_builder::State + install_project_builder::IsComplete,
    > InstallProjectBuilder<'_, T, State>
{
    /// Builds the project's root package, installing its dependencies through
    /// the pipeline. Returns the installed root package.
    pub async fn build(self) -> Result<LockedPackage, Error> {
        let args = self._build();
        let config = args.config;
        let project = args.project;
        let destination = args.target;

        let rockspec = project.build_rockspec()?;
        let name = rockspec.package().clone();

        let behaviour = if matches!(args.behaviour, BuildBehaviour::Ignore) {
            match destination.lockfile()?.entrypoint(&name) {
                Some(existing) => {
                    let source_hash = project.root().hash().await.map_err(Error::Hash)?;
                    if existing.version() == rockspec.version()
                        && existing.hashes().source == source_hash
                        && destination.is_installed(existing)
                    {
                        return Ok(existing.clone());
                    }
                    BuildBehaviour::Force
                }
                None => args.behaviour,
            }
        } else {
            args.behaviour
        };

        let root = FoundPackage::from_project_root(rockspec, project.root().to_path_buf());
        let mut package_db = match args.workspace {
            Some(workspace) => PackageDB::open(config, workspace).await?,
            None => PackageDB::from_config(config).await?,
        };
        if let Some(locked_packages) = args.locked_packages {
            package_db = package_db.with_locks(locked_packages);
        }
        let package_db = package_db.with_local(vec![root]);

        let install_spec = install::Spec::new(
            package::Req::from(name.clone()),
            tree::EntryType::Entrypoint,
        )
        .build_behaviour(behaviour)
        .build();

        let build_transaction = destination.build_transaction()?;
        let build_dependencies = InstallBuildDependencies::new(config, &build_transaction)
            .package(install_spec.clone())
            .package_db(package_db.clone())
            .install()
            .await?;
        build_transaction.commit()?;

        InstallPackages::new(config, destination)
            .package(install_spec)
            .package_db(package_db)
            .install(&build_dependencies.token)
            .await?
            .0
            .into_iter()
            .find(|package| package.name() == &name)
            .ok_or(Error::PackageNotInstalled(name))
    }
}
