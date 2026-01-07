use std::{
    io::{self, Cursor},
    path::{Path, PathBuf},
    sync::Arc,
};

use bon::Builder;
use bytes::Bytes;
use futures::StreamExt;
use itertools::Itertools;
use path_slash::PathExt;
use strum::IntoEnumIterator;
use thiserror::Error;
use tokio::{fs::File, io::AsyncWriteExt};

use crate::{
    build::{RemotePackageSourceSpec, SrcRockSource},
    config::Config,
    lockfile::{LocalPackageLockType, ReadOnly},
    lua_rockspec::RemoteLuaRockspec,
    operations::{
        self,
        resolve::{PackageInstallData, Resolve, ResolveDependenciesError},
        DownloadedRockspec, FetchSrcError, PackageInstallSpec, UnpackError,
    },
    package::PackageReq,
    progress::{MultiProgress, Progress, ProgressBar},
    project::{project_toml::LocalProjectTomlValidationError, Project, ProjectError},
    remote_package_db::{RemotePackageDB, RemotePackageDBError},
    rockspec::Rockspec,
    tree::EntryType,
};

pub enum VendorTarget {
    /// Vendor dependencies of a Lux project
    Project(Project),
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

    progress: Option<Arc<Progress<MultiProgress>>>,
}

#[derive(Error, Debug)]
pub enum VendorError {
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error("project validation failed:\n{0}")]
    LocalProjectTomlValidation(#[from] LocalProjectTomlValidationError),
    #[error("error initialising remote package DB:\n{0}")]
    RemotePackageDB(#[from] RemotePackageDBError),
    #[error("failed to resolve dependencies:\n{0}")]
    ResolveDependencies(#[from] ResolveDependenciesError),
    #[error("failed to delete vendor directory {0}:\n{1}")]
    DeleteVendorDir(String, io::Error),
    #[error("failed to create vendor directory {0}:\n{1}")]
    CreateVendorDir(String, io::Error),
    #[error("failed to create {0}:\n{1}")]
    CreateSrcRock(String, io::Error),
    #[error("failed to vendor Lua RockSpec:\n{0}")]
    LuaRockSpec(String),
    #[error("failed to write Lua RockSpec {0}:\n{1}")]
    WriteLuaRockSpec(String, io::Error),
    #[error("failed to unpack src.rock:\n{0}")]
    Unpack(#[from] UnpackError),
    #[error("failed to fetch rock source:\n{0}")]
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
    let progress = match args.progress {
        Some(p) => p,
        None => MultiProgress::new_arc(args.config),
    };
    let mut all_packages = Vec::new();

    for lock_type in LocalPackageLockType::iter() {
        let (package_db, install_specs) =
            mk_resolve_args(lock_type, no_lock, &target, config, progress.clone()).await?;

        let (dep_tx, mut dep_rx) = tokio::sync::mpsc::unbounded_channel();
        Resolve::<'_, ReadOnly>::new()
            .dependencies_tx(dep_tx.clone())
            .build_dependencies_tx(dep_tx)
            .packages(install_specs)
            .package_db(Arc::new(package_db))
            .config(config)
            .progress(progress.clone())
            .get_all_dependencies()
            .await?;

        while let Some(dep) = dep_rx.recv().await {
            all_packages.push(dep);
        }
    }

    if !no_delete && vendor_dir.exists() {
        tokio::fs::remove_dir_all(&vendor_dir)
            .await
            .map_err(|err| {
                VendorError::DeleteVendorDir(vendor_dir.to_slash_lossy().to_string(), err)
            })?;
    }

    vendor_sources(Arc::new(vendor_dir), progress, config.clone(), all_packages).await
}

async fn mk_resolve_args(
    lock_type: LocalPackageLockType,
    no_lock: bool,
    target: &VendorTarget,
    config: &Config,
    progress: Arc<Progress<MultiProgress>>,
) -> Result<(RemotePackageDB, Vec<PackageInstallSpec>), VendorError> {
    match &target {
        VendorTarget::Project(project) => {
            let toml = project.toml().into_local()?;
            let lockfile = project.lockfile()?;
            let package_db = if !no_lock {
                lockfile.local_pkg_lock(&lock_type).clone().into()
            } else {
                let bar = progress.map(|p| p.new_bar());
                RemotePackageDB::from_config(config, &bar).await?
            };
            let mut install_specs = mk_dependencies_vec(&lock_type, &toml)?;
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
            Ok((package_db, install_specs))
        }
        VendorTarget::Rockspec(remote_lua_rockspec) => {
            let bar = progress.map(|p| p.new_bar());
            let package_db = RemotePackageDB::from_config(config, &bar).await?;
            let install_specs = mk_dependencies_vec(&lock_type, remote_lua_rockspec)?;
            Ok((package_db, install_specs))
        }
    }
}

fn mk_dependencies_vec<R: Rockspec>(
    lock_type: &LocalPackageLockType,
    rockspec: &R,
) -> Result<Vec<PackageInstallSpec>, LocalProjectTomlValidationError> {
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

    Ok(dependencies
        .into_iter()
        .unique()
        .cloned()
        .map(|dep| PackageInstallSpec::new(dep, EntryType::Entrypoint).build())
        .collect_vec())
}

async fn vendor_sources(
    vendor_dir: Arc<PathBuf>,
    progress: Arc<Progress<MultiProgress>>,
    config: Config,
    packages: Vec<PackageInstallData>,
) -> Result<(), VendorError> {
    futures::stream::iter(packages.into_iter().map(|dep| {
        let vendor_dir = Arc::clone(&vendor_dir);
        let progress = Arc::clone(&progress);
        let config = config.clone();
        tokio::spawn(async move {
            match dep.downloaded_rock {
                crate::operations::RemoteRockDownload::RockspecOnly { rockspec_download } => {
                    vendor_rockspec_sources(
                        &vendor_dir,
                        rockspec_download,
                        None,
                        &config,
                        &progress,
                    )
                    .await?
                }
                crate::operations::RemoteRockDownload::BinaryRock {
                    rockspec_download,
                    packed_rock,
                } => {
                    vendor_binary_rock(&vendor_dir, rockspec_download, packed_rock, &progress)
                        .await?
                }
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
                        &progress,
                    )
                    .await?
                }
            };
            Ok::<_, VendorError>(())
        })
    }))
    .buffered(config.max_jobs())
    .collect::<Vec<_>>()
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
    progress: &Progress<MultiProgress>,
) -> Result<(), VendorError> {
    let rockspec = rockspec_download.rockspec;
    let package = rockspec.package();
    let version = rockspec.version();
    let package_version_str = format!("{}@{}", package, version);
    let bar = progress.map(|p| {
        p.add(ProgressBar::from(format!(
            "💼 Vendoring source of {}",
            &package_version_str,
        )))
    });
    let source_spec = match src_rock_source {
        Some(src_rock_source) => RemotePackageSourceSpec::SrcRock(src_rock_source),
        None => RemotePackageSourceSpec::RockSpec(rockspec_download.source_url),
    };

    let package_vendor_dir = vendor_dir.join(&package_version_str);

    tokio::fs::create_dir_all(&package_vendor_dir)
        .await
        .map_err(|err| {
            VendorError::CreateVendorDir(package_vendor_dir.to_slash_lossy().to_string(), err)
        })?;

    let rockspec_lua_content = rockspec
        .to_lua_remote_rockspec_string()
        .map_err(|err| VendorError::LuaRockSpec(err.to_string()))?;

    let rockspec_file_name = format!("{}-{}.rockspec", package, version);
    let rockspec_path = vendor_dir.join(rockspec_file_name);
    tokio::fs::write(&rockspec_path, rockspec_lua_content)
        .await
        .map_err(|err| {
            VendorError::WriteLuaRockSpec(rockspec_path.to_slash_lossy().to_string(), err)
        })?;

    match source_spec {
        RemotePackageSourceSpec::SrcRock(SrcRockSource {
            bytes,
            source_url: _,
        }) => {
            let cursor = Cursor::new(&bytes);
            operations::unpack_src_rock(cursor, package_vendor_dir, &bar).await?;
        }
        RemotePackageSourceSpec::RockSpec(source_url) => {
            operations::FetchSrc::new(&package_vendor_dir, &rockspec, config, &bar)
                .maybe_source_url(source_url)
                .fetch_internal()
                .await?;
        }
    }

    bar.map(|bar| bar.finish_and_clear());

    Ok(())
}

async fn vendor_binary_rock(
    vendor_dir: &Path,
    rockspec_download: DownloadedRockspec,
    packed_rock: Bytes,
    progress: &Progress<MultiProgress>,
) -> Result<(), VendorError> {
    let rockspec = rockspec_download.rockspec;
    let package = rockspec.package();
    let version = rockspec.version();

    let file_name = format!("{}@{}.rock", package, version);

    let bar = progress.map(|p| {
        p.add(ProgressBar::from(format!(
            "💼 Vendoring pre-built binary .rock: {}",
            &file_name,
        )))
    });

    tokio::fs::create_dir_all(&vendor_dir)
        .await
        .map_err(|err| {
            VendorError::CreateVendorDir(vendor_dir.to_slash_lossy().to_string(), err)
        })?;

    let dest_file = vendor_dir.join(&file_name);
    let mut file = File::create(&dest_file)
        .await
        .map_err(|err| VendorError::CreateSrcRock(dest_file.to_slash_lossy().to_string(), err))?;
    file.write_all(&packed_rock)
        .await
        .map_err(|err| VendorError::CreateSrcRock(dest_file.to_slash_lossy().to_string(), err))?;

    bar.map(|bar| bar.finish_and_clear());

    Ok(())
}
