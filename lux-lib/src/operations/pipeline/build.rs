use std::collections::{HashMap, HashSet};

use bon::Builder;
use miette::Diagnostic;
use thiserror::Error;

use crate::{
    build::{BuildBehaviour, deploy},
    config::Config,
    lockfile::{LockedPackage, LockedPackageId},
    lua_installation::{LuaInstallation, LuaInstallationError},
    lua_rockspec::BuildBackendSpec,
    luarocks::{
        install_binary_rock::{BinaryRockInstall, InstallBinaryRockError},
        luarocks_installation::{LuaRocksError, LuaRocksInstallError, LuaRocksInstallation},
    },
    rockspec::Rockspec,
    tree::{InstallTree, TreeError},
};

use super::download_sources_and_hash::{DownloadedPackage, PackageSource};

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum BuildError {
    #[error("failed to build '{0}'")]
    Build(String, #[source] Box<crate::build::BuildError>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] TreeError),
    #[error("failed to install pre-built rock '{0}'")]
    InstallBinaryRock(String, #[source] Box<InstallBinaryRockError>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaInstallation(#[from] LuaInstallationError),
    #[error("error instantiating LuaRocks compatibility layer")]
    #[diagnostic(forward(0))]
    LuaRocks(#[from] LuaRocksError),
    #[error("error installing LuaRocks compatibility layer")]
    #[diagnostic(forward(0))]
    LuaRocksInstall(#[from] Box<LuaRocksInstallError>),
}

impl From<LuaRocksInstallError> for BuildError {
    fn from(source: LuaRocksInstallError) -> Self {
        Self::LuaRocksInstall(Box::new(source))
    }
}

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(crate) struct Build<'a, T: InstallTree + Clone + Send + Sync> {
    #[builder(start_fn)]
    pub(crate) config: &'a Config,
    #[builder(start_fn)]
    pub(crate) tree: &'a T,
    #[builder(field)]
    pub(crate) packages: Vec<DownloadedPackage>,
    pub(crate) behaviour: Option<BuildBehaviour>,
}

impl<T, State> BuildBuilder<'_, T, State>
where
    T: InstallTree + Clone + Send + Sync,
    State: build_builder::State,
{
    pub(crate) fn packages(mut self, packages: Vec<DownloadedPackage>) -> Self {
        self.packages = packages;
        self
    }
}

impl<T, State> BuildBuilder<'_, T, State>
where
    T: InstallTree + Clone + Send + Sync,
    State: build_builder::State + build_builder::IsComplete,
{
    // INVESTIGATE(vhyrro): Is there a benefit of having a `LocalPackage` type which
    // contains a LockedPackage as well as Layout information and installation path?
    // LockedPackages are emitted by DownloadSourcesAndHash, and they represent packages ready for
    // the lockfile but not installed to the tree yet. Either we create an intermediate type
    // or a `LocalPackage` type.
    pub(crate) async fn build(self) -> Result<Vec<LockedPackage>, BuildError> {
        let args = self._build();
        let behaviour = args.behaviour.unwrap_or_default();
        let lua = LuaInstallation::new_from_config(args.config).await?;

        let needs_luarocks = args.packages.iter().any(|package| {
            matches!(
                package.rockspec.build().current_platform().build_backend,
                Some(BuildBackendSpec::LuaRock(_))
            )
        });
        if needs_luarocks {
            LuaRocksInstallation::new(args.config, args.tree.build_tree(args.config)?)?
                .ensure_installed(&lua)
                .await?;
        }

        let mut installed = Vec::new();

        for downloaded in order_by_build_dependencies(args.packages) {
            let DownloadedPackage {
                package,
                rockspec,
                entry_type,
                artifact,
            } = downloaded;

            let is_entrypoint = entry_type.is_entrypoint();
            let constraint = package.spec.constraint();
            let pin = package.spec.pinned();
            let opt = package.spec.opt();

            let pkg = match artifact {
                PackageSource::SourceTree(dir) => deploy(
                    &rockspec,
                    args.tree,
                    package,
                    &lua,
                    dir.path(),
                    entry_type,
                    args.config,
                    behaviour,
                )
                .await
                .map_err(|err| BuildError::Build(rockspec.package().to_string(), Box::new(err)))?,
                PackageSource::PackedRock(bytes) => BinaryRockInstall::new(
                    &rockspec,
                    package.source().clone(),
                    bytes,
                    entry_type,
                    args.config,
                    args.tree,
                )
                .pin(pin)
                .opt(opt)
                .constraint(constraint)
                .behaviour(behaviour)
                .install()
                .await
                .map_err(|err| {
                    BuildError::InstallBinaryRock(rockspec.package().to_string(), Box::new(err))
                })?,
            };

            // Record the installed package in the tree lockfile so that its paths are visible to
            // the packages built after it (e.g. LuaRocks build backends), as well as future builds.
            let mut lockfile = args.tree.lockfile()?.write_guard();
            if is_entrypoint {
                lockfile.add_entrypoint(&pkg);
            } else {
                lockfile.add(&pkg);
            }
            drop(lockfile);

            installed.push(pkg);
        }

        Ok(installed)
    }
}

fn order_by_build_dependencies(packages: Vec<DownloadedPackage>) -> Vec<DownloadedPackage> {
    let mut by_id: HashMap<LockedPackageId, DownloadedPackage> = packages
        .into_iter()
        .map(|package| (package.package.id(), package))
        .collect();

    let mut ordered = Vec::new();
    let mut visited = HashSet::new();

    let ids: Vec<LockedPackageId> = by_id.keys().cloned().collect();
    for id in ids {
        visit(id, &mut by_id, &mut visited, &mut ordered);
    }

    ordered
}

fn visit(
    id: LockedPackageId,
    by_id: &mut HashMap<LockedPackageId, DownloadedPackage>,
    visited: &mut HashSet<LockedPackageId>,
    ordered: &mut Vec<DownloadedPackage>,
) {
    if !visited.insert(id.clone()) {
        return;
    }

    let build_dependencies: Vec<LockedPackageId> = by_id
        .get(&id)
        .map(|package| {
            package
                .package
                .spec
                .build_dependencies()
                .into_iter()
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    for dependency in build_dependencies {
        visit(dependency, by_id, visited, ordered);
    }

    if let Some(package) = by_id.remove(&id) {
        ordered.push(package);
    }
}
