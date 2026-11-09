use std::io;

use crate::fs;
use crate::lockfile::{FlushLockfileError, LockedPackage, LockedPackageId};
use crate::tree::{self, EntryType, Installable};
use bon::Builder;
use itertools::Itertools;
use miette::Diagnostic;
use thiserror::Error;

#[derive(Error, Debug, Diagnostic)]
#[error(transparent)]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] tree::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    FlushLockfile(#[from] FlushLockfileError),
}

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Uninstall<'a, T>
where
    T: Installable,
{
    #[builder(field)]
    packages: Vec<LockedPackageId>,
    target: &'a T,
}

impl<'a, T, State> UninstallBuilder<'a, T, State>
where
    T: Installable,
    State: uninstall_builder::State,
{
    /// Add packages to remove.
    pub fn packages<I>(self, packages: I) -> Self
    where
        I: IntoIterator<Item = LockedPackageId>,
    {
        Self {
            packages: self.packages.into_iter().chain(packages).collect_vec(),
            ..self
        }
    }

    /// Add a package to the set of packages to remove.
    pub fn package(self, package: LockedPackageId) -> Self {
        self.packages(std::iter::once(package))
    }
}

impl<T, State> UninstallBuilder<'_, T, State>
where
    T: Installable,
    State: uninstall_builder::State + uninstall_builder::IsComplete,
{
    /// Remove the packages.
    pub async fn remove(self) -> Result<Vec<LockedPackageId>, Error> {
        let args = self._build();
        remove(args.packages, args.target)
    }
}

// TODO: Remove dependencies recursively too!
fn remove<T>(package_ids: Vec<LockedPackageId>, tree: &T) -> Result<Vec<LockedPackageId>, Error>
where
    T: Installable,
{
    let lockfile = tree.lockfile()?;

    let packages = package_ids
        .iter()
        .filter_map(|id| {
            lockfile
                .get(id)
                .map(|package| (package.clone(), lockfile.entry_type(id)))
        })
        .collect_vec();

    for (package, entry_type) in packages {
        remove_package(&package, tree, entry_type)?;
    }

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
fn remove_package<T>(package: &LockedPackage, tree: &T, entry_type: EntryType) -> Result<(), Error>
where
    T: Installable,
{
    tree.cleanup(package, entry_type)?;
    Ok(())
}
