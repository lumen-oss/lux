use std::{
    io::{self, Cursor},
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::Arc,
};

use bon::Builder;
use bytes::Bytes;
use futures::StreamExt;
use itertools::Itertools;
use miette::Diagnostic;
use path_slash::PathExt;
use strum::IntoEnumIterator;
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tracing::Instrument;

use crate::{
    build::{RemotePackageSourceSpec, SrcRockSource},
    config::Config,
    fs,
    lockfile::{LocalPackageLockType, ReadOnly, RemotePackageSourceUrl},
    lua_rockspec::{BuildBackendSpec, RemoteLuaRockspec},
    operations::{
        self,
        resolve::{PackageInstallData, Resolve, ResolveDependenciesError},
        DownloadedRockspec, FetchSrcError, PackageInstallSpec, UnpackError,
    },
    package::{PackageReq, PackageSpec},
    project::project_toml::LocalProjectTomlValidationError,
    remote_package_db::{RemotePackageDB, RemotePackageDBError},
    reqwest::RequestError,
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
    #[error("project validation failed")]
    #[diagnostic(forward(0))]
    LocalProjectTomlValidation(#[from] LocalProjectTomlValidationError),
    #[error("error initialising remote package DB")]
    #[diagnostic(forward(0))]
    RemotePackageDB(#[from] RemotePackageDBError),
    #[error("failed to resolve dependencies")]
    #[diagnostic(forward(0))]
    ResolveDependencies(#[from] ResolveDependenciesError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::FsError),
    #[error("failed to vendor Lua RockSpec:\n{0}")]
    LuaRockSpec(String),
    #[error("failed to unpack src.rock")]
    #[diagnostic(forward(0))]
    Unpack(#[from] UnpackError),
    #[error("failed to fetch rock source")]
    #[diagnostic(forward(0))]
    FetchSrc(#[from] FetchSrcError),
    #[error("failed to download rock source")]
    #[diagnostic(forward(0))]
    Request(#[from] RequestError),
    #[error("failed to run `cargo vendor`")]
    #[diagnostic(help("ensure cargo is installed"))]
    CargoVendor { source: io::Error },
    #[error("cargo vendor failed.\nstatus: {status}\nstdout: {stdout}\nstderr: {stderr}")]
    #[diagnostic(help("check the output for details."))]
    CargoVendorFailure {
        status: ExitStatus,
        stdout: String,
        stderr: String,
    },
}

impl<State> VendorBuilder<'_, State>
where
    State: vendor_builder::State + vendor_builder::IsComplete,
{
    pub async fn vendor_dependencies(self) -> Result<(), VendorError> {
        do_vendor_dependencies(self._build()).await
    }
}

const CARGO_VENDOR_SUBDIR: &str = "cargo";

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

    // The lockfile may contain the same package (name@version) multiple times,
    // with different constraints.
    let all_packages = all_packages
        .into_iter()
        .unique_by(|pkg| (pkg.spec.name().clone(), pkg.spec.version().clone()))
        .collect_vec();

    let cargo_deps: Vec<(PackageSpec, Option<PathBuf>)> = all_packages
        .iter()
        .filter_map(|pkg| {
            match pkg
                .downloaded_rock
                .rockspec()
                .build()
                .current_platform()
                .build_backend
            {
                Some(BuildBackendSpec::RustMlua(_) | BuildBackendSpec::RustBinary(_)) => Some((
                    pkg.spec.to_package(),
                    pkg.downloaded_rock
                        .rockspec()
                        .source()
                        .current_platform()
                        .unpack_dir
                        .clone(),
                )),
                _ => None,
            }
        })
        .collect();

    if !no_delete && vendor_dir.exists() {
        fs::tokio::remove_dir_all(&vendor_dir).await?;
    }

    let vendor_dir = Arc::new(vendor_dir);
    vendor_sources(vendor_dir.clone(), config.clone(), all_packages).await?;
    vendor_target_cargo_deps(&vendor_dir, &target, config).await?;
    for (dep, unpack_dir) in cargo_deps {
        vendor_package_cargo_deps(&vendor_dir, &dep, &unpack_dir, config).await?;
    }
    Ok(())
}

async fn mk_resolve_args(
    lock_type: LocalPackageLockType,
    no_lock: bool,
    target: &VendorTarget,
    config: &Config,
) -> Result<(RemotePackageDB, Vec<PackageInstallSpec>), VendorError> {
    match &target {
        VendorTarget::Workspace(workspace) => {
            // Resolve against the project's lockfile if present, otherwise fall
            // back to the remote package DB (e.g. for a project that has not
            // yet generated a lockfile).
            let lockfile = workspace.try_lockfile()?;
            let package_db = match lockfile {
                Some(lockfile) if !no_lock => lockfile.local_pkg_lock(&lock_type).clone().into(),
                _ => RemotePackageDB::from_config(config).await?,
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

#[tracing::instrument(
    name = "Vendoring source",
    level = "info",
    skip_all,
    fields(
        package = rockspec_download.rockspec.package().to_string(),
        version = rockspec_download.rockspec.version().to_string(),
    ),
)]
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

    let source_spec = match src_rock_source {
        Some(src_rock_source) => RemotePackageSourceSpec::SrcRock(src_rock_source),
        None => RemotePackageSourceSpec::RockSpec(rockspec_download.source_url),
    };

    let source_path = vendor_dir.join(&package_version_str);

    fs::tokio::create_dir_all(vendor_dir).await?;

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
            fs::tokio::write(&source_path, &bytes).await?;
        }
        RemotePackageSourceSpec::RockSpec(source_url) => match source_url {
            Some(RemotePackageSourceUrl::Url { url }) => {
                let bytes = crate::reqwest::download_bytes(config, &url).await?;
                fs::tokio::write(&source_path, &bytes).await?;
            }
            _ => {
                fs::tokio::create_dir_all(&source_path).await?;
                operations::FetchSrc::new(&source_path, &rockspec, config)
                    .maybe_source_url(source_url)
                    .fetch_internal()
                    .await?;
            }
        },
    }

    Ok(())
}

#[tracing::instrument(
    name = "Vendoring pre-built binary",
    level = "info",
    skip_all,
    fields(
        package = rockspec_download.rockspec.package().to_string(),
        version = rockspec_download.rockspec.version().to_string(),
    ),
)]
async fn vendor_binary_rock(
    vendor_dir: &Path,
    rockspec_download: DownloadedRockspec,
    packed_rock: Bytes,
) -> Result<(), VendorError> {
    let rockspec = rockspec_download.rockspec;
    let package = rockspec.package();
    let version = rockspec.version();

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

    let rockspec_lua_content = rockspec
        .to_lua_remote_rockspec_string()
        .map_err(|err| VendorError::LuaRockSpec(err.to_string()))?;

    let rockspec_file_name = format!("{}-{}.rockspec", package, version);
    let rockspec_path = vendor_dir.join(rockspec_file_name);
    fs::tokio::write(&rockspec_path, rockspec_lua_content).await?;

    Ok(())
}

#[tracing::instrument(name = "Vendoring cargo dependencies", level = "info", skip_all)]
async fn vendor_target_cargo_deps(
    vendor_dir: &Path,
    target: &VendorTarget,
    config: &Config,
) -> Result<(), VendorError> {
    if is_cargo_build_backend(target) {
        match target {
            VendorTarget::Workspace(workspace) => {
                cargo_vendor(vendor_dir, workspace.root().as_path(), config).await
            }
            VendorTarget::Rockspec(rockspec) => {
                let temp_dir = fs::tempfile::tempdir()?;
                operations::FetchSrc::new(temp_dir.path(), rockspec, config)
                    .fetch_internal()
                    .await?;
                cargo_vendor(
                    vendor_dir,
                    &cargo_manifest_dir(
                        temp_dir.path(),
                        rockspec.source().current_platform().unpack_dir.as_deref(),
                    ),
                    config,
                )
                .await
            }
        }
    } else {
        Ok(())
    }
}

#[tracing::instrument(
    name = "Vendoring cargo dependencies",
    level = "info",
    skip_all,
    fields(
        package = package.name().to_string(),
        version = package.version().to_string(),
    ),
)]
async fn vendor_package_cargo_deps(
    vendor_dir: &Path,
    package: &PackageSpec,
    unpack_dir: &Option<PathBuf>,
    config: &Config,
) -> Result<(), VendorError> {
    let source_dir = vendor_dir.join(format!("{}@{}", package.name(), package.version()));
    let source_root = if source_dir.is_dir() {
        Some(source_dir)
    } else if source_dir.is_file() {
        let temp_dir = fs::tempfile::tempdir()?;
        extract_source_archive(&source_dir, unpack_dir.as_deref(), temp_dir.path()).await?;
        Some(temp_dir.path().to_path_buf())
    } else {
        None
    };
    if let Some(source_root) = source_root {
        cargo_vendor(
            vendor_dir,
            &cargo_manifest_dir(&source_root, unpack_dir.as_deref()),
            config,
        )
        .await?;
    }

    Ok(())
}

fn is_cargo_build_backend(target: &VendorTarget) -> bool {
    match target {
        VendorTarget::Workspace(workspace) => workspace.members().iter().any(|project| {
            project.toml().into_local().is_ok_and(|toml| {
                matches!(
                    toml.build().current_platform().build_backend.to_owned(),
                    Some(BuildBackendSpec::RustMlua(_) | BuildBackendSpec::RustBinary(_))
                )
            })
        }),
        VendorTarget::Rockspec(rockspec) => matches!(
            rockspec.build().current_platform().build_backend.to_owned(),
            Some(BuildBackendSpec::RustMlua(_) | BuildBackendSpec::RustBinary(_))
        ),
    }
}

/// The directory containing the crate's `Cargo.toml`.
fn cargo_manifest_dir(source_root: &Path, unpack_dir: Option<&Path>) -> PathBuf {
    match unpack_dir {
        Some(dir) => source_root.join(dir),
        None => source_root.to_path_buf(),
    }
}

async fn extract_source_archive(
    source_file: &Path,
    unpack_dir: Option<&Path>,
    dest_dir: &Path,
) -> Result<(), VendorError> {
    let bytes = fs::tokio::read(source_file).await?;
    let mime_type = infer::get(&bytes).map(|file_type| file_type.mime_type());
    let file_name = source_file
        .file_name()
        .map(|file_name| file_name.to_string_lossy())
        .unwrap_or(source_file.to_slash_lossy())
        .to_string();
    operations::unpack::unpack(
        mime_type,
        Cursor::new(bytes),
        unpack_dir.is_none(),
        file_name,
        dest_dir,
    )
    .await?;
    Ok(())
}

async fn cargo_vendor(
    vendor_dir: &Path,
    source_dir: &Path,
    config: &Config,
) -> Result<(), VendorError> {
    let cargo_vendor_dir = fs::sync::absolute(vendor_dir.join(CARGO_VENDOR_SUBDIR))?;
    fs::tokio::create_dir_all(&cargo_vendor_dir).await?;

    let output = config
        .wrapped_command("cargo", ["vendor", "--locked", "--versioned-dirs"])
        .arg(&cargo_vendor_dir)
        .current_dir(source_dir)
        .output()
        .await
        .map_err(|source| VendorError::CargoVendor { source })?;

    if output.status.success() {
        Ok(())
    } else {
        Err(VendorError::CargoVendorFailure {
            status: output.status,
            stdout: String::from_utf8_lossy(&output.stdout).into(),
            stderr: String::from_utf8_lossy(&output.stderr).into(),
        })
    }
}
