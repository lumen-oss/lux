use crate::package;
use std::{collections::HashMap, io};

use bon::Builder;
use itertools::Itertools;
use miette::Diagnostic;
use thiserror::Error;
use tracing::Instrument;

use crate::{
    build::BuildBehaviour,
    config::Config,
    lockfile::{
        FlushLockfileError, LockedPackage, LockedPackageId, LockedPackageLockType, Lockfile,
        ReadOnly, ReadWrite,
    },
    lua,
    luarocks::installation,
    package::db::PackageDB,
    tree::{self, InstallSession},
    workspace::TreeError,
};

use crate::pipeline::{
    build::{Build as PipelineBuild, Error as PipelineBuildError},
    emit_lockfile::LockfileHandle,
    materialise::{Error as MaterialiseError, Materialise, MaterialisedPackage},
    resolve::{self, ResolvePackageDependencies, ResolvedArtifacts},
};

use crate::operations::install;

/// Token of proof that build dependencies have been installed into a build tree.
///
/// Returned by [`InstallBuildDependencies`] and required by
/// [`InstallPackages`] and [`InstallTestPackages`].
pub struct BuildToken {
    build_lockfile: Lockfile<ReadOnly>,
}

/// Installs a set of regular [`install::Spec`]s into a tree.
///
/// Returns a [`LockfileHandle`] so changes can be committed into
/// the correct lockfile.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct InstallPackages<'a, T>
where
    T: InstallSession,
{
    #[builder(start_fn)]
    config: &'a Config,
    #[builder(start_fn)]
    tree: &'a T,
    #[builder(field)]
    packages: Vec<install::Spec>,
    package_db: PackageDB,
}

impl<'a, T, State> InstallPackagesBuilder<'a, T, State>
where
    T: InstallSession,
    State: install_packages_builder::State,
{
    pub fn packages(mut self, packages: Vec<install::Spec>) -> Self {
        self.packages = packages;
        self
    }

    pub fn package(mut self, package: install::Spec) -> Self {
        self.packages.push(package);
        self
    }
}

impl<T, State> InstallPackagesBuilder<'_, T, State>
where
    State: install_packages_builder::State + install_packages_builder::IsComplete,
    T: InstallSession,
{
    pub async fn install(
        self,
        build_token: &BuildToken,
    ) -> Result<(Vec<LockedPackage>, LockfileHandle), Error> {
        let args = self._build();
        if args.packages.is_empty() {
            return Ok((Vec::new(), LockfileHandle::default()));
        }
        let span = match args.packages.as_slice() {
            [install_spec] => {
                tracing::info_span!("Installing", package = install_spec.package.to_string())
            }
            packages => tracing::info_span!("Installing", count = packages.len()),
        };
        install_regular_packages(args, build_token)
            .instrument(span)
            .await
    }
}

/// Installs a set of test [`install::Spec`]s into a test tree.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct InstallTestPackages<'a, T>
where
    T: InstallSession,
{
    #[builder(start_fn)]
    config: &'a Config,
    #[builder(start_fn)]
    tree: &'a T,
    #[builder(field)]
    packages: Vec<install::Spec>,
    package_db: PackageDB,
}

impl<'a, T, State> InstallTestPackagesBuilder<'a, T, State>
where
    T: InstallSession,
    State: install_test_packages_builder::State,
{
    pub fn packages(mut self, packages: Vec<install::Spec>) -> Self {
        self.packages = packages;
        self
    }

    pub fn package(mut self, package: install::Spec) -> Self {
        self.packages.push(package);
        self
    }
}

impl<T, State> InstallTestPackagesBuilder<'_, T, State>
where
    State: install_test_packages_builder::State + install_test_packages_builder::IsComplete,
    T: InstallSession,
{
    pub async fn install(
        self,
        build_token: &BuildToken,
    ) -> Result<(Vec<LockedPackage>, LockfileHandle), Error> {
        let args = self._build();
        if args.packages.is_empty() {
            return Ok((Vec::new(), LockfileHandle::default()));
        }
        let span = tracing::info_span!("Installing test dependencies", count = args.packages.len());
        install_test_packages(args, build_token)
            .instrument(span)
            .await
    }
}

/// Build dependencies installed into a build tree.
pub struct BuildDependencies {
    pub packages: Vec<LockedPackage>,
    pub lockfile: LockfileHandle,
    pub token: BuildToken,
}

/// Installs build dependencies into a build tree.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct InstallBuildDependencies<'a, T>
where
    T: InstallSession,
{
    #[builder(start_fn)]
    config: &'a Config,
    #[builder(start_fn)]
    tree: &'a T,
    #[builder(field)]
    packages: Vec<install::Spec>,
    #[builder(field)]
    build_packages: Vec<install::Spec>,
    package_db: PackageDB,
}

impl<'a, T, State> InstallBuildDependenciesBuilder<'a, T, State>
where
    T: InstallSession,
    State: install_build_dependencies_builder::State,
{
    pub fn packages(mut self, packages: Vec<install::Spec>) -> Self {
        self.packages = packages;
        self
    }

    pub fn package(mut self, package: install::Spec) -> Self {
        self.packages.push(package);
        self
    }

    pub fn build_packages(mut self, packages: Vec<install::Spec>) -> Self {
        self.build_packages = packages;
        self
    }
}

impl<T, State> InstallBuildDependenciesBuilder<'_, T, State>
where
    State:
        install_build_dependencies_builder::State + install_build_dependencies_builder::IsComplete,
    T: InstallSession,
{
    pub async fn install(self) -> Result<BuildDependencies, Error> {
        let args = self._build();
        if args.packages.is_empty() && args.build_packages.is_empty() {
            return Ok(BuildDependencies {
                packages: Vec::new(),
                lockfile: LockfileHandle::default(),
                token: BuildToken {
                    build_lockfile: args.tree.lockfile()?,
                },
            });
        }
        install_build_dependencies(args)
            .instrument(tracing::info_span!("Installing build dependencies"))
            .await
    }
}

async fn resolve_install(
    config: &Config,
    package_db: PackageDB,
    packages: Vec<install::Spec>,
    lockfile: &Lockfile<ReadOnly>,
    build_lockfile: Option<&Lockfile<ReadOnly>>,
    section: LockedPackageLockType,
) -> Result<(Vec<MaterialisedPackage>, LockfileHandle), Error> {
    let mut installed: HashMap<LockedPackageLockType, Vec<LockedPackage>> = HashMap::new();
    installed.insert(section, lockfile.rocks().values().cloned().collect_vec());
    if let Some(build_lockfile) = build_lockfile {
        installed.insert(
            LockedPackageLockType::Build,
            build_lockfile.rocks().values().cloned().collect_vec(),
        );
    }

    let resolve = ResolvePackageDependencies::new(package_db, config).installed(installed);
    let resolve = match section {
        LockedPackageLockType::Regular => resolve.packages(packages),
        LockedPackageLockType::Build => resolve.build_packages(packages),
        LockedPackageLockType::Test => resolve.test_packages(packages),
    };
    let resolved = resolve.resolve().await?;

    if let Some(build_lockfile) = build_lockfile {
        let missing_build_dependencies = resolved
            .build
            .as_ref()
            .into_iter()
            .flat_map(|packages| packages.iter())
            .filter(|(id, _)| build_lockfile.get(id).is_none())
            .map(|(_, package)| format!("{}@{}", package.spec.name(), package.spec.version()))
            .unique()
            .collect_vec();
        if !missing_build_dependencies.is_empty() {
            return Err(Error::MissingBuildDependencies(
                missing_build_dependencies.join("\n"),
            ));
        }
    }

    materialise_section(config, resolved, section).await
}

async fn materialise_section(
    config: &Config,
    resolved: ResolvedArtifacts,
    section: LockedPackageLockType,
) -> Result<(Vec<MaterialisedPackage>, LockfileHandle), Error> {
    let mut artifacts = Materialise::new(config)
        .resolved(resolved)
        .section(section)
        .materialise()
        .await?;
    let handle = LockfileHandle::from_artifacts(&artifacts);
    let packages = artifacts
        .get_mut(section)
        .take()
        .unwrap_or_default()
        .into_values()
        .collect_vec();
    Ok((packages, handle))
}

async fn install_regular_packages<T>(
    install: InstallPackages<'_, T>,
    build_token: &BuildToken,
) -> Result<(Vec<LockedPackage>, LockfileHandle), Error>
where
    T: InstallSession,
{
    let InstallPackages {
        config,
        tree,
        packages,
        package_db,
    } = install;

    let duplicate_entrypoints = packages
        .iter()
        .filter(|pkg| pkg.entry_type == tree::EntryType::Entrypoint)
        .map(|pkg| pkg.package.name())
        .duplicates()
        .cloned()
        .collect_vec();

    if !duplicate_entrypoints.is_empty() {
        return Err(Error::DuplicateEntrypoints(package::List::new(
            duplicate_entrypoints,
        )));
    }

    let lockfile = tree.lockfile()?;
    let build_lockfile = &build_token.build_lockfile;
    let (packages, handle) = resolve_install(
        config,
        package_db,
        packages,
        &lockfile,
        Some(build_lockfile),
        LockedPackageLockType::Regular,
    )
    .await?;

    // Entrypoints that are already installed. The requested build behaviour decides how to
    // react to them.
    let mut conflicting_entrypoints: Vec<LockedPackage> = Vec::new();
    let mut unforced_conflicts = Vec::new();
    for package in packages
        .iter()
        .filter(|package| package.entry_type == tree::EntryType::Entrypoint)
    {
        match lockfile.entrypoint(package.package.name()) {
            None => continue,
            Some(existing) => match package.build_behaviour {
                // Forced installs replace the existing entrypoint.
                BuildBehaviour::Force => {
                    conflicting_entrypoints.push(existing.clone());
                }
                // `Ignore` tolerates re-installing the exact same version.
                BuildBehaviour::Ignore if existing.version() == package.package.version() => {}
                // Any other existing entrypoint is a conflict.
                BuildBehaviour::Ignore | BuildBehaviour::Conflict => {
                    unforced_conflicts.push(format!("{}@{}", existing.name(), existing.version()));
                }
            },
        }
    }

    if !unforced_conflicts.is_empty() {
        return Err(Error::ConflictingEntrypoints(unforced_conflicts.join("\n")));
    }

    for package in &conflicting_entrypoints {
        tree.cleanup(package, tree::EntryType::Entrypoint)?;
    }

    let entry_types: HashMap<LockedPackageId, tree::EntryType> = packages
        .iter()
        .map(|pkg| (pkg.package.spec.id(), pkg.entry_type))
        .collect();

    let built = PipelineBuild::new(config, tree)
        .packages(packages)
        .build()
        .await?;

    let mut installed_packages: HashMap<LockedPackageId, LockedPackage> = lockfile
        .rocks()
        .iter()
        .map(|(id, package)| (id.clone(), package.clone()))
        .collect();
    installed_packages.extend(built.iter().map(|pkg| (pkg.spec().id(), pkg.clone())));
    let installed_build_deps: HashMap<LockedPackageId, LockedPackage> = build_lockfile
        .rocks()
        .iter()
        .map(|(id, package)| (id.clone(), package.clone()))
        .collect();

    // NOTE: re-read the lockfile here. `lockfile` was loaded before the builds ran,
    // and flushing that snapshot would drop the binaries each build recorded.
    tree.lockfile()?.map_then_flush(|lockfile| {
        for package in &conflicting_entrypoints {
            // A rebuilt package already recorded its own binaries during the build.
            // Removing it here would drop them again.
            if entry_types.contains_key(&package.id()) {
                continue;
            }
            lockfile.remove_by_id(&package.id());
        }
        for package in &built {
            let entry_type = entry_types[&package.spec().id()];
            lockfile.add_dependencies(package, entry_type, &installed_packages)?;
            lockfile.add_build_dependencies(package, &installed_build_deps)?;
        }
        Ok::<_, io::Error>(())
    })?;

    Ok((built, handle))
}

async fn install_test_packages<T>(
    install: InstallTestPackages<'_, T>,
    build_token: &BuildToken,
) -> Result<(Vec<LockedPackage>, LockfileHandle), Error>
where
    T: InstallSession,
{
    let InstallTestPackages {
        config,
        tree,
        packages,
        package_db,
    } = install;

    let lockfile = tree.lockfile()?;
    let build_lockfile = &build_token.build_lockfile;
    let (packages, handle) = resolve_install(
        config,
        package_db,
        packages,
        &lockfile,
        Some(build_lockfile),
        LockedPackageLockType::Test,
    )
    .await?;

    let built = PipelineBuild::new(config, tree)
        .packages(packages)
        .build()
        .await?;

    Ok((built, handle))
}

async fn install_build_dependencies<T>(
    install: InstallBuildDependencies<'_, T>,
) -> Result<BuildDependencies, Error>
where
    T: InstallSession,
{
    let package_db = install.package_db;
    let packages = install.packages;
    let build_packages = install.build_packages;
    let config = install.config;
    let tree = install.tree;

    let lockfile = tree.lockfile()?;
    let mut installed: HashMap<LockedPackageLockType, Vec<LockedPackage>> = HashMap::new();
    installed.insert(
        LockedPackageLockType::Build,
        lockfile.rocks().values().cloned().collect_vec(),
    );

    let resolved = ResolvePackageDependencies::new(package_db, config)
        .installed(installed)
        .packages(packages)
        .build_packages(build_packages)
        .resolve()
        .await?;
    let (packages, handle) =
        materialise_section(config, resolved, LockedPackageLockType::Build).await?;

    let built = PipelineBuild::new(config, tree)
        .packages(packages)
        .build()
        .await?;

    let build_lockfile = tree.lockfile()?;

    Ok(BuildDependencies {
        packages: built,
        lockfile: handle,
        token: BuildToken { build_lockfile },
    })
}

#[derive(Error, Debug, Diagnostic)]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaVersion(#[from] lua::version::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaInstallation(#[from] lua::installation::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    FlushLockfile(#[from] FlushLockfileError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] tree::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    WorkspaceTree(#[from] TreeError),
    #[error("error preparing the LuaRocks compatibility layer")]
    #[diagnostic(forward(0))]
    LuaRocks(#[from] Box<installation::Error>),
    #[error("error initialising remote package DB")]
    #[diagnostic(forward(0))]
    PackageDB(#[from] package::db::Error),
    #[error("cannot install duplicate entrypoints:\n{0}")]
    DuplicateEntrypoints(package::List),
    #[error("cannot install conflicting entrypoints:\n{0}")]
    #[diagnostic(help(
        "only a single entrypoint per package is allowed.\nif you would like to replace the existing entrypoints, retry with `--force` to overwrite the existing entrypoint"
    ))]
    ConflictingEntrypoints(String),
    #[error("build dependencies are not installed:\n{0}")]
    #[diagnostic(help(
        "install the build dependencies into the build tree before installing this package set"
    ))]
    MissingBuildDependencies(String),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Resolve(#[from] resolve::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Download(#[from] MaterialiseError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Build(#[from] PipelineBuildError),
}

impl From<installation::Error> for Error {
    fn from(source: installation::Error) -> Self {
        Self::LuaRocks(Box::new(source))
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
"#,
                    ))?,
            );
        }
        Ok(())
    }
}
