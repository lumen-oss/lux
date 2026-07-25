use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
};

use bon::Builder;
use bytes::Bytes;
use futures::StreamExt;
use itertools::Itertools;
use miette::Diagnostic;
use strum::IntoEnumIterator;
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tracing::{span, Instrument};

use crate::{
    build::{RemotePackageSourceSpec, SrcRockSource},
    config::Config,
    fs,
    lockfile::{LocalPackageLockType, ReadOnly},
    lua_rockspec::RemoteLuaRockspec,
    operations::{
        self,
        resolve::{PackageInstallData, Resolve, ResolveDependenciesError},
        DownloadedRockspec, FetchSrcError, PackageInstallSpec, UnpackError,
    },
    package::PackageReq,
    project::project_toml::LocalProjectTomlValidationError,
    remote_package_db::{RemotePackageDB, RemotePackageDBError},
    rockspec::Rockspec,
    tree::EntryType,
    workspace::{Workspace, WorkspaceError},
};

#[allow(clippy::large_enum_variant)]
pub enum VendorTarget {
    /// Vendor dependencies of a Lux workspace
    Workspace(Workspace),

    /// Vendor dependencies of a Lua RockSpec
    Rockspec(RemoteLuaRockspec),
}

/// Vendor a project's dependencies into the specified directory at `<vendor_dir>`.
/// After this command completes the vendor directory specified by `<vendor_dir>`
/// will contain all remote sources from dependencies specified.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Vendor<'a> {
    target: VendorTarget,

    /// The directory in which to vendor the dependencies.
    vendor_dir: PathBuf,

    /// Ignore the project's lockfile.
    no_lock: Option<bool>,

    /// Don't delete the `<vendor-dir>` when vendoring,{n}
    /// but rather keep all existing contents of the vendor directory.
    no_delete: Option<bool>,

    config: &'a Config,
}

#[derive(Error, Debug, Diagnostic)]
pub enum VendorError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error("project validation failed:\n{0}")]
    #[diagnostic(forward(0))]
    LocalProjectTomlValidation(#[from] LocalProjectTomlValidationError),
    #[error("error initialising remote package DB:\n{0}")]
    #[diagnostic(forward(0))]
    RemotePackageDB(#[from] RemotePackageDBError),
    #[error("failed to resolve dependencies:\n{0}")]
    #[diagnostic(forward(0))]
    ResolveDependencies(#[from] ResolveDependenciesError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::FsError),
    #[error("failed to vendor Lua RockSpec:\n{0}")]
    LuaRockSpec(String),
    #[error("failed to unpack src.rock:\n{0}")]
    #[diagnostic(forward(0))]
    Unpack(#[from] UnpackError),
    #[error("failed to fetch rock source:\n{0}")]
    #[diagnostic(forward(0))]
    FetchSrc(#[from] FetchSrcError),
}

impl<State> VendorBuilder<'_, State>
where
    State: vendor_builder::State + vendor_builder::IsComplete,
{
    pub async fn vendor_dependencies(self) -> Result<(), VendorError> {
        do_vendor_dependencies(self._build()).await
    }
}

async fn do_vendor_dependencies(args: Vendor<'_>) -> Result<(), VendorError> {
    let vendor_dir = args.vendor_dir;
    let no_delete = args.no_delete.unwrap_or(false);
    let no_lock = args.no_lock.unwrap_or(false);
    let target = args.target;
    let config = args.config;
    let mut all_packages = Vec::new();

    for lock_type in LocalPackageLockType::iter() {
        let (package_db, install_specs) =
            mk_resolve_args(lock_type, no_lock, &target, config).await?;

        let (dep_tx, mut dep_rx) = tokio::sync::mpsc::unbounded_channel();
        Resolve::<'_, ReadOnly>::new()
            .dependencies_tx(dep_tx.clone())
            .build_dependencies_tx(dep_tx)
            .packages(install_specs)
            .package_db(Arc::new(package_db))
            .config(config)
            .get_all_dependencies()
            .await?;

        while let Some(dep) = dep_rx.recv().await {
            all_packages.push(dep);
        }
    }

    if !no_delete && vendor_dir.exists() {
        fs::tokio::remove_dir_all(&vendor_dir).await?;
    }

    vendor_sources(Arc::new(vendor_dir), config.clone(), all_packages).await
}

async fn mk_resolve_args(
    lock_type: LocalPackageLockType,
    no_lock: bool,
    target: &VendorTarget,
    config: &Config,
) -> Result<(RemotePackageDB, Vec<PackageInstallSpec>), VendorError> {
    match &target {
        VendorTarget::Workspace(workspace) => {
            let lockfile = workspace.lockfile()?;
            let package_db = if !no_lock {
                lockfile.local_pkg_lock(&lock_type).clone().into()
            } else {
                RemotePackageDB::from_config(config).await?
            };
            let mut install_specs = Vec::new();
            for project in workspace.members() {
                let toml = project.toml().into_local()?;
                push_dependencies(&lock_type, &toml, &mut install_specs)?;
                if lock_type == LocalPackageLockType::Test {
                    for test_spec_dependency in toml
                        .test()
                        .current_platform()
                        .test_dependencies(project)
                        .iter()
                        .cloned()
                        .map(|dep| PackageInstallSpec::new(dep, EntryType::Entrypoint).build())
                    {
                        install_specs.push(test_spec_dependency);
                    }
                }
            }
            Ok((package_db, install_specs))
        }
        VendorTarget::Rockspec(remote_lua_rockspec) => {
            let package_db = RemotePackageDB::from_config(config).await?;
            let mut install_specs = Vec::new();
            push_dependencies(&lock_type, remote_lua_rockspec, &mut install_specs)?;
            Ok((package_db, install_specs))
        }
    }
}

fn push_dependencies<R: Rockspec>(
    lock_type: &LocalPackageLockType,
    rockspec: &R,
    install_specs: &mut Vec<PackageInstallSpec>,
) -> Result<(), LocalProjectTomlValidationError> {
    let dependencies: Vec<&PackageReq> = match lock_type {
        LocalPackageLockType::Regular => rockspec
            .dependencies()
            .current_platform()
            .iter()
            .map(|dep| dep.package_req())
            .collect_vec(),
        LocalPackageLockType::Test => rockspec
            .test_dependencies()
            .current_platform()
            .iter()
            .map(|dep| dep.package_req())
            .collect_vec(),
        LocalPackageLockType::Build => rockspec
            .build_dependencies()
            .current_platform()
            .iter()
            .map(|dep| dep.package_req())
            .collect_vec(),
    };
    install_specs.extend(
        dependencies
            .into_iter()
            .unique()
            .cloned()
            .map(|dep| PackageInstallSpec::new(dep, EntryType::Entrypoint).build())
            .collect_vec(),
    );
    Ok(())
}

async fn vendor_sources(
    vendor_dir: Arc<PathBuf>,
    config: Config,
    packages: Vec<PackageInstallData>,
) -> Result<(), VendorError> {
    futures::stream::iter(packages.into_iter().map(|dep| {
        let vendor_dir = Arc::clone(&vendor_dir);
        let config = config.clone();
        tokio::spawn(
            async move {
                match dep.downloaded_rock {
                    crate::operations::RemoteRockDownload::RockspecOnly { rockspec_download } => {
                        vendor_rockspec_sources(&vendor_dir, rockspec_download, None, &config)
                            .await?
                    }
                    crate::operations::RemoteRockDownload::BinaryRock {
                        rockspec_download,
                        packed_rock,
                    } => vendor_binary_rock(&vendor_dir, rockspec_download, packed_rock).await?,
                    crate::operations::RemoteRockDownload::SrcRock {
                        rockspec_download,
                        src_rock,
                        source_url,
                    } => {
                        let src_rock_source = SrcRockSource {
                            bytes: src_rock,
                            source_url,
                        };
                        vendor_rockspec_sources(
                            &vendor_dir,
                            rockspec_download,
                            Some(src_rock_source),
                            &config,
                        )
                        .await?
                    }
                };
                Ok::<_, VendorError>(())
            }
            .instrument(tracing::trace_span!("vendor_worker")),
        )
    }))
    .buffered(config.max_jobs())
    .collect::<Vec<_>>()
    .instrument(tracing::trace_span!("vendor_collector"))
    .await
    .into_iter()
    .flatten()
    .try_collect()
}

async fn vendor_rockspec_sources(
    vendor_dir: &Path,
    rockspec_download: DownloadedRockspec,
    src_rock_source: Option<SrcRockSource>,
    config: &Config,
) -> Result<(), VendorError> {
    let rockspec = rockspec_download.rockspec;
    let package = rockspec.package();
    let version = rockspec.version();
    let package_version_str = format!("{}@{}", package, version);

    let span = span!(
        tracing::Level::INFO,
        "💼 Vendoring source",
        package = package.to_string(),
        version = version.to_string(),
    );
    let _enter = span.enter();

    let source_spec = match src_rock_source {
        Some(src_rock_source) => RemotePackageSourceSpec::SrcRock(src_rock_source),
        None => RemotePackageSourceSpec::RockSpec(rockspec_download.source_url),
    };

    let package_vendor_dir = vendor_dir.join(&package_version_str);

    fs::tokio::create_dir_all(&package_vendor_dir).await?;

    let rockspec_lua_content = rockspec
        .to_lua_remote_rockspec_string()
        .map_err(|err| VendorError::LuaRockSpec(err.to_string()))?;

    let rockspec_file_name = format!("{}-{}.rockspec", package, version);
    let rockspec_path = vendor_dir.join(rockspec_file_name);
    fs::tokio::write(&rockspec_path, rockspec_lua_content).await?;

    match source_spec {
        RemotePackageSourceSpec::SrcRock(SrcRockSource {
            bytes,
            source_url: _,
        }) => {
            let cursor = Cursor::new(&bytes);
            operations::unpack_src_rock(cursor, package_vendor_dir).await?;
        }
        RemotePackageSourceSpec::RockSpec(source_url) => {
            operations::FetchSrc::new(&package_vendor_dir, &rockspec, config)
                .maybe_source_url(source_url)
                .fetch_internal()
                .await?;
        }
    }

    Ok(())
}

async fn vendor_binary_rock(
    vendor_dir: &Path,
    rockspec_download: DownloadedRockspec,
    packed_rock: Bytes,
) -> Result<(), VendorError> {
    let rockspec = rockspec_download.rockspec;
    let package = rockspec.package();
    let version = rockspec.version();

    let span = span!(
        tracing::Level::INFO,
        "💼 Vendoring pre-built binary",
        package = package.to_string(),
        version = version.to_string(),
    );
    let _enter = span.enter();

    let file_name = format!("{}@{}.rock", package, version);

    fs::tokio::create_dir_all(&vendor_dir).await?;

    let dest_file = vendor_dir.join(&file_name);
    let mut file = fs::tokio::create(&dest_file).await?;
    file.write_all(&packed_rock)
        .await
        .map_err(|source| fs::FsError::Write {
            path: dest_file.to_path_buf(),
            source,
        })?;

    Ok(())
}
