use crate::package;
use bon::Builder;
use miette::Diagnostic;
use thiserror::Error;

use crate::{
    build::{self, BuildBehaviour},
    config::Config,
    drivers::install_packages::{self, InstallPackages},
    hash::HasIntegrity,
    lockfile::LockedPackage,
    operations::PackageInstallSpec,
    package_db::{self, PackageDB},
    pipeline::discover::FoundPackage,
    project::{self, IntoLocalRockspecError, Project},
    rockspec::Rockspec,
    tree::{self, InstallTree},
    workspace::{self, Workspace},
};

#[derive(Debug, Error, Diagnostic)]
pub enum InstallProjectError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Project(#[from] project::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LocalRockspec(#[from] IntoLocalRockspecError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    PackageDB(#[from] package_db::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] tree::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Install(#[from] Box<install_packages::Error>),
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

impl From<install_packages::Error> for InstallProjectError {
    fn from(source: install_packages::Error) -> Self {
        Self::Install(Box::new(source))
    }
}

impl From<build::Error> for InstallProjectError {
    fn from(source: build::Error) -> Self {
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

    /// Workspace whose lockfile is preferred during dependency resolution.
    workspace: Option<&'a Workspace>,
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
        let package_db = match args.workspace {
            Some(workspace) => PackageDB::open(config, workspace).await?,
            None => PackageDB::from_config(config).await?,
        }
        .with_local(vec![root]);

        let install_spec = PackageInstallSpec::new(
            package::Req::from(name.clone()),
            tree::EntryType::Entrypoint,
        )
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
