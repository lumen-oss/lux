use std::{collections::HashMap, io, sync::Arc};

use bon::Builder;
use itertools::Itertools;
use miette::Diagnostic;
use thiserror::Error;

use crate::{
    build::BuildBehaviour,
    config::Config,
    lockfile::{FlushLockfileError, LockedPackage, LockedPackageId, Lockfile, ReadWrite},
    lua_installation::LuaInstallationError,
    luarocks::luarocks_installation::{LuaRocksError, LuaRocksInstallError},
    package::{PackageName, PackageNameList},
    remote_package_db::{RemotePackageDB, RemotePackageDBError},
    tree::{self, InstallTree, TreeError},
    workspace::WorkspaceTreeError,
};

use super::{
    build::Build as PipelineBuild,
    discover::FindPackageFromLuarocks,
    download_sources_and_hash::DownloadSourcesAndHash,
    resolve::ResolvePackageDependencies,
};

use crate::operations::PackageInstallSpec;

/// Installs a set of [`PackageInstallSpec`]s into an install tree.
///
/// This is the composable core of [`Install`](crate::operations::Install): it resolves the
/// dependency graph, downloads and hashes sources, builds the packages (build dependencies
/// first) and records the result in the tree's lockfile.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct InstallPackages<'a, T>
where
    T: InstallTree + Clone + Send + Sync,
{
    #[builder(start_fn)]
    config: &'a Config,
    #[builder(start_fn)]
    tree: T,
    packages: Vec<PackageInstallSpec>,
    package_db: Option<RemotePackageDB>,
}

impl<T, State> InstallPackagesBuilder<'_, T, State>
where
    State: install_packages_builder::State + install_packages_builder::IsComplete,
    T: InstallTree + Clone + Send + Sync + 'static,
{
    pub async fn install(self) -> Result<Vec<LockedPackage>, InstallPackagesError> {
        let args = self._build();
        if args.packages.is_empty() {
            return Ok(Vec::new());
        }
        install_packages(args).await
    }
}

async fn install_packages<T>(
    install: InstallPackages<'_, T>,
) -> Result<Vec<LockedPackage>, InstallPackagesError>
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
        return Err(InstallPackagesError::DuplicateEntrypoints(
            PackageNameList::new(duplicate_entrypoints),
        ));
    }

    let packages = install.packages;
    let config = install.config;
    let tree = &install.tree;

    let lockfile = tree.lockfile()?;
    let build_tree = tree.build_tree(config)?;

    let entrypoint_specs = packages
        .iter()
        .filter(|spec| spec.entry_type == tree::EntryType::Entrypoint)
        .collect_vec();

    // Entrypoints already installed that conflict with another entrypoint. This enumerates both
    // packages that are planned to be removed (`--force`), as well as those which are
    // unintentionally causing conflicts.
    let conflicting_entrypoints: HashMap<PackageName, LockedPackage> = entrypoint_specs
        .iter()
        .filter_map(|spec| lockfile.entrypoint(spec.package.name()).cloned())
        .map(|package| (package.name().clone(), package))
        .collect();

    let unforced_conflicts = entrypoint_specs
        .iter()
        .filter(|spec| spec.build_behaviour != BuildBehaviour::Force)
        .filter_map(|spec| conflicting_entrypoints.get(spec.package.name()))
        .map(|existing| format!("{}@{}", existing.name(), existing.version()))
        .collect_vec();

    if !unforced_conflicts.is_empty() {
        return Err(InstallPackagesError::ConflictingEntrypoints(
            unforced_conflicts.join("\n"),
        ));
    }

    // Forced overwrites: remove the conflicting entrypoints from the tree.
    // NOTE: non-transactional. If an error occurs, this removes the conflicting package without
    // installing the substitute. Fix when transactions are implemented.
    let conflicting_entrypoints = conflicting_entrypoints.into_values().collect_vec();
    for package in &conflicting_entrypoints {
        tree.cleanup(package, tree::EntryType::Entrypoint)?;
    }

    let discover = FindPackageFromLuarocks::new(Arc::new(package_db), Arc::new(config.clone())).build();
    let resolved = ResolvePackageDependencies::new(&discover, config)
        .packages(packages)
        .resolve()
        .await
        .map_err(|err| InstallPackagesError::Pipeline(Box::new(err)))?;
    let artifacts = DownloadSourcesAndHash::new(config)
        .resolved(resolved)
        .download_sources_and_hash()
        .await
        .map_err(|err| InstallPackagesError::Pipeline(Box::new(err)))?;

    let build_packages = artifacts.build.into_values().collect_vec();
    let regular_packages = artifacts.regular.into_values().collect_vec();
    let regular_entry_types: HashMap<LockedPackageId, tree::EntryType> = regular_packages
        .iter()
        .map(|pkg| (pkg.package.spec.id(), pkg.entry_type))
        .collect();

    // Build dependencies first, then the packages themselves.
    let built_build_deps = PipelineBuild::new(config, &build_tree)
        .packages(build_packages)
        .build()
        .await
        .map_err(|err| InstallPackagesError::Pipeline(Box::new(err)))?;
    let built = PipelineBuild::new(config, tree)
        .packages(regular_packages)
        .build()
        .await
        .map_err(|err| InstallPackagesError::Pipeline(Box::new(err)))?;

    let installed_packages: HashMap<LockedPackageId, LockedPackage> =
        built.iter().map(|pkg| (pkg.spec.id(), pkg.clone())).collect();
    let installed_build_deps: HashMap<LockedPackageId, LockedPackage> = built_build_deps
        .iter()
        .map(|pkg| (pkg.spec.id(), pkg.clone()))
        .collect();

    lockfile.map_then_flush(|lockfile| {
        for package in &conflicting_entrypoints {
            lockfile.remove_by_id(&package.id());
        }
        for package in &built {
            let entry_type = regular_entry_types[&package.spec.id()];
            lockfile.add_dependencies(package, entry_type, &installed_packages)?;
            lockfile.add_build_dependencies(package, &installed_build_deps)?;
        }
        Ok::<_, io::Error>(())
    })?;

    Ok(built)
}

#[derive(Error, Debug, Diagnostic)]
pub enum InstallPackagesError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaVersionUnset(#[from] crate::lua_version::LuaVersionUnset),
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
    LuaRocksInstall(#[from] Box<LuaRocksInstallError>),
    #[error("error initialising remote package DB")]
    #[diagnostic(forward(0))]
    RemotePackageDB(#[from] RemotePackageDBError),
    #[error("cannot install duplicate entrypoints:\n{0}")]
    DuplicateEntrypoints(PackageNameList),
    #[error("cannot install conflicting entrypoints:\n{0}")]
    #[diagnostic(help(
        "only a single entrypoint per package is allowed.\nretry with `--force` to overwrite the existing entrypoint"
    ))]
    ConflictingEntrypoints(String),
    #[error("failed to install packages")]
    #[diagnostic(forward(0))]
    Pipeline(Box<dyn Diagnostic + Send + Sync + 'static>),
}

impl From<LuaRocksInstallError> for InstallPackagesError {
    fn from(source: LuaRocksInstallError) -> Self {
        Self::LuaRocksInstall(Box::new(source))
    }
}

trait LockfileExt {
    fn add_dependencies(
        self,
        pkg: &LockedPackage,
        entry_type: tree::EntryType,
        installed_packages: &HashMap<LockedPackageId, LockedPackage>,
    ) -> io::Result<()>;

    fn add_build_dependencies(
        self,
        pkg: &LockedPackage,
        build_dependencies: &HashMap<LockedPackageId, LockedPackage>,
    ) -> io::Result<()>;
}

impl LockfileExt for &mut Lockfile<ReadWrite> {
    fn add_dependencies(
        self,
        pkg: &LockedPackage,
        entry_type: tree::EntryType,
        installed_packages: &HashMap<LockedPackageId, LockedPackage>,
    ) -> io::Result<()> {
        if entry_type == tree::EntryType::Entrypoint {
            self.add_entrypoint(pkg);
        }

        for dependency_id in pkg.spec.dependencies() {
            self.add_dependency(
                pkg,
                installed_packages.get(dependency_id).ok_or(io::Error::other(
                    r#"
error writing dependencies to the lockfile.
A required dependency was not installed correctly.
This is likely a bug in Lux.

[THIS IS A BUG!]
"#,
                ))?,
            );
        }
        Ok(())
    }

    fn add_build_dependencies(
        self,
        pkg: &LockedPackage,
        build_dependencies: &HashMap<LockedPackageId, LockedPackage>,
    ) -> io::Result<()> {
        for dependency_id in pkg.spec.build_dependencies() {
            self.add_build_dependency(
                pkg,
                build_dependencies.get(dependency_id).ok_or(io::Error::other(
                    r#"
error writing build dependencies to the lockfile.
A required build dependency was not installed correctly.
This is likely a bug in Lux.

[THIS IS A BUG!]
"#,
                ))?,
            );
        }
        Ok(())
    }
}
