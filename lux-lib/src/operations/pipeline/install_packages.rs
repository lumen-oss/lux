use std::{collections::HashMap, io};

use bon::Builder;
use itertools::Itertools;
use miette::Diagnostic;
use thiserror::Error;
use tracing::Instrument;

use crate::{
    build::BuildBehaviour,
    config::Config,
    lockfile::{FlushLockfileError, LockedPackage, LockedPackageId, Lockfile, ReadWrite},
    lua_installation::LuaInstallationError,
    luarocks::luarocks_installation::{LuaRocksError, LuaRocksInstallError},
    package::PackageNameList,
    remote_package_db::{RemotePackageDB, RemotePackageDBError},
    tree::{self, InstallTree, TreeError},
    workspace::WorkspaceTreeError,
};

use super::{
    build::{Build as PipelineBuild, BuildError as PipelineBuildError},
    download_sources_and_hash::{DownloadSourcesAndHash, DownloadSourcesAndHashError},
    emit_lockfile::LockfileHandle,
    resolve::{ResolveError, ResolvePackageDependencies},
};

use crate::operations::PackageInstallSpec;

/// Installs a set of [`PackageInstallSpec`]s into a tree.
///
/// Returns a [`LockfileHandle`] so changes can be committed into
/// the correct lockfile.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct InstallPackages<'a, T>
where
    T: InstallTree + Send + Sync,
{
    #[builder(start_fn)]
    config: &'a Config,
    #[builder(start_fn)]
    tree: &'a T,
    #[builder(field)]
    build_packages: Vec<PackageInstallSpec>,
    #[builder(field)]
    test_packages: Option<Vec<PackageInstallSpec>>,
    #[builder(field)]
    packages: Vec<PackageInstallSpec>,
    package_db: Option<RemotePackageDB>,
}

impl<'a, T, State> InstallPackagesBuilder<'a, T, State>
where
    T: InstallTree + Send + Sync,
    State: install_packages_builder::State,
{
    pub fn packages(mut self, packages: Vec<PackageInstallSpec>) -> Self {
        self.packages = packages;
        self
    }

    pub fn package(mut self, package: PackageInstallSpec) -> Self {
        self.packages.push(package);
        self
    }

    pub fn build_packages(mut self, packages: Vec<PackageInstallSpec>) -> Self {
        self.build_packages = packages;
        self
    }

    pub fn test_packages(mut self, packages: Vec<PackageInstallSpec>) -> Self {
        self.test_packages = Some(packages);
        self
    }
}

impl<T, State> InstallPackagesBuilder<'_, T, State>
where
    State: install_packages_builder::State + install_packages_builder::IsComplete,
    T: InstallTree + Send + Sync,
{
    pub async fn install(
        self,
    ) -> Result<(Vec<LockedPackage>, LockfileHandle), InstallPackagesError> {
        let args = self._build();
        if args.packages.is_empty()
            && args.build_packages.is_empty()
            && args.test_packages.is_none()
        {
            return Ok((Vec::new(), LockfileHandle::default()));
        }
        let span = match args.packages.as_slice() {
            [install_spec] => {
                tracing::info_span!("Installing", package = install_spec.package.to_string())
            }
            packages => tracing::info_span!("Installing", count = packages.len()),
        };
        install_packages(args).instrument(span).await
    }
}

async fn install_packages<T>(
    install: InstallPackages<'_, T>,
) -> Result<(Vec<LockedPackage>, LockfileHandle), InstallPackagesError>
where
    T: InstallTree + Send + Sync,
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
    let tree = install.tree;

    let lockfile = tree.lockfile()?;
    let build_tree = tree.build_tree(config)?;

    let mut resolve = ResolvePackageDependencies::new(package_db, config)
        .packages(packages)
        .build_packages(install.build_packages);
    if let Some(test_packages) = install.test_packages {
        resolve = resolve.test_packages(test_packages);
    }
    let resolved = resolve.resolve().await?;
    let artifacts = DownloadSourcesAndHash::new(config)
        .resolved(resolved)
        .download_sources_and_hash()
        .await?;
    let handle = LockfileHandle::from_artifacts(&artifacts);

    let build_packages = artifacts
        .build
        .unwrap_or_default()
        .into_values()
        .collect_vec();
    let regular_packages = artifacts
        .regular
        .unwrap_or_default()
        .into_values()
        .collect_vec();

    // Entrypoints that are already installed. The requested build behaviour decides how to
    // react to them.
    let mut conflicting_entrypoints: HashMap<LockedPackageId, LockedPackage> = HashMap::new();
    let mut unforced_conflicts = Vec::new();
    for package in regular_packages
        .iter()
        .filter(|package| package.entry_type == tree::EntryType::Entrypoint)
    {
        let Some(existing) = lockfile.entrypoint(package.package.name()) else {
            continue;
        };
        match package.build_behaviour {
            // Forced installs replace the existing entrypoint.
            BuildBehaviour::Force => {
                conflicting_entrypoints.insert(existing.id(), existing.clone());
            }
            // `Ignore` tolerates re-installing the exact same version.
            BuildBehaviour::Ignore if existing.version() == package.package.version() => {}
            // Any other existing entrypoint is a conflict.
            BuildBehaviour::Ignore | BuildBehaviour::Conflict => {
                unforced_conflicts.push(format!("{}@{}", existing.name(), existing.version()));
            }
        }
    }

    if !unforced_conflicts.is_empty() {
        return Err(InstallPackagesError::ConflictingEntrypoints(
            unforced_conflicts.join("\n"),
        ));
    }

    // FIXME(vhyrro): non-transactional. If an error occurs this removes the conflicting package
    // without installing the substitute. Implement transactions at some point.
    let conflicting_entrypoints = conflicting_entrypoints.into_values().collect_vec();
    for package in &conflicting_entrypoints {
        tree.cleanup(package, tree::EntryType::Entrypoint)?;
    }

    let regular_entry_types: HashMap<LockedPackageId, tree::EntryType> = regular_packages
        .iter()
        .map(|pkg| (pkg.package.spec.id(), pkg.entry_type))
        .collect();

    // Build dependencies first, then the packages themselves.
    let built_build_deps = PipelineBuild::new(config, &build_tree)
        .packages(build_packages)
        .build()
        .await?;
    let built = PipelineBuild::new(config, tree)
        .packages(regular_packages)
        .build()
        .await?;
    if let Some(test_packages) = artifacts.test {
        let test_tree = tree.test_tree(config)?;
        PipelineBuild::new(config, &test_tree)
            .packages(test_packages.into_values().collect_vec())
            .build()
            .await?;
    }

    let installed_packages: HashMap<LockedPackageId, LockedPackage> = built
        .iter()
        .map(|pkg| (pkg.spec().id(), pkg.clone()))
        .collect();
    let installed_build_deps: HashMap<LockedPackageId, LockedPackage> = built_build_deps
        .iter()
        .map(|pkg| (pkg.spec().id(), pkg.clone()))
        .collect();

    lockfile.map_then_flush(|lockfile| {
        for package in &conflicting_entrypoints {
            lockfile.remove_by_id(&package.id());
        }
        for package in &built {
            let entry_type = regular_entry_types[&package.spec().id()];
            lockfile.add_dependencies(package, entry_type, &installed_packages)?;
            lockfile.add_build_dependencies(package, &installed_build_deps)?;
        }
        Ok::<_, io::Error>(())
    })?;

    Ok((built, handle))
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
    #[error(transparent)]
    #[diagnostic(transparent)]
    Resolve(#[from] ResolveError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Download(#[from] DownloadSourcesAndHashError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Build(#[from] PipelineBuildError),
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

        for dependency_id in pkg.spec().dependencies() {
            self.add_dependency(
                pkg,
                installed_packages
                    .get(dependency_id)
                    .ok_or(io::Error::other(
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
        for dependency_id in pkg.spec().build_dependencies() {
            self.add_build_dependency(
                pkg,
                build_dependencies
                    .get(dependency_id)
                    .ok_or(io::Error::other(
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
