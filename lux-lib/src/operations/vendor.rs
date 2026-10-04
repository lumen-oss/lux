use std::{
    collections::HashMap,
    io::{self, Cursor},
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::Arc,
};

use bon::Builder;
use futures::StreamExt;
use futures::TryStreamExt;
use itertools::Itertools;
use miette::Diagnostic;
use path_slash::PathExt;
use strum::IntoEnumIterator;
use thiserror::Error;
use tokio::io::AsyncWriteExt;

use crate::{
    build::resolve_source_dir,
    config::Config,
    fs,
    lockfile::LockedPackageLockType,
    lua_rockspec::{BuildBackendSpec, RemoteLuaRockspec},
    operations::{
        self,
        pipeline::{
            discover::FindPackageFromLuarocks,
            download_sources_and_hash::{
                DownloadSourcesAndHash, DownloadSourcesAndHashError, DownloadedPackage,
                PackageSource,
            },
            resolve::{luarocks_build_backend_name, ResolveError, ResolvePackageDependencies},
            Artifacts,
        },
        PackageInstallSpec, UnpackError,
    },
    package::{PackageReq, PackageSpec},
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
    #[error("project validation failed")]
    #[diagnostic(forward(0))]
    LocalProjectTomlValidation(#[from] LocalProjectTomlValidationError),
    #[error("error initialising remote package DB")]
    #[diagnostic(forward(0))]
    RemotePackageDB(#[from] RemotePackageDBError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Resolve(#[from] ResolveError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Download(#[from] DownloadSourcesAndHashError),
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
    FetchSrc(#[from] crate::operations::FetchSrcError),
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

/// Resolves the requested packages and downloads (and hashes) their sources.
async fn resolve_and_download(
    config: &Config,
    package_db: RemotePackageDB,
    install_specs: Artifacts<Vec<PackageInstallSpec>>,
) -> Result<Vec<DownloadedPackage>, VendorError> {
    let discover =
        FindPackageFromLuarocks::new(Arc::new(package_db), Arc::new(config.clone())).build();

    let Artifacts {
        regular,
        build,
        test,
    } = install_specs;
    let mut resolve =
        ResolvePackageDependencies::new(&discover, config).packages(regular.unwrap_or_default());
    if let Some(build) = build {
        resolve = resolve.build_packages(build);
    }
    if let Some(test) = test {
        resolve = resolve.test_packages(test);
    }
    let resolved = resolve.resolve().await?;

    let artifacts = DownloadSourcesAndHash::new(config)
        .resolved(resolved)
        .download_sources_and_hash()
        .await?;

    // The lockfile may contain the same package (name@version) multiple times,
    // with different constraints.
    Ok(artifacts
        .into_iter()
        .flat_map(|(_, packages)| packages)
        .flat_map(HashMap::into_values)
        .unique_by(|pkg| {
            (
                pkg.package.spec.name().clone(),
                pkg.package.spec.version().clone(),
            )
        })
        .collect())
}

/// Vendors the sources of all packages into `vendor_dir`.
async fn vendor_sources(
    vendor_dir: &Path,
    packages: &[DownloadedPackage],
    config: &Config,
) -> Result<(), VendorError> {
    futures::stream::iter(
        packages
            .iter()
            .map(|package| vendor_package_sources(vendor_dir, package)),
    )
    .buffered(config.max_jobs())
    .try_collect()
    .await
}

/// Cargo-based build backends need their Cargo dependencies vendored too.
fn cargo_dependencies(
    packages: &[DownloadedPackage],
) -> Vec<(PackageSpec, Option<PathBuf>, Vec<PathBuf>)> {
    packages
        .iter()
        .filter_map(|package| {
            let rockspec = &package.rockspec;
            match rockspec.build().current_platform().build_backend {
                Some(BuildBackendSpec::RustMlua(_) | BuildBackendSpec::RustBinary(_)) => Some((
                    package.package.spec.to_package(),
                    rockspec.source().current_platform().unpack_dir.clone(),
                    rockspec.build().current_platform().copy_directories.clone(),
                )),
                _ => None,
            }
        })
        .collect()
}

async fn do_vendor_dependencies(args: Vendor<'_>) -> Result<(), VendorError> {
    let vendor_dir = args.vendor_dir;
    let no_delete = args.no_delete.unwrap_or(false);
    let no_lock = args.no_lock.unwrap_or(false);
    let target = args.target;
    let config = args.config;

    let (package_db, install_specs) = gather_install_specs(no_lock, &target, config).await?;
    let packages = resolve_and_download(config, package_db, install_specs).await?;

    let cargo_deps = cargo_dependencies(&packages);

    if !no_delete && vendor_dir.exists() {
        fs::tokio::remove_dir_all(&vendor_dir).await?;
    }

    let vendor_dir = Arc::new(vendor_dir);
    vendor_sources(&vendor_dir, &packages, config).await?;
    vendor_target_cargo_deps(&vendor_dir, &target, config).await?;
    for (dep, unpack_dir, copy_dirs) in cargo_deps {
        vendor_package_cargo_deps(&vendor_dir, &dep, &unpack_dir, &copy_dirs, config).await?;
    }
    Ok(())
}

async fn gather_install_specs(
    no_lock: bool,
    target: &VendorTarget,
    config: &Config,
) -> Result<(RemotePackageDB, Artifacts<Vec<PackageInstallSpec>>), VendorError> {
    // Resolve against the project's lockfile if present, otherwise fall back to
    // the remote package DB (e.g. for a project that has not yet generated a lockfile).
    let package_db = match target {
        VendorTarget::Workspace(workspace) => match workspace.try_lockfile()? {
            Some(lockfile) if !no_lock => lockfile.local_pkg_locks().into(),
            _ => RemotePackageDB::from_config(config).await?,
        },
        VendorTarget::Rockspec(_) => RemotePackageDB::from_config(config).await?,
    };

    let mut install_specs: Artifacts<Vec<PackageInstallSpec>> = Artifacts::default();
    for lock_type in LockedPackageLockType::iter() {
        let specs = install_specs
            .get_mut(lock_type)
            .get_or_insert_with(Vec::new);
        match target {
            VendorTarget::Workspace(workspace) => {
                for project in workspace.members() {
                    let toml = project.toml().into_local()?;
                    push_dependencies(&lock_type, &toml, specs)?;
                    if lock_type == LockedPackageLockType::Test {
                        specs.extend(
                            toml.test()
                                .current_platform()
                                .test_dependencies(project)
                                .iter()
                                .cloned()
                                .map(|dep| {
                                    PackageInstallSpec::new(dep, EntryType::Entrypoint).build()
                                }),
                        );
                    }
                }
            }
            VendorTarget::Rockspec(rockspec) => {
                push_dependencies(&lock_type, rockspec, specs)?;
            }
        }
    }

    Ok((package_db, install_specs))
}

fn push_dependencies<R: Rockspec>(
    lock_type: &LockedPackageLockType,
    rockspec: &R,
    install_specs: &mut Vec<PackageInstallSpec>,
) -> Result<(), LocalProjectTomlValidationError> {
    let mut dependencies: Vec<PackageReq> = match lock_type {
        LockedPackageLockType::Regular => rockspec
            .dependencies()
            .current_platform()
            .iter()
            .map(|dep| dep.package_req().clone())
            .collect_vec(),
        LockedPackageLockType::Test => rockspec
            .test_dependencies()
            .current_platform()
            .iter()
            .map(|dep| dep.package_req().clone())
            .collect_vec(),
        LockedPackageLockType::Build => rockspec
            .build_dependencies()
            .current_platform()
            .iter()
            .map(|dep| dep.package_req().clone())
            .collect_vec(),
    };
    if *lock_type == LockedPackageLockType::Build {
        if let Some(backend) = luarocks_build_backend_name(rockspec) {
            dependencies.insert(0, backend.into());
        }
    }
    install_specs.extend(
        dependencies
            .into_iter()
            .unique()
            .map(|dep| PackageInstallSpec::new(dep, EntryType::Entrypoint).build())
            .collect_vec(),
    );
    Ok(())
}

/// Vendors the materialized source and rockspec of a single package.
#[tracing::instrument(
    name = "Vendoring source",
    level = "info",
    skip_all,
    fields(
        package = package.package.spec.name().to_string(),
        version = package.package.spec.version().to_string(),
    ),
)]
async fn vendor_package_sources(
    vendor_dir: &Path,
    package: &DownloadedPackage,
) -> Result<(), VendorError> {
    let rockspec = &package.rockspec;
    let name = rockspec.package();
    let version = rockspec.version();

    fs::tokio::create_dir_all(vendor_dir).await?;

    let rockspec_lua_content = rockspec
        .to_lua_remote_rockspec_string()
        .map_err(|err| VendorError::LuaRockSpec(err.to_string()))?;
    let rockspec_path = vendor_dir.join(format!("{}-{}.rockspec", name, version));
    fs::tokio::write(&rockspec_path, rockspec_lua_content).await?;

    let source_path = vendor_dir.join(format!("{}@{}", name, version));
    match &package.artifact {
        // A fully materialized source tree.
        PackageSource::SourceTree(dir) => {
            fs::tokio::remove_dir_all(&source_path).await.ok();
            fs::tokio::copy_dir_all(dir.path(), &source_path).await?;
        }
        // A pre-built binary rock.
        PackageSource::PackedRock(bytes) => {
            let rock_path = vendor_dir.join(format!("{}@{}.rock", name, version));
            let mut file = fs::tokio::create(&rock_path).await?;
            file.write_all(bytes)
                .await
                .map_err(|source| fs::FsError::Write {
                    path: rock_path,
                    source,
                })?;
        }
    }

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
                    &resolve_source_dir(
                        temp_dir.path(),
                        rockspec.source().current_platform().unpack_dir.as_deref(),
                        &rockspec.build().current_platform().copy_directories,
                    )?,
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
    copy_dirs: &[PathBuf],
    config: &Config,
) -> Result<(), VendorError> {
    let source_dir = vendor_dir.join(format!("{}@{}", package.name(), package.version()));
    if source_dir.is_dir() {
        cargo_vendor(
            vendor_dir,
            &resolve_source_dir(&source_dir, unpack_dir.as_deref(), copy_dirs)?,
            config,
        )
        .await?;
    } else if source_dir.is_file() {
        // The vendored source is an archive; extract it so `cargo vendor` can
        // read its `Cargo.toml`. Keep the temp dir alive while cargo runs.
        let temp_dir = fs::tempfile::tempdir()?;
        extract_source_archive(&source_dir, unpack_dir.as_deref(), temp_dir.path()).await?;
        cargo_vendor(
            vendor_dir,
            &resolve_source_dir(temp_dir.path(), unpack_dir.as_deref(), copy_dirs)?,
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use bytes::Bytes;
    use tempfile::TempDir;

    use super::*;
    use crate::{
        config::ConfigBuilder,
        lockfile::{LockConstraint, LockedPackage, LockedPackageSpec, OptState, PinnedState},
        operations::unpack_rockspec,
        remote_package_source::RemotePackageSource,
    };
    use assert_fs::prelude::PathCopy;

    fn test_config() -> Config {
        ConfigBuilder::new()
            .unwrap()
            .lua_version(Some(crate::lua_version::LuaVersion::Lua51))
            .build()
            .unwrap()
    }

    fn fixture_bytes(name: &str) -> Bytes {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test")
            .join(name);
        Bytes::from(std::fs::read(path).unwrap())
    }

    /// Builds a `DownloadedPackage` for a materialized source tree.
    fn make_downloaded_package(
        rockspec: RemoteLuaRockspec,
        source: RemotePackageSource,
        artifact: PackageSource,
    ) -> DownloadedPackage {
        let spec = LockedPackageSpec::new(
            rockspec.package(),
            rockspec.version(),
            LockConstraint::Unconstrained,
            Vec::new(),
            Vec::new(),
            &PinnedState::Unpinned,
            &OptState::Required,
        );
        let package = LockedPackage::new(
            spec,
            source,
            None,
            crate::lockfile::LockedPackageHashes {
                rockspec: "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
                    .parse()
                    .unwrap(),
                source: "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
                    .parse()
                    .unwrap(),
            },
        );
        DownloadedPackage {
            package,
            rockspec,
            entry_type: EntryType::Entrypoint,
            artifact,
        }
    }

    /// Vendors a package with a materialized source tree: the rockspec and the
    /// source tree directory must both be written.
    #[tokio::test]
    async fn vendor_source_tree_writes_rockspec_and_source_dir() {
        let vendor_dir = assert_fs::TempDir::new().unwrap();

        // A source tree that has already been materialized by the pipeline.
        let src_dir = TempDir::new().unwrap();
        std::fs::write(src_dir.path().join("hello.lua"), "return 'hello'").unwrap();
        let rockspec = RemoteLuaRockspec::from_package_and_source_spec(
            "rockspec-only-url@1.0.0".parse().unwrap(),
            crate::lua_rockspec::RockSourceSpec::File(src_dir.path().to_path_buf()),
        );
        let package = make_downloaded_package(
            rockspec,
            RemotePackageSource::RockspecContent(String::new()),
            PackageSource::SourceTree(src_dir),
        );

        vendor_package_sources(vendor_dir.path(), &package)
            .await
            .unwrap();

        let rockspec_file = vendor_dir.path().join("rockspec-only-url-1.0.0-1.rockspec");
        assert!(rockspec_file.is_file(), "rockspec not vendored");
        let rockspec_content = std::fs::read_to_string(&rockspec_file).unwrap();
        assert!(
            rockspec_content.contains("rockspec-only-url"),
            "vendored rockspec is not for rockspec-only-url:\n{rockspec_content}"
        );

        let source_path = vendor_dir.path().join("rockspec-only-url@1.0.0-1");
        assert!(source_path.is_dir(), "source tree not vendored");
        assert!(
            source_path.join("hello.lua").is_file(),
            "source tree contents not vendored"
        );
    }

    /// Vendors a binary rock: both the packed `.rock` and its rockspec must be written.
    #[tokio::test]
    async fn vendor_binary_rock_writes_rock_and_rockspec() {
        let vendor_dir = assert_fs::TempDir::new().unwrap();

        let bytes = fixture_bytes("toml-edit-0.6.0-1.linux-x86_64.rock");
        let rock = crate::operations::DownloadedPackedRockBytes {
            name: "toml-edit".into(),
            version: "0.6.0-1".parse().unwrap(),
            bytes: bytes.clone(),
            file_name: "toml-edit-0.6.0-1.linux-x86_64.rock".into(),
            url: "https://example.org/toml-edit-0.6.0-1.linux-x86_64.rock"
                .parse()
                .unwrap(),
        };
        let rockspec = unpack_rockspec(&rock).await.unwrap();
        let package = make_downloaded_package(
            rockspec,
            RemotePackageSource::LuarocksBinaryRock("https://example.org/".parse().unwrap()),
            PackageSource::PackedRock(bytes),
        );

        vendor_package_sources(vendor_dir.path(), &package)
            .await
            .unwrap();

        assert!(
            vendor_dir.path().join("toml-edit@0.6.0-1.rock").is_file(),
            "packed rock not vendored"
        );
        let rockspec_file = vendor_dir.path().join("toml-edit-0.6.0-1.rockspec");
        assert!(rockspec_file.is_file(), "rockspec not vendored");
        let rockspec_content = std::fs::read_to_string(&rockspec_file).unwrap();
        assert!(
            rockspec_content.contains("package = ") && rockspec_content.contains("toml-edit"),
            "vendored rockspec is not valid:\n{rockspec_content}"
        );
    }

    /// A pre-existing vendored source directory must be replaced, not merged.
    #[tokio::test]
    async fn vendor_source_tree_replaces_stale_source_dir() {
        let vendor_dir = assert_fs::TempDir::new().unwrap();
        let stale_dir = vendor_dir.path().join("rockspec-only-url@1.0.0-1");
        std::fs::create_dir_all(&stale_dir).unwrap();
        std::fs::write(stale_dir.join("stale.lua"), "return 'stale'").unwrap();

        let src_dir = TempDir::new().unwrap();
        std::fs::write(src_dir.path().join("hello.lua"), "return 'hello'").unwrap();
        let rockspec = RemoteLuaRockspec::from_package_and_source_spec(
            "rockspec-only-url@1.0.0".parse().unwrap(),
            crate::lua_rockspec::RockSourceSpec::File(src_dir.path().to_path_buf()),
        );
        let package = make_downloaded_package(
            rockspec,
            RemotePackageSource::RockspecContent(String::new()),
            PackageSource::SourceTree(src_dir),
        );

        vendor_package_sources(vendor_dir.path(), &package)
            .await
            .unwrap();

        assert!(
            !stale_dir.join("stale.lua").exists(),
            "stale vendored source was not replaced"
        );
        assert!(stale_dir.join("hello.lua").is_file());
    }

    /// `no_delete` must be respected: if the vendor dir exists and `no_delete` is
    /// set, pre-existing contents are preserved.
    #[tokio::test]
    async fn vendor_workspace_no_delete_preserves_existing_contents() {
        let sample_project_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample-projects/busted-with-lockfile/");
        let temp_dir = assert_fs::TempDir::new().unwrap();
        temp_dir
            .copy_from(&sample_project_dir, &["**"])
            .expect("failed to copy sample project");
        let workspace = Workspace::from_exact(temp_dir.path()).unwrap().unwrap();
        let config = test_config();
        let vendor_dir = assert_fs::TempDir::new().unwrap();
        let marker = vendor_dir.path().join("EXISTING_FILE");
        std::fs::write(&marker, "keep me").unwrap();

        Vendor::new()
            .target(VendorTarget::Workspace(workspace))
            .vendor_dir(vendor_dir.to_path_buf())
            .no_delete(true)
            .config(&config)
            .vendor_dependencies()
            .await
            .unwrap();

        assert!(
            marker.is_file(),
            "no_delete should have preserved existing contents"
        );
    }
}
