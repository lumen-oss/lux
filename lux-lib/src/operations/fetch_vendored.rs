use std::{
    io,
    path::{Path, PathBuf},
};

use bon::Builder;
use path_slash::PathExt;
use thiserror::Error;

use crate::{
    lockfile::RemotePackageSourceUrl,
    lua_rockspec::{LuaRockspecError, RemoteLuaRockspec},
    operations::{DownloadedRockspec, RemoteRockDownload},
    package::{PackageReq, PackageSpec},
    progress::{Progress, ProgressBar},
    remote_package_db::{RemotePackageDB, SearchError},
    remote_package_source::RemotePackageSource,
};

/// Fetch a vendored rock from `<vendor_dir>`
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(crate) struct FetchVendored<'a> {
    vendor_dir: &'a Path,
    package: &'a PackageReq,
    package_db: &'a RemotePackageDB,
    progress: &'a Progress<ProgressBar>,
}

#[derive(Error, Debug)]
pub enum FetchVendoredError {
    #[error(transparent)]
    Search(#[from] SearchError),
    #[error("could not find a vendored RockSpec for package {0} in vendor directory {1}.")]
    RockspecNotFound(PackageSpec, String),
    #[error("could not read vendored RockSpec content at {rockspec_path}:\n{err}")]
    ReadRockspecContent {
        rockspec_path: String,
        err: io::Error,
    },
    #[error("could not parse vendored RockSpec {rockspec_path}:\n{err}")]
    ParseRockspec {
        rockspec_path: String,
        err: LuaRockspecError,
    },
    #[error("could not find a source or .rock archive for {0} in vendor directory {1}.")]
    PackageNotFound(PackageSpec, String),
    #[error("unable to read binary rock: {0}:\n{1}")]
    ReadBinaryRock(String, io::Error),
}

#[derive(Debug, PartialEq, Eq)]
enum VendoredPackage {
    Source(PathBuf),
    BinaryRock(PathBuf),
}

impl<State> FetchVendoredBuilder<'_, State>
where
    State: fetch_vendored_builder::State + fetch_vendored_builder::IsComplete,
{
    pub(crate) async fn fetch_vendored_rock(
        self,
    ) -> Result<RemoteRockDownload, FetchVendoredError> {
        do_fetch_vendored_rock(self._build()).await
    }
}

async fn do_fetch_vendored_rock(
    args: FetchVendored<'_>,
) -> Result<RemoteRockDownload, FetchVendoredError> {
    let vendor_dir = args.vendor_dir;
    let package = args.package;
    let package_db = args.package_db;
    let progress = args.progress;
    let package_spec = package_db.find(package, None, progress)?.package;
    let rockspec = load_vendored_rockspec(vendor_dir, &package_spec).await?;
    match load_vendored_package(vendor_dir, &package_spec)? {
        VendoredPackage::Source(path) => Ok(RemoteRockDownload::RockspecOnly {
            rockspec_download: DownloadedRockspec {
                rockspec,
                source: RemotePackageSource::Local,
                source_url: Some(RemotePackageSourceUrl::File { path }),
            },
        }),
        VendoredPackage::BinaryRock(path) => {
            let packed_rock = tokio::fs::read(&path)
                .await
                .map_err(|err| {
                    FetchVendoredError::ReadBinaryRock(path.to_slash_lossy().to_string(), err)
                })?
                .into();
            Ok(RemoteRockDownload::BinaryRock {
                rockspec_download: DownloadedRockspec {
                    rockspec,
                    source: RemotePackageSource::Local,
                    source_url: None,
                },
                packed_rock,
            })
        }
    }
}

/// Load the vendored RockSpec for a package, expecting one to be present.
async fn load_vendored_rockspec(
    vendor_dir: &Path,
    package: &PackageSpec,
) -> Result<RemoteLuaRockspec, FetchVendoredError> {
    let rockspec_path =
        vendor_dir.join(format!("{}-{}.rockspec", package.name(), package.version()));
    if !rockspec_path.is_file() {
        return Err(FetchVendoredError::RockspecNotFound(
            package.clone(),
            vendor_dir.to_slash_lossy().to_string(),
        ));
    }
    let rockspec_content = tokio::fs::read_to_string(&rockspec_path)
        .await
        .map_err(|err| FetchVendoredError::ReadRockspecContent {
            rockspec_path: rockspec_path.to_slash_lossy().to_string(),
            err,
        })?;
    let rockspec = RemoteLuaRockspec::new(&rockspec_content).map_err(|err| {
        FetchVendoredError::ParseRockspec {
            rockspec_path: rockspec_path.to_slash_lossy().to_string(),
            err,
        }
    })?;
    Ok(rockspec)
}

/// Load a vendored package, expecting either a source or a .rock archive to be present.
#[allow(clippy::result_large_err)] // This is ok because it's just a Deserialize helper
fn load_vendored_package(
    vendor_dir: &Path,
    package: &PackageSpec,
) -> Result<VendoredPackage, FetchVendoredError> {
    let source_dir = vendor_dir.join(format!("{}@{}", package.name(), package.version()));
    if source_dir.is_dir() {
        Ok(VendoredPackage::Source(source_dir.clone()))
    } else {
        let rock_path = vendor_dir.join(format!("{}@{}.rock", package.name(), package.version()));
        if rock_path.is_file() {
            Ok(VendoredPackage::BinaryRock(rock_path.clone()))
        } else {
            Err(FetchVendoredError::PackageNotFound(
                package.clone(),
                vendor_dir.to_slash_lossy().to_string(),
            ))
        }
    }
}

#[cfg(test)]
mod test {
    use assert_fs::{
        prelude::{FileTouch, FileWriteStr, PathChild, PathCreateDir},
        TempDir,
    };

    use super::*;

    #[tokio::test]
    async fn test_load_vendored_rockspec() {
        let vendor_dir = TempDir::new().unwrap();
        let foo_rockspec = vendor_dir.child("foo-1.0.0-1.rockspec");
        foo_rockspec.touch().unwrap();
        foo_rockspec
            .write_str(
                r#"
        package = 'foo'
        version = '1.0.0-1'
        source = {
            url = 'https://github.com/lumen-oss/foo/archive/1.0.0/foo.zip',
        }
        "#,
            )
            .unwrap();
        let package_spec = PackageSpec::new("foo".into(), "1.0.0-1".parse().unwrap());
        let rockspec = load_vendored_rockspec(vendor_dir.path(), &package_spec).await;
        assert!(rockspec.is_ok());
    }

    #[tokio::test]
    async fn test_load_vendored_package() {
        let vendor_dir = TempDir::new().unwrap();
        let foo_dir = vendor_dir.child("foo@1.0.0-1");
        foo_dir.create_dir_all().unwrap();
        let package_spec = PackageSpec::new("foo".into(), "1.0.0-1".parse().unwrap());
        assert!(matches!(
            load_vendored_package(vendor_dir.path(), &package_spec),
            Ok(VendoredPackage::Source { .. })
        ));

        let bar_rock = vendor_dir.child("bar@1.0.0-1.rock");
        bar_rock.touch().unwrap();
        let package_spec = PackageSpec::new("bar".into(), "1.0.0-1".parse().unwrap());
        assert!(matches!(
            load_vendored_package(vendor_dir.path(), &package_spec),
            Ok(VendoredPackage::BinaryRock { .. })
        ));
    }
}
