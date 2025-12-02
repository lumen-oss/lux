use std::{fmt::Display, sync::Arc};

use async_recursion::async_recursion;
use bon::Builder;
use futures::StreamExt;
use itertools::Itertools;
use thiserror::Error;
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    build::BuildBehaviour,
    config::Config,
    lockfile::{
        LocalPackageId, LocalPackageSpec, Lockfile, LockfilePermissions, OptState, PinnedState,
    },
    package::PackageName,
    progress::{MultiProgress, Progress},
    remote_package_db::RemotePackageDB,
    rockspec::Rockspec,
    tree,
};

use super::{Download, PackageInstallSpec, RemoteRockDownload, SearchAndDownloadError};

#[derive(Error, Debug)]
pub enum ResolveDependenciesError {
    #[error(transparent)]
    SearchAndDownload(#[from] SearchAndDownloadError),
    #[error("cyclic dependency detected:\n{0}")]
    CyclicDependency(DependencyCycle),
    #[error("error processing resolved dependency:\n{0}")]
    ChannelSend(String),
}

#[derive(Debug)]
pub struct DependencyCycle(Vec<PackageName>);

impl Display for DependencyCycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.iter().join(" -> ").fmt(f)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PackageInstallData {
    pub build_behaviour: BuildBehaviour,
    pub pin: PinnedState,
    pub opt: OptState,
    pub downloaded_rock: RemoteRockDownload,
    pub spec: LocalPackageSpec,
    pub entry_type: tree::EntryType,
}

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(crate) struct Resolve<'a, P>
where
    P: LockfilePermissions + Send + Sync + 'static,
{
    dependencies_tx: UnboundedSender<PackageInstallData>,
    build_dependencies_tx: UnboundedSender<PackageInstallData>,
    packages: Vec<PackageInstallSpec>,
    package_db: Arc<RemotePackageDB>,
    parent_packages: Option<Arc<Vec<PackageName>>>,
    lockfile: Arc<Lockfile<P>>,
    build_lockfile: Arc<Lockfile<P>>,
    config: &'a Config,
    progress: Arc<Progress<MultiProgress>>,
}

impl<P, State> ResolveBuilder<'_, P, State>
where
    P: LockfilePermissions + Send + Sync + 'static,
    State: resolve_builder::State + resolve_builder::IsComplete,
{
    pub(crate) async fn get_all_dependencies(
        self,
    ) -> Result<Vec<LocalPackageId>, ResolveDependenciesError> {
        let args = self._build();
        do_get_all_dependencies(args).await
    }
}

#[async_recursion]
async fn do_get_all_dependencies<'a, P>(
    args: Resolve<'a, P>,
) -> Result<Vec<LocalPackageId>, ResolveDependenciesError>
where
    'a: 'async_recursion,
    P: LockfilePermissions + Send + Sync + 'static,
{
    let dependencies_tx = args.dependencies_tx;
    let build_dependencies_tx = args.build_dependencies_tx;
    let packages = args.packages;
    let parent_packages = args.parent_packages.unwrap_or_default();
    let package_db = args.package_db;
    let lockfile = args.lockfile;
    let build_lockfile = args.build_lockfile;
    let config = args.config;
    let progress = args.progress;
    futures::stream::iter(
        packages
            .into_iter()
            // Exclude packages that are already installed
            .filter(
                |PackageInstallSpec {
                     package,
                     build_behaviour,
                     ..
                 }| {
                    *build_behaviour == BuildBehaviour::Force
                        || lockfile.has_rock(package, None).is_none()
                },
            )
            .map(
                // NOTE: we propagate build_behaviour, pin and opt to all dependencies
                |PackageInstallSpec {
                     package,
                     build_behaviour,
                     pin,
                     opt,
                     entry_type,
                     constraint,
                     source,
                 }| {
                    let config = config.clone();
                    let dependencies_tx = dependencies_tx.clone();
                    let build_dependencies_tx = build_dependencies_tx.clone();
                    let parent_packages = Arc::clone(&parent_packages);
                    let package_db = Arc::clone(&package_db);
                    let progress = Arc::clone(&progress);
                    let build_dep_progress = Arc::clone(&progress);
                    let lockfile = Arc::clone(&lockfile);
                    let build_lockfile = Arc::clone(&build_lockfile);

                    tokio::spawn(async move {
                        let bar = progress.map(|p| p.new_bar());

                        let downloaded_rock = if let Some(source) = source {
                            RemoteRockDownload::from_package_req_and_source_spec(
                                package.clone(),
                                source,
                            )?
                        } else {
                            Download::new(&package, &config, &bar)
                                .package_db(&package_db)
                                .download_remote_rock()
                                .await?
                        };

                        let constraint = constraint.unwrap_or(package.version_req().clone().into());

                        let rockspec = downloaded_rock.rockspec();

                        if parent_packages.contains(rockspec.package()) {
                            return Err(ResolveDependenciesError::CyclicDependency(
                                DependencyCycle(
                                    parent_packages
                                        .iter()
                                        .cloned()
                                        .chain(std::iter::once(rockspec.package().clone()))
                                        .collect_vec(),
                                ),
                            ));
                        }

                        // NOTE: We don't need to install build dependencies to install binary rocks.
                        if !matches!(downloaded_rock, RemoteRockDownload::BinaryRock { .. }) {
                            let build_dependencies = rockspec
                                .build_dependencies()
                                .current_platform()
                                .iter()
                                .filter(|dep| {
                                    // Exclude luarocks build backends that we have implemented in lux
                                    !matches!(
                                        dep.name().to_string().as_str(),
                                        "luarocks-build-rust-mlua"
                                            | "luarocks-build-treesitter-parser"
                                    )
                                })
                                .map(|dep| {
                                    // We always install build dependencies as entrypoints
                                    // with regard to the build tree
                                    let entry_type = tree::EntryType::Entrypoint;
                                    PackageInstallSpec::new(dep.package_req().clone(), entry_type)
                                        .build_behaviour(build_behaviour)
                                        .pin(pin)
                                        .opt(opt)
                                        .maybe_source(dep.source().clone())
                                        .build()
                                })
                                .collect_vec();

                            // NOTE: We treat transitive regular dependencies of build dependencies
                            // as build dependencies
                            Resolve::new()
                                .dependencies_tx(build_dependencies_tx.clone())
                                .build_dependencies_tx(build_dependencies_tx.clone())
                                .packages(build_dependencies)
                                .parent_packages(Arc::new(
                                    parent_packages
                                        .iter()
                                        .cloned()
                                        .chain(std::iter::once(rockspec.package().clone()))
                                        .collect_vec(),
                                ))
                                .package_db(package_db.clone())
                                .lockfile(build_lockfile.clone())
                                .build_lockfile(build_lockfile.clone())
                                .config(&config)
                                .progress(build_dep_progress)
                                .get_all_dependencies()
                                .await?;
                        }

                        let dependencies = rockspec
                            .dependencies()
                            .current_platform()
                            .iter()
                            .map(|dep| {
                                // If we're forcing a rebuild, retain the `EntryType`
                                // of existing dependencies
                                let entry_type = if build_behaviour == BuildBehaviour::Force
                                    && lockfile.has_rock(dep.package_req(), None).is_some_and(
                                        |installed_rock| {
                                            lockfile.is_entrypoint(&installed_rock.id())
                                        },
                                    ) {
                                    tree::EntryType::Entrypoint
                                } else {
                                    tree::EntryType::DependencyOnly
                                };

                                PackageInstallSpec::new(dep.package_req().clone(), entry_type)
                                    .build_behaviour(build_behaviour)
                                    .pin(pin)
                                    .opt(opt)
                                    .maybe_source(dep.source().clone())
                                    .build()
                            })
                            .collect_vec();

                        let dependencies = Resolve::new()
                            .dependencies_tx(dependencies_tx.clone())
                            .build_dependencies_tx(build_dependencies_tx)
                            .packages(dependencies)
                            .parent_packages(Arc::new(
                                parent_packages
                                    .iter()
                                    .cloned()
                                    .chain(std::iter::once(rockspec.package().clone()))
                                    .collect_vec(),
                            ))
                            .package_db(package_db)
                            .lockfile(lockfile)
                            .build_lockfile(build_lockfile)
                            .config(&config)
                            .progress(progress)
                            .get_all_dependencies()
                            .await?;

                        let rockspec = downloaded_rock.rockspec();
                        let local_spec = LocalPackageSpec::new(
                            rockspec.package(),
                            rockspec.version(),
                            constraint,
                            dependencies,
                            &pin,
                            &opt,
                            rockspec.binaries(),
                        );

                        let install_spec = PackageInstallData {
                            build_behaviour,
                            pin,
                            opt,
                            spec: local_spec.clone(),
                            downloaded_rock,
                            entry_type,
                        };

                        dependencies_tx.send(install_spec).map_err(|err| {
                            ResolveDependenciesError::ChannelSend(err.to_string())
                        })?;

                        Ok::<_, ResolveDependenciesError>(local_spec.id())
                    })
                },
            ),
    )
    .buffered(config.max_jobs())
    .collect::<Vec<_>>()
    .await
    .into_iter()
    .flatten()
    .try_collect()
}
