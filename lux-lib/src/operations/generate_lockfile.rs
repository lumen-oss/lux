use bon::Builder;
use miette::Diagnostic;
use thiserror::Error;

use crate::{
    config::Config,
    drivers::sync::{self, gather_dependencies, DependencyKind},
    package::db::{self, PackageDB},
    pipeline::{
        emit_lockfile::LockfileHandle,
        materialise::{Error as MaterialiseError, Materialise},
        resolve::{self, ResolvePackageDependencies},
    },
    workspace::{self, Workspace},
};

/// Generates a workspace's lockfile without installing anything.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct GenerateLockfile<'a> {
    #[builder(start_fn)]
    config: &'a Config,
    #[builder(start_fn)]
    workspace: &'a Workspace,
}

impl<State> GenerateLockfileBuilder<'_, State>
where
    State: generate_lockfile_builder::State + generate_lockfile_builder::IsComplete,
{
    pub async fn generate(self) -> Result<(), Error> {
        generate_lockfile(self._build()).await
    }
}

async fn generate_lockfile(args: GenerateLockfile<'_>) -> Result<(), Error> {
    let GenerateLockfile { config, workspace } = args;

    let members = workspace.member_names()?;
    let package_db = PackageDB::open(config, workspace).await?;

    let resolve = ResolvePackageDependencies::new(package_db, config)
        .packages(gather_dependencies(
            workspace,
            DependencyKind::Regular,
            &members,
        )?)
        .build_packages(gather_dependencies(
            workspace,
            DependencyKind::Build,
            &members,
        )?)
        .test_packages(gather_dependencies(
            workspace,
            DependencyKind::Test,
            &members,
        )?);

    let resolved = resolve.resolve().await?;
    let artifacts = Materialise::new(config)
        .resolved(resolved)
        .materialise()
        .await?;

    let handle = LockfileHandle::from_artifacts(&artifacts);

    let mut lockfile = workspace.lockfile()?.write_guard();
    handle.commit(&mut lockfile);

    Ok(())
}

#[derive(Error, Debug, Diagnostic)]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    PackageDB(#[from] db::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Workspace(#[from] workspace::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Sync(Box<sync::Error>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Resolve(#[from] resolve::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Materialise(#[from] MaterialiseError),
}

impl From<sync::Error> for Error {
    fn from(source: sync::Error) -> Self {
        Self::Sync(Box::new(source))
    }
}
