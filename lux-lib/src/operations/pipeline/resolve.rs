use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use async_recursion::async_recursion;
use bon::Builder;
use miette::Diagnostic;
use thiserror::Error;

use crate::{
    config::Config, lockfile::{LockedPackage, LockedPackageHashes, LockedPackageId, LockedPackageSpec, RemotePackageSourceUrl}, lua_rockspec::RemoteLuaRockspec, operations::{PackageInstallSpec, RemoteRockDownload, resolve::build_dependencies_to_install}, package::{PackageName, PackageReq, RemotePackage}, remote_package_source::RemotePackageSource, rockspec::Rockspec, tree::EntryType,
};

use super::{
    discover::{FindPackageFromProvider, DiscoverError, FoundPackage, FoundPackageType},
    download_sources_and_hash::PackageSource,
};

pub(crate) struct ResolvedPackage {
    pub(crate) spec: LockedPackageSpec,
    pub(crate) rockspec: RemoteLuaRockspec,
    pub(crate) source: RemotePackageSource,
    pub(crate) source_url: Option<RemotePackageSourceUrl>,
    pub(crate) entry_type: EntryType,
    pub(crate) artifact: Option<PackageSource>,
}

impl ResolvedPackage {
    pub(crate) fn id(&self) -> LockedPackageId {
        self.spec.id()
    }

    pub(crate) fn with_hashes(self, hashes: LockedPackageHashes) -> LockedPackage {
        LockedPackage {
            spec: self.spec,
            source: self.source,
            source_url: self.source_url,
            hashes,
        }
    }
}

#[derive(Default)]
pub(crate) struct ResolvedArtifacts {
    pub(crate) regular: HashMap<LockedPackageId, ResolvedPackage>,
    pub(crate) build: HashMap<LockedPackageId, ResolvedPackage>,
    pub(crate) test: HashMap<LockedPackageId, ResolvedPackage>,
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub(crate) enum ResolveError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Discover(#[from] DiscoverError),
    #[error("cyclic dependency detected:\n{0}")]
    CyclicDependency(String),
    #[error("discovery returned a precomputed closure, which this resolver does not support")]
    UnsupportedClosure,
    #[error(transparent)]
    Join(#[from] tokio::task::JoinError),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ResolveStateType {
    Regular,
    Build,
    Test,
}

#[derive(Default)]
struct ResolveState {
    regular: HashMap<LockedPackageId, ResolvedPackage>,
    build: HashMap<LockedPackageId, ResolvedPackage>,
    test: HashMap<LockedPackageId, ResolvedPackage>,
}

impl ResolveState {
    fn select(&mut self, state_type: ResolveStateType) -> &mut HashMap<LockedPackageId, ResolvedPackage> {
        match state_type {
            ResolveStateType::Regular => &mut self.regular,
            ResolveStateType::Build => &mut self.build,
            ResolveStateType::Test => &mut self.test,
        }
    }

    fn contains(&self, state_type: ResolveStateType, id: &LockedPackageId) -> bool {
        match state_type {
            ResolveStateType::Regular => self.regular.contains_key(id),
            ResolveStateType::Build => self.build.contains_key(id),
            ResolveStateType::Test => self.test.contains_key(id),
        }
    }
}

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(crate) struct ResolvePackageDependencies<'a, D: FindPackageFromProvider> {
    #[builder(start_fn)]
    pub(crate) discover: &'a D,
    #[builder(start_fn)]
    pub(crate) config: &'a Config,
    #[builder(field)]
    pub(crate) packages: Vec<PackageInstallSpec>,
    #[builder(field)]
    pub(crate) build_packages: Vec<PackageInstallSpec>,
    #[builder(field)]
    pub(crate) test_packages: Vec<PackageInstallSpec>,
}

impl<D, State> ResolvePackageDependenciesBuilder<'_, D, State>
where
    D: FindPackageFromProvider,
    State: resolve_package_dependencies_builder::State,
{
    pub(crate) fn packages(mut self, packages: Vec<PackageInstallSpec>) -> Self {
        self.packages = packages;
        self
    }

    pub(crate) fn package(mut self, package: PackageInstallSpec) -> Self {
        self.packages.push(package);
        self
    }

    pub(crate) fn build_packages(mut self, packages: Vec<PackageInstallSpec>) -> Self {
        self.build_packages = packages;
        self
    }

    pub(crate) fn test_packages(mut self, packages: Vec<PackageInstallSpec>) -> Self {
        self.test_packages = packages;
        self
    }

    pub(crate) fn test_package(mut self, package: PackageInstallSpec) -> Self {
        self.test_packages.push(package);
        self
    }
}

impl<D, State> ResolvePackageDependenciesBuilder<'_, D, State>
where
    D: FindPackageFromProvider,
    State: resolve_package_dependencies_builder::State + resolve_package_dependencies_builder::IsComplete,
{
    pub(crate) async fn resolve(self) -> Result<ResolvedArtifacts, ResolveError> {
        let args = self._build();
        // TODO(vhyrro): Rewrite to be parallel and no RefCell
        let state = Rc::new(RefCell::new(ResolveState::default()));

        for package in args.packages {
            resolve_spec(
                package,
                ResolveStateType::Regular,
                Vec::new(),
                state.clone(),
                args.discover,
                args.config,
            )
            .await?;
        }

        for package in args.build_packages {
            resolve_spec(
                package,
                ResolveStateType::Build,
                Vec::new(),
                state.clone(),
                args.discover,
                args.config,
            )
            .await?;
        }

        for package in args.test_packages {
            resolve_spec(
                package,
                ResolveStateType::Test,
                Vec::new(),
                state.clone(),
                args.discover,
                args.config,
            )
            .await?;
        }

        let mut state = state.borrow_mut();
        Ok(ResolvedArtifacts {
            regular: std::mem::take(&mut state.regular),
            build: std::mem::take(&mut state.build),
            test: std::mem::take(&mut state.test),
        })
    }
}

#[async_recursion(?Send)]
async fn resolve_spec<D: FindPackageFromProvider>(
    spec: PackageInstallSpec,
    section: ResolveStateType,
    parents: Vec<PackageName>,
    state: Rc<RefCell<ResolveState>>,
    discover: &D,
    config: &Config,
) -> Result<LockedPackageId, ResolveError> {
    let FoundPackage {
        package,
        rockspec,
        artifact,
    } = find_package_from_provider(&spec, discover, config).await?;

    let package_spec = package.package.clone();
    if parents.contains(package_spec.name()) {
        let chain = parents
            .iter()
            .map(|name| name.to_string())
            .chain(std::iter::once(package_spec.name().to_string()))
            .collect::<Vec<_>>()
            .join(" -> ");
        return Err(ResolveError::CyclicDependency(chain));
    }

    let constraint = spec
        .constraint
        .clone()
        .unwrap_or_else(|| spec.package.version_req().clone().into());
    let id = LockedPackageId::new(
        package_spec.name(),
        package_spec.version(),
        spec.pin,
        spec.opt,
        constraint.clone(),
    );

    if state.borrow().contains(section, &id) {
        return Ok(id);
    }

    let mut child_parents = parents;
    child_parents.push(package_spec.name().clone());

    let is_binary = matches!(package.source, RemotePackageSource::LuarocksBinaryRock(_));

    let build_dependencies = if is_binary {
        Vec::new()
    } else {
        let mut ids = Vec::new();
        for dependency in build_dependency_specs(&rockspec, &spec) {
            ids.push(
                resolve_spec(
                    dependency,
                    ResolveStateType::Build,
                    child_parents.clone(),
                    state.clone(),
                    discover,
                    config,
                )
                .await?,
            );
        }
        ids
    };

    let mut dependencies = Vec::new();
    for dependency in rockspec.dependencies().current_platform() {
        let dependency_spec =
            PackageInstallSpec::new(dependency.package_req().clone(), EntryType::DependencyOnly)
                .build_behaviour(spec.build_behaviour)
                .pin(spec.pin)
                .opt(spec.opt)
                .maybe_source(dependency.source().clone())
                .build();
        dependencies.push(
            resolve_spec(
                dependency_spec,
                section,
                child_parents.clone(),
                state.clone(),
                discover,
                config,
            )
            .await?,
        );
    }

    let locked_spec = LockedPackageSpec::new(
        package_spec.name(),
        package_spec.version(),
        constraint,
        dependencies,
        build_dependencies,
        &spec.pin,
        &spec.opt,
        rockspec.binaries(),
    );

    let resolved = ResolvedPackage {
        spec: locked_spec,
        rockspec,
        source: package.source,
        source_url: package.source_url,
        entry_type: spec.entry_type,
        artifact,
    };

    state
        .borrow_mut()
        .select(section)
        .insert(id.clone(), resolved);

    Ok(id)
}

async fn find_package_from_provider<D: FindPackageFromProvider>(
    spec: &PackageInstallSpec,
    discover: &D,
    _config: &Config,
) -> Result<FoundPackage, ResolveError> {
    if let Some(source) = &spec.source {
        let download = RemoteRockDownload::from_package_req_and_source_spec(
            spec.package.clone(),
            source.clone(),
        )
        .map_err(|err| DiscoverError::Download(spec.package.clone(), Box::new(err)))?;
        let rockspec = download.rockspec().clone();
        let package = RemotePackage::new(
            spec.package.clone().try_into().map_err(|err| {
                DiscoverError::Download(
                    spec.package.clone(),
                    Box::new(crate::operations::SearchAndDownloadError::from(err)),
                )
            })?,
            download.rockspec_download().source.clone(),
            download.rockspec_download().source_url.clone(),
        );
        return Ok(FoundPackage {
            package,
            rockspec,
            artifact: None,
        });
    }

    // Run discovery on a separate task so that the deep download/HTTP future stack is not nested
    // inside this recursive resolver's call chain.
    let discover = discover.clone();
    let package = spec.package.clone();
    match tokio::spawn(async move { discover.find(&package).await }).await?? {
        FoundPackageType::Package(package) => Ok(package),
        FoundPackageType::Closure { .. } => Err(ResolveError::UnsupportedClosure),
    }
}

fn build_dependency_specs(
    rockspec: &RemoteLuaRockspec,
    parent: &PackageInstallSpec,
) -> Vec<PackageInstallSpec> {
    build_dependencies_to_install(rockspec)
        .into_iter()
        .filter_map(|name| {
            rockspec
                .build_dependencies()
                .current_platform()
                .iter()
                .find(|dep| dep.name() == &name)
                .map(|dep| {
                    PackageInstallSpec::new(dep.package_req().clone(), EntryType::Entrypoint)
                        .build_behaviour(parent.build_behaviour)
                        .pin(parent.pin)
                        .opt(parent.opt)
                        .maybe_source(dep.source().clone())
                        .build()
                })
                .or_else(|| {
                    Some(
                        PackageInstallSpec::new(PackageReq::from(name), EntryType::Entrypoint)
                            .build_behaviour(parent.build_behaviour)
                            .pin(parent.pin)
                            .opt(parent.opt)
                            .build(),
                    )
                })
        })
        .collect()
}
