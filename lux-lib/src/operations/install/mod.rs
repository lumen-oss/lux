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
        build_dependencies_to_install, PackageInstallData, Resolve, ResolveDependenciesError,
    },
    package::{PackageName, PackageNameList, PackageReq},
    remote_package_db::{RemotePackageDB, RemotePackageDBError},
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
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::{JoinError, JoinHandle};

use tracing::Instrument;
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
            tracing::info_span!("Installing", count)
        } else {
            let install_spec = &install_built.packages[0];
            tracing::info_span!("Installing", package = install_spec.package.to_string())
        };
        install_impl(install_built).instrument(span).await
    }
}

type InstallWorkerOutput = Result<(LocalPackageId, (LocalPackage, tree::EntryType)), InstallError>;

#[derive(Error, Debug, Diagnostic)]
pub enum InstallError {
    #[error("unable to resolve dependencies")]
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
    #[error("error instantiating LuaRocks compatibility layer")]
    #[diagnostic(forward(0))]
    LuaRocks(#[from] LuaRocksError),
    #[error("error installing LuaRocks compatibility layer")]
    #[diagnostic(forward(0))]
    LuaRocksInstall(#[from] LuaRocksInstallError),
    #[error("failed to build {0}")]
    Build(PackageName, #[source] BuildError),
    #[error("failed to install build depencency {0}")]
    BuildDependency(PackageName, #[source] BuildError),
    #[error("error initialising remote package DB")]
    #[diagnostic(forward(0))]
    RemotePackageDB(#[from] RemotePackageDBError),
    #[error("failed to install pre-built rock {0}")]
    InstallBinaryRock(PackageName, #[source] InstallBinaryRockError),
    #[error("cannot install duplicate entrypoints:\n{0}")]
    DuplicateEntrypoints(PackageNameList),
    #[error("install worker panicked")]
    #[diagnostic(help(
        r#"this is a bug in Lux, please report it, ideally with `RUST_BACKTRACE=1`.
retrying with fewer parallel jobs (`--max-jobs`) may avoid the panic in the meantime"#
    ))]
    Join(#[from] tokio::task::JoinError),
}

async fn install_impl<T>(install: Install<'_, T>) -> Result<Vec<LocalPackage>, InstallError>
where
    T: InstallTree + Clone + Send + Sync + 'static,
{
    let package_db = match install.package_db {
        Some(db) => db,
        None => RemotePackageDB::from_config(install.config).await?,
    };

    let duplicate_entrypoints = install
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

    let packages = install.packages;
    let package_db = Arc::new(package_db);
    let config = install.config;
    let tree = &install.tree;

    let (dep_tx, mut dep_rx) = tokio::sync::mpsc::unbounded_channel();
    let (build_dep_tx, build_dep_rx) = tokio::sync::mpsc::unbounded_channel();
    let (build_dep_install_done_tx, mut build_dep_install_done_rx) =
        tokio::sync::mpsc::unbounded_channel::<PackageName>();

    let lockfile = tree.lockfile()?;
    let build_lockfile = tree.build_tree(config)?.lockfile()?;

    let lua = Arc::new(LuaInstallation::new_from_config(config).await?);

    let mut resolve_worker = spawn_resolve_worker(
        config,
        packages,
        package_db,
        lockfile.clone(),
        build_lockfile.clone(),
        dep_tx,
        build_dep_tx,
    );
    let mut build_deps_worker = spawn_build_deps_worker(
        config,
        tree,
        lua.clone(),
        build_dep_rx,
        build_dep_install_done_tx,
    );

    let mut all_packages: HashMap<LocalPackageId, PackageInstallData> = HashMap::new();
    let mut scheduled_packages: HashSet<LocalPackageId> = HashSet::new();
    let mut installed_packages: HashMap<LocalPackageId, (LocalPackage, tree::EntryType)> =
        HashMap::new();
    let mut installed_build_deps: HashSet<PackageName> = HashSet::new();
    let mut ongoing_installs: FuturesUnordered<
        tracing::instrument::Instrumented<tokio::task::JoinHandle<InstallWorkerOutput>>,
    > = FuturesUnordered::new();
    let mut resolve_done = false;
    let mut build_deps_done = false;
    let mut dep_rx_drained = false;
    let mut build_dep_rx_drained = false;
    let mut install_loop_result: Result<(), InstallError> = Ok(());
    let max_jobs = config.max_jobs();

    'install: loop {
        if resolve_done && build_deps_done && dep_rx_drained && build_dep_rx_drained {
            break;
        }
        tokio::select! {
            resolve_result = &mut resolve_worker, if !resolve_done => {
                if let Err(err) = worker_result(resolve_result) {
                    install_loop_result = Err(*err);
                    break 'install;
                }
                resolve_done = true;
            }
            build_deps_result = &mut build_deps_worker, if !build_deps_done => {
                if let Err(err) = worker_result(build_deps_result) {
                    install_loop_result = Err(*err);
                    break 'install;
                }
                build_deps_done = true;
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

        for (package_id, package_install_data) in ready_to_install(
            &all_packages,
            &scheduled_packages,
            &build_lockfile,
            &installed_build_deps,
        ) {
            if max_jobs > 0 && ongoing_installs.len() >= max_jobs {
                if let Err(err) =
                    wait_for_next_install(&mut ongoing_installs, &mut installed_packages).await
                {
                    install_loop_result = Err(err);
                    break 'install;
                }
            }
            scheduled_packages.insert(package_id);
            ongoing_installs.push(spawn_install_worker(
                package_install_data,
                &lua,
                tree,
                config,
            ));
        }
    }

    match install_loop_result {
        Ok(_) => {
            while wait_for_next_install(&mut ongoing_installs, &mut installed_packages).await? {}

            lockfile.map_then_flush(|lockfile| {
                for (package_id, (package, is_entrypoint)) in installed_packages.iter().unique() {
                    lockfile.add_dependencies(
                        package_id,
                        package,
                        *is_entrypoint,
                        &all_packages,
                        &installed_packages,
                    )?;
                }
                Ok::<_, io::Error>(())
            })?;

            Ok(installed_packages
                .into_values()
                .map(|(pkg, _)| pkg)
                .collect_vec())
        }
        Err(err) => {
            resolve_worker.into_inner().abort();
            build_deps_worker.into_inner().abort();
            for install in ongoing_installs {
                install.into_inner().abort();
            }
            Err(err)
        }
    }
}

fn spawn_resolve_worker(
    config: &Config,
    packages: Vec<PackageInstallSpec>,
    package_db: Arc<RemotePackageDB>,
    lockfile: Lockfile<ReadOnly>,
    build_lockfile: Lockfile<ReadOnly>,
    dep_tx: UnboundedSender<PackageInstallData>,
    build_dep_tx: UnboundedSender<PackageInstallData>,
) -> tracing::instrument::Instrumented<JoinHandle<Result<(), InstallError>>> {
    tokio::spawn({
        let config = config.clone();
        let lockfile = Arc::new(lockfile);
        let build_lockfile = Arc::new(build_lockfile);
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
    .instrument(tracing::trace_span!("resolve_worker"))
}

fn spawn_build_deps_worker<T>(
    config: &Config,
    tree: &T,
    lua: Arc<LuaInstallation>,
    mut build_dep_rx: UnboundedReceiver<PackageInstallData>,
    build_dep_install_done_tx: UnboundedSender<PackageName>,
) -> tracing::instrument::Instrumented<JoinHandle<Result<(), InstallError>>>
where
    T: InstallTree + Clone + Send + Sync + 'static,
{
    tokio::spawn({
        let config = config.clone();
        let tree = tree.clone();
        let lua = lua.clone();
        async move {
            while let Some(build_dep_spec) = build_dep_rx.recv().await {
                let rockspec = build_dep_spec.downloaded_rock.rockspec();
                let package = rockspec.package().clone();
                let span = tracing::info_span!(
                    "Installing build dependency",
                    package = package.to_string(),
                    version = rockspec.version().to_string()
                );
                async {
                    let build_tree = tree.build_tree(&config)?;
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
    .instrument(tracing::trace_span!("build_deps_worker"))
}

fn ready_to_install(
    all_packages: &HashMap<LocalPackageId, PackageInstallData>,
    scheduled: &HashSet<LocalPackageId>,
    build_lockfile: &Lockfile<ReadOnly>,
    installed_build_deps: &HashSet<PackageName>,
) -> Vec<(LocalPackageId, PackageInstallData)> {
    all_packages
        .iter()
        .filter(|(id, data)| {
            !scheduled.contains(*id)
                && match &data.downloaded_rock {
                    RemoteRockDownload::BinaryRock { .. } => true,
                    _ => build_dependencies_ready(
                        data.downloaded_rock.rockspec(),
                        data.build_behaviour,
                        build_lockfile,
                        installed_build_deps,
                    ),
                }
        })
        .map(|(id, data)| (id.clone(), data.clone()))
        .collect()
}

fn spawn_install_worker<T>(
    data: PackageInstallData,
    lua: &Arc<LuaInstallation>,
    tree: &T,
    config: &Config,
) -> tracing::instrument::Instrumented<JoinHandle<InstallWorkerOutput>>
where
    T: InstallTree + Clone + Send + Sync + 'static,
{
    let config = config.clone();
    let tree = tree.clone();
    let lua = lua.clone();
    let entry_type = data.entry_type;
    tokio::spawn(async move {
        let pkg = install_package(data, &lua, &tree, &config).await?;
        Ok::<_, InstallError>((pkg.id(), (pkg, entry_type)))
    })
    .instrument(tracing::trace_span!("install_worker"))
}

#[tracing::instrument(level = "trace", skip_all)]
async fn install_package<T>(
    data: PackageInstallData,
    lua: &Arc<LuaInstallation>,
    tree: &T,
    config: &Config,
) -> Result<LocalPackage, InstallError>
where
    T: InstallTree + Sync,
{
    match data.downloaded_rock {
        RemoteRockDownload::RockspecOnly { rockspec_download } => {
            install_rockspec(
                rockspec_download,
                None,
                data.spec.constraint(),
                data.build_behaviour,
                data.pin,
                data.opt,
                data.entry_type,
                lua,
                tree,
                config,
            )
            .await
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
                config,
                tree,
            )
            .await
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
                lua,
                tree,
                config,
            )
            .await
        }
    }
}

fn worker_result(
    result: Result<Result<(), InstallError>, JoinError>,
) -> Result<(), Box<InstallError>> {
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(err.into()),
        Err(join) => Err(InstallError::from(join).into()),
    }
}

async fn wait_for_next_install(
    ongoing_installs: &mut FuturesUnordered<
        tracing::instrument::Instrumented<tokio::task::JoinHandle<InstallWorkerOutput>>,
    >,
    installed_packages: &mut HashMap<LocalPackageId, (LocalPackage, tree::EntryType)>,
) -> Result<bool, InstallError> {
    if let Some(result) = ongoing_installs.next().await {
        match result {
            Ok(Ok((id, installed))) => {
                installed_packages.insert(id, installed);
                Ok(true)
            }
            Ok(Err(err)) => Err(err),
            Err(join) => Err(InstallError::from(join)),
        }
    } else {
        Ok(false)
    }
}
trait LockfileExt {
    fn add_dependencies(
        self,
        id: &LocalPackageId,
        pkg: &LocalPackage,
        entry_type: tree::EntryType,
        all_packages: &HashMap<LocalPackageId, PackageInstallData>,
        installed_packages: &HashMap<LocalPackageId, (LocalPackage, tree::EntryType)>,
    ) -> io::Result<()>;
}

impl LockfileExt for &mut Lockfile<ReadWrite> {
    fn add_dependencies(
        self,
        id: &LocalPackageId,
        pkg: &LocalPackage,
        entry_type: tree::EntryType,
        all_packages: &HashMap<LocalPackageId, PackageInstallData>,
        installed_packages: &HashMap<LocalPackageId, (LocalPackage, tree::EntryType)>,
    ) -> io::Result<()> {
        if entry_type == tree::EntryType::Entrypoint {
            self.add_entrypoint(pkg);
        }

        for dependency_id in all_packages
            .get(id)
            .map(|pkg| pkg.spec.dependencies())
            .unwrap_or_default()
            .into_iter()
        {
            self.add_dependency(
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
    }
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
    build_dependencies_to_install(rockspec).iter().all(|name| {
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
#[tracing::instrument(
    name = "Installing",
    level = "info",
    skip_all,
    fields(
        package = rockspec_download.rockspec.package().to_string(),
        version = rockspec_download.rockspec.version().to_string(),
    ),
)]
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
#[tracing::instrument(
    name = "Installing (pre-built)",
    level = "info",
    skip_all,
    fields(
        package = rockspec_download.rockspec.package().to_string(),
        version = rockspec_download.rockspec.version().to_string(),
    ),
)]
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
