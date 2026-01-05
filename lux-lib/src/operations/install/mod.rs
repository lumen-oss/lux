use std::{collections::HashMap, io, sync::Arc};

use crate::{
    build::{Build, BuildBehaviour, BuildError, RemotePackageSourceSpec, SrcRockSource},
    config::{Config, LuaVersionUnset},
    lockfile::{
        FlushLockfileError, LocalPackage, LocalPackageId, LockConstraint, Lockfile, OptState,
        PinnedState, ReadWrite,
    },
    lua_installation::{LuaInstallation, LuaInstallationError},
    lua_rockspec::BuildBackendSpec,
    luarocks::{
        install_binary_rock::{BinaryRockInstall, InstallBinaryRockError},
        luarocks_installation::{LuaRocksError, LuaRocksInstallError, LuaRocksInstallation},
    },
    operations::resolve::{Resolve, ResolveDependenciesError},
    package::{PackageName, PackageNameList},
    progress::{MultiProgress, Progress, ProgressBar},
    project::{Project, ProjectTreeError},
    remote_package_db::{PackageDB, RemotePackageDBError, RemotePackageDbIntegrityError},
    rockspec::Rockspec,
    tree::{self, Tree, TreeError},
};

pub use crate::operations::install::spec::PackageInstallSpec;

use bon::Builder;
use bytes::Bytes;
use futures::StreamExt;
use itertools::Itertools;
use thiserror::Error;

use super::{DownloadedRockspec, RemoteRockDownload};

pub mod spec;

/// A rocks package installer, providing fine-grained control
/// over how packages should be installed.
/// Can install multiple packages in parallel.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Install<'a> {
    #[builder(start_fn)]
    config: &'a Config,
    #[builder(field)]
    packages: Vec<PackageInstallSpec>,
    #[builder(setters(name = "_tree", vis = ""))]
    tree: Tree,
    package_db: Option<PackageDB>,
    progress: Option<Arc<Progress<MultiProgress>>>,
}

impl<'a, State> InstallBuilder<'a, State>
where
    State: install_builder::State,
{
    pub fn tree(self, tree: Tree) -> InstallBuilder<'a, install_builder::SetTree<State>>
    where
        State::Tree: install_builder::IsUnset,
    {
        self._tree(tree)
    }

    pub fn project(
        self,
        project: &'a Project,
    ) -> Result<InstallBuilder<'a, install_builder::SetTree<State>>, ProjectTreeError>
    where
        State::Tree: install_builder::IsUnset,
    {
        let config = self.config;
        Ok(self._tree(project.tree(config)?))
    }

    pub fn packages(self, packages: Vec<PackageInstallSpec>) -> Self {
        Self { packages, ..self }
    }

    pub fn package(self, package: PackageInstallSpec) -> Self {
        Self {
            packages: self
                .packages
                .into_iter()
                .chain(std::iter::once(package))
                .collect(),
            ..self
        }
    }
}

impl<State> InstallBuilder<'_, State>
where
    State: install_builder::State + install_builder::IsComplete,
{
    /// Install the packages.
    pub async fn install(self) -> Result<Vec<LocalPackage>, InstallError> {
        let install_built = self._build();
        if install_built.packages.is_empty() {
            return Ok(Vec::default());
        }
        let progress = match install_built.progress {
            Some(p) => p,
            None => MultiProgress::new_arc(install_built.config),
        };
        let package_db = match install_built.package_db {
            Some(db) => db,
            None => {
                let bar = progress.map(|p| p.new_bar());
                PackageDB::from_config(install_built.config, &bar).await?
            }
        };

        let duplicate_entrypoints = install_built
            .packages
            .iter()
            .filter(|pkg| pkg.entry_type == tree::EntryType::Entrypoint)
            .map(|pkg| pkg.package.name())
            .duplicates()
            .cloned()
            .collect_vec();

        if !duplicate_entrypoints.is_empty() {
            return Err(InstallError::DuplicateEntrypoints(PackageNameList::new(
                duplicate_entrypoints,
            )));
        }

        install_impl(
            install_built.packages,
            Arc::new(package_db),
            install_built.config,
            &install_built.tree,
            progress,
        )
        .await
    }
}

#[derive(Error, Debug)]
pub enum InstallError {
    #[error("unable to resolve dependencies:\n{0}")]
    ResolveDependencies(#[from] ResolveDependenciesError),
    #[error(transparent)]
    LuaVersionUnset(#[from] LuaVersionUnset),
    #[error(transparent)]
    LuaInstallation(#[from] LuaInstallationError),
    #[error(transparent)]
    FlushLockfile(#[from] FlushLockfileError),
    #[error(transparent)]
    Tree(#[from] TreeError),
    #[error("error instantiating LuaRocks compatibility layer:\n{0}")]
    LuaRocksError(#[from] LuaRocksError),
    #[error("error installing LuaRocks compatibility layer:\n{0}")]
    LuaRocksInstallError(#[from] LuaRocksInstallError),
    #[error("failed to build {0}: {1}")]
    BuildError(PackageName, BuildError),
    #[error("failed to install build depencency {0}:\n{1}")]
    BuildDependencyError(PackageName, BuildError),
    #[error("error initialising remote package DB:\n{0}")]
    RemotePackageDB(#[from] RemotePackageDBError),
    #[error("failed to install pre-built rock {0}:\n{1}")]
    InstallBinaryRockError(PackageName, InstallBinaryRockError),
    #[error("integrity error for package {0}:\n{1}")]
    Integrity(PackageName, RemotePackageDbIntegrityError),
    #[error(transparent)]
    ProjectTreeError(#[from] ProjectTreeError),
    #[error("cannot install duplicate entrypoints:\n{0}")]
    DuplicateEntrypoints(PackageNameList),
}

// TODO(vhyrro): This function has too many arguments. Refactor it.
#[allow(clippy::too_many_arguments)]
async fn install_impl(
    packages: Vec<PackageInstallSpec>,
    package_db: Arc<PackageDB>,
    config: &Config,
    tree: &Tree,
    progress_arc: Arc<Progress<MultiProgress>>,
) -> Result<Vec<LocalPackage>, InstallError> {
    let (dep_tx, mut dep_rx) = tokio::sync::mpsc::unbounded_channel();
    let (build_dep_tx, mut build_dep_rx) = tokio::sync::mpsc::unbounded_channel();

    let lockfile = tree.lockfile()?;
    let build_lockfile = tree.build_tree(config)?.lockfile()?;

    Resolve::new()
        .dependencies_tx(dep_tx)
        .build_dependencies_tx(build_dep_tx)
        .packages(packages)
        .package_db(package_db.clone())
        .lockfile(Arc::new(lockfile.clone()))
        .build_lockfile(Arc::new(build_lockfile.clone()))
        .config(config)
        .progress(progress_arc.clone())
        .get_all_dependencies()
        .await?;

    let lua = Arc::new(
        LuaInstallation::new_from_config(config, &progress_arc.map(|progress| progress.new_bar()))
            .await?,
    );

    // We have to install transitive build dependencies sequentially
    while let Some(build_dep_spec) = build_dep_rx.recv().await {
        let rockspec = build_dep_spec.downloaded_rock.rockspec();
        let bar = progress_arc.map(|p| {
            p.add(ProgressBar::from(format!(
                "💻 Installing build dependency: {}",
                build_dep_spec.downloaded_rock.rockspec().package(),
            )))
        });
        let package = rockspec.package().clone();
        let build_tree = tree.build_tree(config)?;
        // We have to write to the build tree's lockfile after each build,
        // so that each transitive build dependency is available for the
        // next build dependencies that may depend on it.
        let mut build_lockfile = build_tree.lockfile()?.write_guard();
        let pkg = Build::new()
            .rockspec(rockspec)
            .lua(&lua)
            .tree(&build_tree)
            .entry_type(tree::EntryType::Entrypoint)
            .config(config)
            .progress(&bar)
            .constraint(build_dep_spec.spec.constraint())
            .behaviour(build_dep_spec.build_behaviour)
            .build()
            .await
            .map_err(|err| InstallError::BuildDependencyError(package, err))?;
        build_lockfile.add_entrypoint(&pkg);
    }

    let mut all_packages = HashMap::with_capacity(dep_rx.len());
    while let Some(dep) = dep_rx.recv().await {
        all_packages.insert(dep.spec.id(), dep);
    }

    let installed_packages =
        futures::stream::iter(all_packages.clone().into_values().map(|install_spec| {
            let progress_arc = progress_arc.clone();
            let downloaded_rock = install_spec.downloaded_rock;
            let config = config.clone();
            let tree = tree.clone();
            let lua = lua.clone();

            tokio::spawn({
                async move {
                    let pkg = match downloaded_rock {
                        RemoteRockDownload::RockspecOnly { rockspec_download } => {
                            install_rockspec(
                                rockspec_download,
                                None,
                                install_spec.spec.constraint(),
                                install_spec.build_behaviour,
                                install_spec.pin,
                                install_spec.opt,
                                install_spec.entry_type,
                                &lua,
                                &tree,
                                &config,
                                progress_arc,
                            )
                            .await?
                        }
                        RemoteRockDownload::BinaryRock {
                            rockspec_download,
                            packed_rock,
                        } => {
                            install_binary_rock(
                                rockspec_download,
                                packed_rock,
                                install_spec.spec.constraint(),
                                install_spec.build_behaviour,
                                install_spec.pin,
                                install_spec.opt,
                                install_spec.entry_type,
                                &config,
                                &tree,
                                progress_arc,
                            )
                            .await?
                        }
                        RemoteRockDownload::SrcRock {
                            rockspec_download,
                            src_rock,
                            source_url,
                        } => {
                            let src_rock_source = SrcRockSource {
                                bytes: src_rock,
                                source_url,
                            };
                            install_rockspec(
                                rockspec_download,
                                Some(src_rock_source),
                                install_spec.spec.constraint(),
                                install_spec.build_behaviour,
                                install_spec.pin,
                                install_spec.opt,
                                install_spec.entry_type,
                                &lua,
                                &tree,
                                &config,
                                progress_arc,
                            )
                            .await?
                        }
                    };

                    Ok::<_, InstallError>((pkg.id(), (pkg, install_spec.entry_type)))
                }
            })
        }))
        .buffered(config.max_jobs())
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .flatten()
        .try_collect::<_, HashMap<LocalPackageId, (LocalPackage, tree::EntryType)>, _>()?;

    let write_dependency = |lockfile: &mut Lockfile<ReadWrite>,
                            id: &LocalPackageId,
                            pkg: &LocalPackage,
                            entry_type: tree::EntryType|
     -> io::Result<()> {
        if entry_type == tree::EntryType::Entrypoint {
            lockfile.add_entrypoint(pkg);
        }

        for dependency_id in all_packages
            .get(id)
            .map(|pkg| pkg.spec.dependencies())
            .unwrap_or_default()
            .into_iter()
        {
            lockfile.add_dependency(
                pkg,
                installed_packages
                    .get(dependency_id)
                    .map(|(pkg, _)| pkg)
                    .ok_or(io::Error::other(
                        r#"
error writing dependencies to the lockfile.
A required dependency was not installed correctly.
This is likely because an install thread panicked and was interrupted unexpectedly.

[THIS IS A BUG!]
"#,
                    ))?,
            );
        }
        Ok(())
    };

    lockfile.map_then_flush(|lockfile| {
        for (id, (pkg, is_entrypoint)) in installed_packages.iter() {
            write_dependency(lockfile, id, pkg, *is_entrypoint)?;
        }
        Ok::<_, io::Error>(())
    })?;

    Ok(installed_packages
        .into_values()
        .map(|(pkg, _)| pkg)
        .collect_vec())
}

#[allow(clippy::too_many_arguments)]
async fn install_rockspec(
    rockspec_download: DownloadedRockspec,
    src_rock_source: Option<SrcRockSource>,
    constraint: LockConstraint,
    behaviour: BuildBehaviour,
    pin: PinnedState,
    opt: OptState,
    entry_type: tree::EntryType,
    lua: &LuaInstallation,
    tree: &Tree,
    config: &Config,
    progress_arc: Arc<Progress<MultiProgress>>,
) -> Result<LocalPackage, InstallError> {
    let progress = Arc::clone(&progress_arc);
    let rockspec = rockspec_download.rockspec;
    let source = rockspec_download.source;
    let package = rockspec.package().clone();
    let bar = progress.map(|p| p.add(ProgressBar::from(format!("💻 Installing {}", &package,))));

    if let Some(BuildBackendSpec::LuaRock(_)) = &rockspec.build().current_platform().build_backend {
        let luarocks_tree = tree.build_tree(config)?;
        let luarocks = LuaRocksInstallation::new(config, luarocks_tree)?;
        luarocks.ensure_installed(lua, &bar).await?;
    }

    let source_spec = match src_rock_source {
        Some(src_rock_source) => RemotePackageSourceSpec::SrcRock(src_rock_source),
        None => RemotePackageSourceSpec::RockSpec(rockspec_download.source_url),
    };

    let pkg = Build::new()
        .rockspec(&rockspec)
        .lua(lua)
        .tree(tree)
        .entry_type(entry_type)
        .config(config)
        .progress(&bar)
        .pin(pin)
        .opt(opt)
        .constraint(constraint)
        .behaviour(behaviour)
        .source(source)
        .source_spec(source_spec)
        .build()
        .await
        .map_err(|err| InstallError::BuildError(package, err))?;

    bar.map(|b| b.finish_and_clear());

    Ok(pkg)
}

#[allow(clippy::too_many_arguments)]
async fn install_binary_rock(
    rockspec_download: DownloadedRockspec,
    packed_rock: Bytes,
    constraint: LockConstraint,
    behaviour: BuildBehaviour,
    pin: PinnedState,
    opt: OptState,
    entry_type: tree::EntryType,
    config: &Config,
    tree: &Tree,
    progress_arc: Arc<Progress<MultiProgress>>,
) -> Result<LocalPackage, InstallError> {
    let progress = Arc::clone(&progress_arc);
    let rockspec = rockspec_download.rockspec;
    let package = rockspec.package().clone();
    let bar = progress.map(|p| {
        p.add(ProgressBar::from(format!(
            "💻 Installing {} (pre-built)",
            &package,
        )))
    });
    let pkg = BinaryRockInstall::new(
        &rockspec,
        rockspec_download.source,
        packed_rock,
        entry_type,
        config,
        tree,
        &bar,
    )
    .pin(pin)
    .opt(opt)
    .constraint(constraint)
    .behaviour(behaviour)
    .install()
    .await
    .map_err(|err| InstallError::InstallBinaryRockError(package, err))?;

    bar.map(|b| b.finish_and_clear());

    Ok(pkg)
}
