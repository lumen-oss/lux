use bon::Builder;
use miette::Diagnostic;
use thiserror::Error;

use crate::{
    build::{BuildBehaviour, BuildError},
    config::Config,
    hash::HasIntegrity,
    lockfile::LockedPackage,
    operations::{
        pipeline::{
            discover::FoundPackage,
            install_packages::{InstallPackages, InstallPackagesError},
        },
        PackageInstallSpec,
    },
    package::{PackageName, PackageReq},
    project::{IntoLocalRockspecError, Project, ProjectError},
    remote_package_db::{RemotePackageDB, RemotePackageDBError},
    rockspec::Rockspec,
    tree::{self, InstallTree, TreeError},
};

#[derive(Debug, Error, Diagnostic)]
pub enum InstallProjectError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Project(#[from] ProjectError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LocalRockspec(#[from] IntoLocalRockspecError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    RemotePackageDB(#[from] RemotePackageDBError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Install(#[from] Box<InstallPackagesError>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Build(#[from] Box<BuildError>),
    #[error("package '{0}' was not installed")]
    PackageNotInstalled(PackageName),
    #[error("failed to hash the project's sources")]
    Hash(#[source] std::io::Error),
}

impl From<InstallPackagesError> for InstallProjectError {
    fn from(source: InstallPackagesError) -> Self {
        Self::Install(Box::new(source))
    }
}

impl From<BuildError> for InstallProjectError {
    fn from(source: BuildError) -> Self {
        Self::Build(Box::new(source))
    }
}

/// Installs a project into a [`Tree`].
/// Typically, you will want to use [`crate::operations::BuildWorkspace`].
/// Useful for installing a project and its dependencies outside of a workspace tree.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct InstallProject<'a, T>
where
    T: InstallTree,
{
    project: &'a Project,

    config: &'a Config,

    tree: &'a T,

    #[builder(default = BuildBehaviour::Force)]
    behaviour: BuildBehaviour,
}

impl<
        T: InstallTree + Sync + Send,
        State: install_project_builder::State + install_project_builder::IsComplete,
    > InstallProjectBuilder<'_, T, State>
{
    /// Builds the project's root package, installing its dependencies through
    /// the pipeline. Returns the installed root package.
    pub async fn build(self) -> Result<LockedPackage, InstallProjectError> {
        let args = self._build();
        let config = args.config;
        let project = args.project;
        let tree = args.tree;

        let rockspec = project.local_remote_rockspec()?;
        let name = rockspec.package().clone();

        let behaviour = if matches!(args.behaviour, BuildBehaviour::Ignore) {
            match tree.lockfile()?.entrypoint(&name) {
                Some(existing) => {
                    let source_hash = project
                        .root()
                        .hash()
                        .await
                        .map_err(InstallProjectError::Hash)?;
                    if existing.version() == rockspec.version()
                        && existing.hashes().source == source_hash
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
        let package_db = RemotePackageDB::from_config(config)
            .await?
            .with_local(vec![root]);

        let install_spec =
            PackageInstallSpec::new(PackageReq::from(name.clone()), tree::EntryType::Entrypoint)
                .build_behaviour(behaviour)
                .build();

        InstallPackages::new(config, tree)
            .package_db(package_db)
            .package(install_spec)
            .install()
            .await?
            .0
            .into_iter()
            .find(|package| package.name() == &name)
            .ok_or(InstallProjectError::PackageNotInstalled(name))
    }
}
