use std::io;
use std::sync::Arc;

use crate::config::{LuaVersion, LuaVersionUnset};
use crate::lockfile::{FlushLockfileError, LocalPackage, LocalPackageId};
use crate::progress::{MultiProgress, Progress, ProgressBar};
use crate::tree::TreeError;
use crate::{config::Config, tree::Tree};
use bon::Builder;
use futures::StreamExt;
use itertools::Itertools;
use thiserror::Error;

#[derive(Error, Debug)]
#[error(transparent)]
pub enum RemoveError {
    LuaVersionUnset(#[from] LuaVersionUnset),
    Io(#[from] io::Error),
    #[error(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    FlushLockfile(#[from] FlushLockfileError),
}

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Uninstall<'a> {
    #[builder(field)]
    packages: Vec<LocalPackageId>,
    config: &'a Config,
    progress: Option<Arc<Progress<MultiProgress>>>,
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
    pub async fn remove(self) -> Result<(), RemoveError> {
        let args = self._build();
        let progress = match args.progress {
            Some(p) => p,
            None => MultiProgress::new_arc(args.config),
        };
        let tree = args.tree.unwrap_or(
            args.config
                .user_tree(LuaVersion::from(args.config)?.clone())?,
        );
        remove(args.packages, tree, args.config, &Arc::clone(&progress)).await
    }
}

// TODO: Remove dependencies recursively too!
async fn remove(
    package_ids: Vec<LocalPackageId>,
    tree: Tree,
    config: &Config,
    progress: &Progress<MultiProgress>,
) -> Result<(), RemoveError> {
    let lockfile = tree.lockfile()?;

    let packages = package_ids
        .iter()
        .filter_map(|id| lockfile.get(id))
        .cloned()
        .collect_vec();

    futures::stream::iter(packages.into_iter().map(|package| {
        let bar = progress.map(|p| p.new_bar());

        let tree = tree.clone();
        tokio::spawn(remove_package(package, tree, bar))
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

    Ok(())
}

async fn remove_package(
    package: LocalPackage,
    tree: Tree,
    bar: Progress<ProgressBar>,
) -> Result<(), RemoveError> {
    bar.map(|p| {
        p.set_message(format!(
            "🗑️ Removing {}@{}",
            package.name(),
            package.version()
        ))
    });

    let rock_layout = tree.installed_rock_layout(&package)?;
    tokio::fs::remove_dir_all(&rock_layout.etc).await?;
    tokio::fs::remove_dir_all(&rock_layout.rock_path).await?;

    // Delete the corresponding binaries attached to the current package (located under `{LUX_TREE}/bin/`)
    for relative_binary_path in package.spec.binaries() {
        if let Some(binary_file_name) = relative_binary_path.file_name() {
            let binary_path = tree.bin().join(binary_file_name);
            if binary_path.is_file() {
                tokio::fs::remove_file(binary_path).await?;
            }

            let unwrapped_binary_path = tree.unwrapped_bin().join(binary_file_name);
            if unwrapped_binary_path.is_file() {
                tokio::fs::remove_file(unwrapped_binary_path).await?;
            }
        }
    }

    bar.map(|p| p.finish_and_clear());
    Ok(())
}
