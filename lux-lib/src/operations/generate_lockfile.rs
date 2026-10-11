use bon::Builder;
use miette::Diagnostic;
use thiserror::Error;

use crate::{
    config::Config,
    drivers::sync::{gather_dependencies, DependencyKind, SyncError},
    package_db::{PackageDB, PackageDBError},
    pipeline::{
        emit_lockfile::LockfileHandle,
        materialise::{Error as MaterialiseError, Materialise},
        resolve::{ResolveError, ResolvePackageDependencies},
    },
    workspace::{Workspace, WorkspaceError},
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
    pub async fn generate(self) -> Result<(), GenerateLockfileError> {
        generate_lockfile(self._build()).await
    }
}

async fn generate_lockfile(args: GenerateLockfile<'_>) -> Result<(), GenerateLockfileError> {
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
pub enum GenerateLockfileError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    PackageDB(#[from] PackageDBError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Sync(Box<SyncError>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Resolve(#[from] ResolveError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Materialise(#[from] MaterialiseError),
}

impl From<SyncError> for GenerateLockfileError {
    fn from(source: SyncError) -> Self {
        Self::Sync(Box::new(source))
    }
}
