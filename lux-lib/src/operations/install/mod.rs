use std::{
    collections::{HashMap, HashSet},
    io,
    sync::Arc,
};

use crate::{
    build::{Build, BuildBehaviour, BuildError, RemotePackageSourceSpec, SrcRockSource},
    config::Config,
    lockfile::{
        FlushLockfileError, LocalPackage, LocalPackageId, LockConstraint, Lockfile, OptState,
        PinnedState, ReadOnly, ReadWrite,
    },
    lua_installation::{LuaInstallation, LuaInstallationError},
    lua_rockspec::BuildBackendSpec,
    lua_version::LuaVersionUnset,
    luarocks::{
        install_binary_rock::{BinaryRockInstall, InstallBinaryRockError},
        luarocks_installation::{LuaRocksError, LuaRocksInstallError, LuaRocksInstallation},
    },
    operations::resolve::{
        build_dependency_names, PackageInstallData, Resolve, ResolveDependenciesError,
    },
    package::{PackageName, PackageNameList, PackageReq},
    remote_package_db::{RemotePackageDB, RemotePackageDBError, RemotePackageDbIntegrityError},
    rockspec::Rockspec,
    tree::{self, InstallTree, Tree, TreeError},
    workspace::{Workspace, WorkspaceTreeError},
};

pub use crate::operations::install::spec::PackageInstallSpec;

use super::{DownloadedRockspec, RemoteRockDownload};
use bon::Builder;
use bytes::Bytes;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use itertools::Itertools;
use miette::Diagnostic;
use thiserror::Error;

use tracing::{info_span, span, Instrument};
pub mod spec;

/// A rocks package installer, providing fine-grained control
/// over how packages should be installed.
/// Can install multiple packages in parallel.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Install<'a, T>
where
    T: InstallTree + Clone + Send + Sync,
{
    #[builder(start_fn)]
    config: &'a Config,
    #[builder(field)]
    packages: Vec<PackageInstallSpec>,
    #[builder(setters(name = "_tree", vis = ""))]
    tree: T,
    package_db: Option<RemotePackageDB>,
}

impl<'a, State> InstallBuilder<'a, Tree, State>
where
    State: install_builder::State,
{
    pub fn workspace(
        self,
        workspace: &'a Workspace,
    ) -> Result<InstallBuilder<'a, Tree, install_builder::SetTree<State>>, WorkspaceTreeError>
    where
        State::Tree: install_builder::IsUnset,
    {
        let config = self.config;
        Ok(self._tree(workspace.tree(config)?))
    }
}

impl<'a, T, State> InstallBuilder<'a, T, State>
where
    State: install_builder::State,
    T: InstallTree + Clone + Send + Sync,
{
    pub fn tree(self, tree: T) -> InstallBuilder<'a, T, install_builder::SetTree<State>>
    where
        State::Tree: install_builder::IsUnset,
    {
        self._tree(tree)
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

impl<State, T> InstallBuilder<'_, T, State>
where
    State: install_builder::State + install_builder::IsComplete,
    T: InstallTree + Clone + Send + Sync + 'static,
{
    /// Install the packages.
    pub async fn install(self) -> Result<Vec<LocalPackage>, InstallError> {
        let install_built = self._build();
        if install_built.packages.is_empty() {
            return Ok(Vec::default());
        }
        let count = install_built.packages.len();
        let span = if count > 1 {
            info_span!("Installing", count,)
        } else {
            let install_spec = &install_built.packages[0];
            info_span!("Installing", package = install_spec.package.to_string(),)
        };
        let _enter = span.enter();
        let package_db = match install_built.package_db {
            Some(db) => db,
            None => RemotePackageDB::from_config(install_built.config).await?,
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
        )
        .await
    }
}

type InstallWorkerOutput = Result<(LocalPackageId, (LocalPackage, tree::EntryType)), InstallError>;

#[derive(Error, Debug, Diagnostic)]
pub enum InstallError {
    #[error("unable to resolve dependencies:\n{0}")]
    #[diagnostic(forward(0))]
    ResolveDependencies(#[from] ResolveDependenciesError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaVersionUnset(#[from] LuaVersionUnset),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaInstallation(#[from] LuaInstallationError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    FlushLockfile(#[from] FlushLockfileError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    WorkspaceTree(#[from] WorkspaceTreeError),
    #[error("error instantiating LuaRocks compatibility layer:\n{0}")]
    #[diagnostic(forward(0))]
    LuaRocks(#[from] LuaRocksError),
    #[error("error installing LuaRocks compatibility layer:\n{0}")]
    #[diagnostic(forward(0))]
    LuaRocksInstall(#[from] LuaRocksInstallError),
    #[error("failed to build {0}: {1}")]
    Build(PackageName, BuildError),
    #[error("failed to install build depencency {0}:\n{1}")]
    BuildDependency(PackageName, BuildError),
    #[error("error initialising remote package DB:\n{0}")]
    #[diagnostic(forward(0))]
    RemotePackageDB(#[from] RemotePackageDBError),
    #[error("failed to install pre-built rock {0}:\n{1}")]
    InstallBinaryRock(PackageName, InstallBinaryRockError),
    #[error("integrity error for package '{package}'")]
    Integrity {
        package: PackageName,
        #[diagnostic_source]
        err: RemotePackageDbIntegrityError,
    },
    #[error("cannot install duplicate entrypoints:\n{0}")]
    DuplicateEntrypoints(PackageNameList),
    #[error("install worker panicked")]
    #[diagnostic(help(
        r#"this is a bug in Lux, please report it, ideally with `RUST_BACKTRACE=1`.
retrying with fewer parallel jobs (`--max-jobs`) may avoid the panic in the meantime"#
    ))]
    Join(#[from] tokio::task::JoinError),
}

// TODO(vhyrro): This function has too many arguments. Refactor it.
#[allow(clippy::too_many_arguments)]
async fn install_impl<T>(
    packages: Vec<PackageInstallSpec>,
    package_db: Arc<RemotePackageDB>,
    config: &Config,
    tree: &T,
) -> Result<Vec<LocalPackage>, InstallError>
where
    T: InstallTree + Clone + Send + Sync + 'static,
{
    let (dep_tx, mut dep_rx) = tokio::sync::mpsc::unbounded_channel();
    let (build_dep_tx, mut build_dep_rx) = tokio::sync::mpsc::unbounded_channel();
    let (build_dep_install_done_tx, mut build_dep_install_done_rx) =
        tokio::sync::mpsc::unbounded_channel::<PackageName>();

    let lockfile = tree.lockfile()?;
    let build_lockfile = tree.build_tree(config)?.lockfile()?;

    let lua = Arc::new(LuaInstallation::new_from_config(config).await?);

    let mut resolve = tokio::spawn({
        let config = config.clone();
        let lockfile = Arc::new(lockfile.clone());
        let build_lockfile = Arc::new(build_lockfile.clone());
        async move {
            Resolve::new()
                .dependencies_tx(dep_tx)
                .build_dependencies_tx(build_dep_tx)
                .packages(packages)
                .package_db(package_db)
                .lockfile(lockfile)
                .build_lockfile(build_lockfile)
                .config(&config)
                .get_all_dependencies()
                .await?;
            Ok::<(), InstallError>(())
        }
    })
    .instrument(tracing::trace_span!("resolve_worker"));

    // We have to install transitive build dependencies sequentially,
    // because a build dependency can itself depend on other build dependencies.
    let mut build_deps = tokio::spawn({
        let config = config.clone();
        let tree = tree.clone();
        let lua = lua.clone();
        async move {
            while let Some(build_dep_spec) = build_dep_rx.recv().await {
                let rockspec = build_dep_spec.downloaded_rock.rockspec();
                let package = rockspec.package().clone();
                let span = info_span!(
                    "Installing build dependency",
                    package = package.to_string(),
                    version = rockspec.version().to_string()
                );
                async {
                    let build_tree = tree.build_tree(&config)?;
                    // We have to write to the build tree's lockfile after each build,
                    // so that each transitive build dependency is available for the
                    // next build dependencies that may depend on it.
                    let mut build_lockfile = build_tree.lockfile()?.write_guard();
                    let pkg = Build::new()
                        .rockspec(rockspec)
                        .lua(&lua)
                        .tree(&build_tree)
                        .entry_type(tree::EntryType::Entrypoint)
                        .config(&config)
                        .constraint(build_dep_spec.spec.constraint())
                        .behaviour(build_dep_spec.build_behaviour)
                        .build()
                        .await
                        .map_err(|err| InstallError::BuildDependency(package.clone(), err))?;
                    build_lockfile.add_entrypoint(&pkg);
                    Ok::<_, InstallError>(())
                }
                .instrument(span)
                .await?;
                let _ = build_dep_install_done_tx.send(package);
            }
            Ok::<(), InstallError>(())
        }
    })
    .instrument(tracing::trace_span!("build_deps_worker"));

    let mut all_packages: HashMap<LocalPackageId, PackageInstallData> = HashMap::new();
    let mut scheduled: HashSet<LocalPackageId> = HashSet::new();
    let mut installed_packages: HashMap<LocalPackageId, (LocalPackage, tree::EntryType)> =
        HashMap::new();
    let mut installed_build_deps: HashSet<PackageName> = HashSet::new();
    let mut installs: FuturesUnordered<
        tracing::instrument::Instrumented<tokio::task::JoinHandle<InstallWorkerOutput>>,
    > = FuturesUnordered::new();
    let mut resolve_done = false;
    let mut build_deps_done = false;
    let mut dep_rx_drained = false;
    let mut build_dep_rx_drained = false;
    let mut error: Option<InstallError> = None;
    let max_jobs = config.max_jobs();

    'install: loop {
        if resolve_done && build_deps_done && dep_rx_drained && build_dep_rx_drained {
            break;
        }
        tokio::select! {
            resolve_result = &mut resolve, if !resolve_done => {
                match resolve_result {
                    Ok(Ok(_)) => resolve_done = true,
                    Ok(Err(err)) => {
                        error = Some(err);
                        break 'install;
                    }
                    Err(join) => {
                        error = Some(join.into());
                        break 'install;
                    }
                }
            }
            build_deps_result = &mut build_deps, if !build_deps_done => {
                match build_deps_result {
                    Ok(Ok(_)) => build_deps_done = true,
                    Ok(Err(err)) => {
                        error = Some(err);
                        break 'install;
                    }
                    Err(join) => {
                        error = Some(join.into());
                        break 'install;
                    }
                }
            }
            name = build_dep_install_done_rx.recv(), if !build_dep_rx_drained => {
                if let Some(name) = name {
                    installed_build_deps.insert(name);
                } else {
                    build_dep_rx_drained = true;
                }
            }
            dep = dep_rx.recv(), if !dep_rx_drained => {
                if let Some(dep) = dep {
                    all_packages.insert(dep.spec.id(), dep);
                } else {
                    dep_rx_drained = true;
                }
            }
        }

        // Schedule installs for packages whose build dependencies have all
        // been installed into the build tree.
        // NOTE: Binary rocks don't need their build dependencies installed.
        let ready: Vec<(LocalPackageId, PackageInstallData)> = all_packages
            .iter()
            .filter(|(id, data)| {
                !scheduled.contains(*id)
                    && match &data.downloaded_rock {
                        RemoteRockDownload::BinaryRock { .. } => true,
                        _ => build_dependencies_ready(
                            data.downloaded_rock.rockspec(),
                            data.build_behaviour,
                            &build_lockfile,
                            &installed_build_deps,
                        ),
                    }
            })
            .map(|(id, data)| (id.clone(), data.clone()))
            .collect();

        for (package_id, data) in ready {
            if max_jobs > 0 && installs.len() >= max_jobs {
                if let Some(result) = installs.next().await {
                    match result {
                        Ok(Ok(installed)) => {
                            installed_packages.insert(installed.0, installed.1);
                        }
                        Ok(Err(err)) => {
                            error = Some(err);
                            break 'install;
                        }
                        Err(join) => {
                            error = Some(join.into());
                            break 'install;
                        }
                    }
                }
            }
            scheduled.insert(package_id);
            let config = config.clone();
            let tree = tree.clone();
            let lua = lua.clone();
            installs.push(
                tokio::spawn({
                    async move {
                        let pkg = match data.downloaded_rock {
                            RemoteRockDownload::RockspecOnly { rockspec_download } => {
                                install_rockspec(
                                    rockspec_download,
                                    None,
                                    data.spec.constraint(),
                                    data.build_behaviour,
                                    data.pin,
                                    data.opt,
                                    data.entry_type,
                                    &lua,
                                    &tree,
                                    &config,
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
                                    data.spec.constraint(),
                                    data.build_behaviour,
                                    data.pin,
                                    data.opt,
                                    data.entry_type,
                                    &config,
                                    &tree,
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
                                    data.spec.constraint(),
                                    data.build_behaviour,
                                    data.pin,
                                    data.opt,
                                    data.entry_type,
                                    &lua,
                                    &tree,
                                    &config,
                                )
                                .await?
                            }
                        };

                        Ok::<_, InstallError>((pkg.id(), (pkg, data.entry_type)))
                    }
                })
                .instrument(tracing::trace_span!("install_worker")),
            );
        }
    }

    if let Some(err) = error {
        resolve.into_inner().abort();
        build_deps.into_inner().abort();
        for install in installs {
            install.into_inner().abort();
        }
        return Err(err);
    }

    while let Some(result) = installs.next().await {
        match result {
            Ok(Ok(installed)) => {
                installed_packages.insert(installed.0, installed.1);
            }
            Ok(Err(err)) => return Err(err),
            Err(join) => return Err(join.into()),
        }
    }

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

/// Whether all build dependencies of the given rockspec have been installed
/// into the build tree, so that the package can start building.
///
/// A build dependency is considered ready when it has been freshly installed
/// into the build tree, or when it was already present in the build lockfile
/// and satisfies the dependency constraint.
fn build_dependencies_ready(
    rockspec: &impl Rockspec,
    behaviour: BuildBehaviour,
    build_lockfile: &Lockfile<ReadOnly>,
    installed: &HashSet<PackageName>,
) -> bool {
    let build_deps = rockspec.build_dependencies().current_platform();
    build_dependency_names(rockspec).iter().all(|name| {
        installed.contains(name)
            || (behaviour != BuildBehaviour::Force
                && build_lockfile
                    .has_rock(
                        &build_deps
                            .iter()
                            .find(|dep| dep.name() == name)
                            .map(|dep| dep.package_req().clone())
                            .unwrap_or_else(|| PackageReq::from(name.clone())),
                        None,
                    )
                    .is_some())
    })
}

#[allow(clippy::too_many_arguments)]
async fn install_rockspec<T>(
    rockspec_download: DownloadedRockspec,
    src_rock_source: Option<SrcRockSource>,
    constraint: LockConstraint,
    behaviour: BuildBehaviour,
    pin: PinnedState,
    opt: OptState,
    entry_type: tree::EntryType,
    lua: &LuaInstallation,
    tree: &T,
    config: &Config,
) -> Result<LocalPackage, InstallError>
where
    T: InstallTree + Sync,
{
    let package = rockspec_download.rockspec.package().clone();
    let rockspec = rockspec_download.rockspec;
    let span = info_span!(
        "Installing",
        package = package.to_string(),
        version = rockspec.version().to_string(),
    );
    let _enter = span.enter();
    let source = rockspec_download.source;

    if let Some(BuildBackendSpec::LuaRock(_)) = &rockspec.build().current_platform().build_backend {
        let luarocks_tree = tree.build_tree(config)?;
        let luarocks = LuaRocksInstallation::new(config, luarocks_tree)?;
        luarocks.ensure_installed(lua).await?;
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
        .pin(pin)
        .opt(opt)
        .constraint(constraint)
        .behaviour(behaviour)
        .source(source)
        .source_spec(source_spec)
        .build()
        .await
        .map_err(|err| InstallError::Build(package, err))?;
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
    tree: &impl InstallTree,
) -> Result<LocalPackage, InstallError> {
    let rockspec = rockspec_download.rockspec;
    let package = rockspec.package().clone();
    let span = span!(
        tracing::Level::INFO,
        "Installing (pre-built)",
        package = package.to_string(),
        version = rockspec.version().to_string(),
    );
    let _enter = span.enter();
    let pkg = BinaryRockInstall::new(
        &rockspec,
        rockspec_download.source,
        packed_rock,
        entry_type,
        config,
        tree,
    )
    .pin(pin)
    .opt(opt)
    .constraint(constraint)
    .behaviour(behaviour)
    .install()
    .await
    .map_err(|err| InstallError::InstallBinaryRock(package, err))?;
    Ok(pkg)
}
