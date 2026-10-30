use std::collections::{HashMap, HashSet};

use bon::Builder;
use miette::Diagnostic;
use thiserror::Error;

use crate::rockspec;
use crate::{
    build::Deploy,
    config::Config,
    lockfile::{LockedPackage, LockedPackageId},
    lua,
    luarocks::{
        self,
        install_binary_rock::{self, BinaryRockInstall},
    },
    rockspec::Rockspec,
    tree::{self, InstallTree},
};

use super::materialise::{MaterialisedPackage, PackageSource};

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("failed to build '{0}'")]
    Build(String, #[source] Box<crate::build::Error>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] tree::Error),
    #[error("failed to install pre-built rock '{0}'")]
    InstallBinaryRock(String, #[source] Box<install_binary_rock::Error>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaInstallation(#[from] lua::installation::Error),
    #[error("error preparing the LuaRocks compatibility layer")]
    #[diagnostic(forward(0))]
    LuaRocks(#[from] Box<luarocks::installation::Error>),
}

impl From<luarocks::installation::Error> for Error {
    fn from(source: luarocks::installation::Error) -> Self {
        Self::LuaRocks(Box::new(source))
    }
}

/// A rocks package builder, providing fine-grained control
/// over how a package should be built.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(crate) struct Build<'a, T: InstallTree + Send + Sync> {
    #[builder(start_fn)]
    pub(crate) config: &'a Config,
    #[builder(start_fn)]
    pub(crate) tree: &'a T,
    #[builder(field)]
    pub(crate) packages: Vec<MaterialisedPackage>,
}

impl<T, State> BuildBuilder<'_, T, State>
where
    T: InstallTree + Send + Sync,
    State: build_builder::State,
{
    pub(crate) fn packages(mut self, packages: Vec<MaterialisedPackage>) -> Self {
        self.packages = packages;
        self
    }
}

impl<T, State> BuildBuilder<'_, T, State>
where
    T: InstallTree + Send + Sync,
    State: build_builder::State + build_builder::IsComplete,
{
    pub(crate) async fn build(self) -> Result<Vec<LockedPackage>, Error> {
        let args = self._build();
        let lua = lua::Installation::new_from_config(args.config).await?;

        let needs_luarocks = args.packages.iter().any(|package| {
            matches!(
                package.rockspec.build().current_platform().backend,
                Some(rockspec::build::backend::Spec::LuaRock(_))
            )
        });
        if needs_luarocks {
            luarocks::Installation::new(args.config, args.tree.build_tree(args.config)?)?
                .ensure_installed(&lua)
                .await?;
        }

        let mut installed = Vec::new();

        for downloaded in order_by_build_dependencies(args.packages) {
            let MaterialisedPackage {
                package,
                rockspec,
                entry_type,
                build_behaviour: package_behaviour,
                artifact,
                temp_dir: _temp_dir,
            } = downloaded;

            let is_entrypoint = entry_type.is_entrypoint();

            let pkg = match artifact {
                PackageSource::SourceDir(path) => Deploy::new(&rockspec, args.tree)
                    .package(package)
                    .lua(&lua)
                    .source_root(&path)
                    .entry_type(entry_type)
                    .config(args.config)
                    .behaviour(package_behaviour)
                    .deploy()
                    .await
                    .map_err(|err| {
                        Error::Build(
                            format!("{}-{}", rockspec.package(), rockspec.version()),
                            Box::new(err),
                        )
                    })?,
                PackageSource::Archive(bytes) => BinaryRockInstall::new(
                    &rockspec,
                    package.spec().clone(),
                    bytes,
                    package.source().clone(),
                    entry_type,
                    args.config,
                    args.tree,
                )
                .behaviour(package_behaviour)
                .install()
                .await
                .map_err(|err| {
                    Error::InstallBinaryRock(rockspec.package().to_string(), Box::new(err))
                })?,
            };

            // Record the installed package in the tree lockfile so that its paths are visible to
            // the packages built after it (e.g. LuaRocks build backends), as well as future builds.
            {
                let mut lockfile = args.tree.lockfile()?.write_guard();
                if is_entrypoint {
                    lockfile.add_entrypoint(&pkg);
                } else {
                    lockfile.add(&pkg);
                }
            }

            installed.push(pkg);
        }

        Ok(installed)
    }
}

fn order_by_build_dependencies(packages: Vec<MaterialisedPackage>) -> Vec<MaterialisedPackage> {
    fn visit(
        id: LockedPackageId,
        by_id: &mut HashMap<LockedPackageId, MaterialisedPackage>,
        visited: &mut HashSet<LockedPackageId>,
        ordered: &mut Vec<MaterialisedPackage>,
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

    let mut by_id: HashMap<LockedPackageId, MaterialisedPackage> = packages
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
