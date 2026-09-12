use std::io;

use crate::fs;
use crate::lockfile::{FlushLockfileError, LocalPackage, LocalPackageId};
use crate::lua_version::{LuaVersion, LuaVersionUnset};
use crate::tree::{EntryType, InstallTree, TreeError};
use crate::{config::Config, tree::Tree};
use bon::Builder;
use futures::StreamExt;
use itertools::Itertools;
use miette::Diagnostic;
use thiserror::Error;
use tracing::Instrument;
#[derive(Error, Debug, Diagnostic)]
#[error(transparent)]
pub enum RemoveError {
    #[diagnostic(transparent)]
    LuaVersionUnset(#[from] LuaVersionUnset),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::FsError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    FlushLockfile(#[from] FlushLockfileError),
}

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Uninstall<'a> {
    #[builder(field)]
    packages: Vec<LocalPackageId>,
    config: &'a Config,
    tree: Option<Tree>,
}

impl<'a, State> UninstallBuilder<'a, State>
where
    State: uninstall_builder::State,
{
    /// Add packages to remove.
    pub fn packages<I>(self, packages: I) -> Self
    where
        I: IntoIterator<Item = LocalPackageId>,
    {
        Self {
            packages: self.packages.into_iter().chain(packages).collect_vec(),
            ..self
        }
    }

    /// Add a package to the set of packages to remove.
    pub fn package(self, package: LocalPackageId) -> Self {
        self.packages(std::iter::once(package))
    }
}

impl<'a, State> UninstallBuilder<'a, State>
where
    State: uninstall_builder::State + uninstall_builder::IsComplete,
{
    /// Remove the packages.
    pub async fn remove(self) -> Result<Vec<LocalPackageId>, RemoveError> {
        let args = self._build();
        let tree = args.tree.unwrap_or(
            args.config
                .user_tree(LuaVersion::from(args.config)?.clone())?,
        );
        remove(args.packages, tree, args.config).await
    }
}

// TODO: Remove dependencies recursively too!
async fn remove(
    package_ids: Vec<LocalPackageId>,
    tree: Tree,
    config: &Config,
) -> Result<Vec<LocalPackageId>, RemoveError> {
    let lockfile = tree.lockfile()?;

    let packages = package_ids
        .iter()
        .filter_map(|id| {
            lockfile
                .get(id)
                .map(|package| (package.clone(), lockfile.entry_type(id)))
        })
        .collect_vec();

    futures::stream::iter(packages.into_iter().map(|(package, entry_type)| {
        let tree = tree.clone();
        tokio::spawn(
            remove_package(package, tree, entry_type)
                .instrument(tracing::trace_span!("remove_worker")),
        )
    }))
    .buffered(config.max_jobs())
    .collect::<Vec<_>>()
    .await;

    lockfile.map_then_flush(|lockfile| {
        package_ids
            .iter()
            .for_each(|package| lockfile.remove_by_id(package));

        Ok::<_, io::Error>(())
    })?;

    Ok(package_ids)
}

#[tracing::instrument(
    name = "Removing",
    level = "info",
    skip_all,
    fields(
        package = package.name().to_string(),
        version = package.version().to_string(),
    ),
)]
async fn remove_package(
    package: LocalPackage,
    tree: Tree,
    entry_type: EntryType,
) -> Result<(), RemoveError> {
    tree.cleanup(&package, entry_type)?;
    Ok(())
}
