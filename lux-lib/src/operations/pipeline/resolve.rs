use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use async_recursion::async_recursion;
use bon::Builder;
use futures::{StreamExt, TryStreamExt};
use itertools::Itertools;
use miette::Diagnostic;
use thiserror::Error;

use crate::{
    config::Config, lockfile::{LockedPackageId, LockedPackageLockType, LockedPackageSpec, RemotePackageSourceUrl}, lua_rockspec::{BuildBackendSpec, RemoteLuaRockspec}, operations::{PackageInstallSpec, RemoteRockDownload}, package::{PackageName, PackageReq, RemotePackage}, remote_package_source::RemotePackageSource, rockspec::Rockspec, tree::EntryType,
};

use super::{
    discover::{FindPackageFromProvider, DiscoverError, FoundPackage, FoundPackageType},
    download_sources_and_hash::PackageSource,
    Artifacts,
};

/// The build dependencies of a rockspec that still need to be installed, with the
/// LuaRocks build backend (if any) first.
pub(crate) fn build_dependencies_to_install<R: Rockspec>(rockspec: &R) -> Vec<PackageName> {
    let mut names = rockspec
        .build_dependencies()
        .current_platform()
        .iter()
        .filter(|dep| {
            !matches!(
                dep.name().to_string().as_str(),
                "luarocks-build-rust-mlua"
                    | "luarocks-build-rust-binary"
                    | "luarocks-build-treesitter-parser"
            )
        })
        .map(|dep| dep.name().clone())
        .collect_vec();

    if let Some(backend) = luarocks_build_backend_name(rockspec) {
        names.insert(0, backend);
    }
    names
}

/// The name of the luarocks build backend rock required to build this rockspec (if any).
pub(crate) fn luarocks_build_backend_name<R: Rockspec>(rockspec: &R) -> Option<PackageName> {
    match &rockspec.build().current_platform().build_backend {
        Some(BuildBackendSpec::LuaRock(backend)) => {
            Some(PackageName::new(format!("luarocks-build-{backend}")))
        }
        _ => None,
    }
}

pub(crate) struct ResolvedPackage {
    pub(crate) spec: LockedPackageSpec,
    pub(crate) rockspec: RemoteLuaRockspec,
    pub(crate) source: RemotePackageSource,
    pub(crate) source_url: Option<RemotePackageSourceUrl>,
    pub(crate) entry_type: EntryType,
    pub(crate) artifact: Option<PackageSource>,
}

pub(crate) type ResolvedArtifacts = Artifacts<HashMap<LockedPackageId, ResolvedPackage>>;

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub(crate) enum ResolveError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Discover(#[from] DiscoverError),
    #[error("cyclic dependency detected:\n{0}")]
    CyclicDependency(String),
    #[error(transparent)]
    Join(#[from] tokio::task::JoinError),
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

    pub(crate) fn build_packages(mut self, packages: Vec<PackageInstallSpec>) -> Self {
        self.build_packages = packages;
        self
    }

    pub(crate) fn test_packages(mut self, packages: Vec<PackageInstallSpec>) -> Self {
        self.test_packages = packages;
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
        let state = Rc::new(RefCell::new(ResolvedArtifacts::default()));
        let (discover, config) = (args.discover, args.config);

        let packages = Artifacts {
            regular: args.packages,
            build: args.build_packages,
            test: args.test_packages,
        };

        futures::stream::iter(packages)
            .then(|(section, packages)| {
                let state = state.clone();
                async move {
                    futures::stream::iter(packages)
                        .then(|package| {
                            resolve_spec(
                                package,
                                section,
                                Vec::new(),
                                state.clone(),
                                discover,
                                config,
                            )
                        })
                        .try_for_each(|_| async { Ok(()) })
                        .await
                }
            })
            .try_for_each(|_| async { Ok(()) })
            .await?;

        let resolved = std::mem::take(&mut *state.borrow_mut());
        Ok(resolved)
    }
}

#[async_recursion(?Send)]
async fn resolve_spec<D: FindPackageFromProvider>(
    spec: PackageInstallSpec,
    section: LockedPackageLockType,
    parents: Vec<PackageName>,
    state: Rc<RefCell<ResolvedArtifacts>>,
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

    if state.borrow().get(section).contains_key(&id) {
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
                    LockedPackageLockType::Build,
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
        .get_mut(section)
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
    let FoundPackageType::Package(package) =
        tokio::spawn(async move { discover.find(&package).await }).await??;
    Ok(package)
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
