use std::collections::HashMap;
use std::io;
use std::io::Cursor;

use bon::Builder;
use bytes::Bytes;
use futures::{StreamExt, TryStreamExt};
use miette::Diagnostic;
use tempfile::TempDir;
use thiserror::Error;

use crate::{
    build::{BuildError, RemotePackageSourceSpec, SrcRockSource},
    config::Config,
    fs::{self, FsError},
    hash::HasIntegrity,
    lockfile::{
        LockConstraint, LockedPackage, LockedPackageHashes, LockedPackageId, OptState, PinnedState,
        RemotePackageSourceUrl,
    },
    lua_rockspec::RemoteLuaRockspec,
    operations::{unpack_src_rock, FetchSrc, RemotePackageSourceMetadata},
    package::PackageSpec,
    remote_package_source::RemotePackageSource,
    rockspec::Rockspec,
    tree::EntryType,
};

use super::{
    resolve::{ResolvedArtifacts, ResolvedPackage},
    Artifacts,
};

pub(crate) enum PackageSource {
    SourceTree(TempDir),
    PackedRock(Bytes),
}

pub(crate) struct DownloadedPackage {
    pub(crate) package: LockedPackage,
    pub(crate) rockspec: RemoteLuaRockspec,
    pub(crate) entry_type: EntryType,
    pub(crate) artifact: PackageSource,
}

pub(crate) type DownloadSourcesAndHashArtifacts =
    Artifacts<HashMap<LockedPackageId, DownloadedPackage>>;

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub(crate) enum DownloadSourcesAndHashError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] FsError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Build(#[from] BuildError),
    #[error("failed to hash the source for '{0}'")]
    Hash(String, #[source] io::Error),
    #[error("missing source url for '{0}'")]
    MissingSourceUrl(String),
}

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(crate) struct DownloadSourcesAndHash<'a> {
    #[builder(start_fn)]
    pub(crate) config: &'a Config,
    pub(crate) resolved: ResolvedArtifacts,
}

impl<State> DownloadSourcesAndHashBuilder<'_, State>
where
    State: download_sources_and_hash_builder::State + download_sources_and_hash_builder::IsComplete,
{
    pub(crate) async fn download_sources_and_hash(
        self,
    ) -> Result<DownloadSourcesAndHashArtifacts, DownloadSourcesAndHashError> {
        let args = self._build();
        let config = args.config;

        futures::stream::iter(args.resolved)
            .then(|(section, packages)| async move {
                let packages = futures::stream::iter(packages)
                    .then(|(id, package)| async move {
                        download_sources_and_hash(config, package)
                            .await
                            .map(|package| (id, package))
                    })
                    .try_collect::<HashMap<_, _>>()
                    .await?;
                Ok((section, packages))
            })
            .try_collect()
            .await
    }
}

async fn download_sources_and_hash(
    config: &Config,
    resolved: ResolvedPackage,
) -> Result<DownloadedPackage, DownloadSourcesAndHashError> {
    let ResolvedPackage {
        spec,
        rockspec,
        source,
        source_url,
        entry_type,
        artifact,
    } = resolved;

    let constraint = spec.constraint();
    let pin = spec.pinned();
    let opt = spec.opt();
    let name = spec.name().to_string();

    let (package, artifact) = match artifact {
        Some(PackageSource::PackedRock(bytes)) => match &source {
            RemotePackageSource::LuarocksBinaryRock(url) => {
                let binary_url = url.clone();
                let rockspec_hash = rockspec
                    .hash()
                    .await
                    .map_err(|err| DownloadSourcesAndHashError::Hash(name.clone(), err))?;
                let source_hash = bytes
                    .hash()
                    .await
                    .map_err(|err| DownloadSourcesAndHashError::Hash(name.clone(), err))?;
                let hashes = LockedPackageHashes {
                    rockspec: rockspec_hash,
                    source: source_hash,
                };
                let package = LockedPackage {
                    spec,
                    source,
                    source_url: Some(RemotePackageSourceUrl::Url { url: binary_url }),
                    hashes,
                };
                (package, PackageSource::PackedRock(bytes))
            }
            _ => {
                let source_url = source_url
                    .clone()
                    .ok_or_else(|| DownloadSourcesAndHashError::MissingSourceUrl(name.clone()))?;
                let (mut package, dir) = fetch_and_hash_source(
                    &rockspec,
                    Some(RemotePackageSourceSpec::SrcRock(SrcRockSource {
                        bytes,
                        source_url,
                    })),
                    Some(source),
                    constraint,
                    pin,
                    opt,
                    config,
                )
                .await?;
                package.spec = spec;
                (package, PackageSource::SourceTree(dir))
            }
        },
        Some(PackageSource::SourceTree(dir)) => {
            let rockspec_hash = rockspec
                .hash()
                .await
                .map_err(|err| DownloadSourcesAndHashError::Hash(name.clone(), err))?;
            let source_hash = dir
                .path()
                .hash()
                .await
                .map_err(|err| DownloadSourcesAndHashError::Hash(name.clone(), err))?;
            let hashes = LockedPackageHashes {
                rockspec: rockspec_hash,
                source: source_hash,
            };
            let package = LockedPackage {
                spec,
                source,
                source_url,
                hashes,
            };
            (package, PackageSource::SourceTree(dir))
        }
        None => {
            let (mut package, dir) = fetch_and_hash_source(
                &rockspec,
                Some(RemotePackageSourceSpec::RockSpec(source_url)),
                Some(source),
                constraint,
                pin,
                opt,
                config,
            )
            .await?;
            package.spec = spec;
            (package, PackageSource::SourceTree(dir))
        }
    };

    Ok(DownloadedPackage {
        package,
        rockspec,
        entry_type,
        artifact,
    })
}

pub(crate) async fn fetch_and_hash_source<R: Rockspec + HasIntegrity>(
    rockspec: &R,
    source_spec: Option<RemotePackageSourceSpec>,
    source: Option<RemotePackageSource>,
    constraint: LockConstraint,
    pin: PinnedState,
    opt: OptState,
    config: &Config,
) -> Result<(LockedPackage, TempDir), BuildError> {
    let temp_dir = fs::tempfile::tempdir()?;

    let source_metadata = match source_spec {
        Some(RemotePackageSourceSpec::SrcRock(SrcRockSource { bytes, source_url })) => {
            // FIXME(vhyrro): This shouldn't be in this function, or the function should be named differently.
            // "fetching" has nothing to do with unpacking.
            let hash = bytes.hash().await?;
            let cursor = Cursor::new(bytes);
            unpack_src_rock(cursor, temp_dir.path().to_path_buf())
                .await
                .map_err(BuildError::UnpackSrcRock)?;
            RemotePackageSourceMetadata { hash, source_url }
        }
        Some(RemotePackageSourceSpec::RockSpec(source_url)) => {
            FetchSrc::new(temp_dir.path(), rockspec, config)
                .maybe_source_url(source_url)
                .fetch_internal()
                .await?
        }
        None => {
            FetchSrc::new(temp_dir.path(), rockspec, config)
                .fetch_internal()
                .await?
        }
    };

    let hashes = LockedPackageHashes {
        rockspec: rockspec.hash().await?,
        source: source_metadata.hash.clone(),
    };

    let mut package = LockedPackage::from(
        &PackageSpec::new(rockspec.package().clone(), rockspec.version().clone()),
        constraint,
        rockspec.binaries(),
        source
            .map(Result::Ok)
            .unwrap_or_else(|| {
                rockspec
                    .to_lua_remote_rockspec_string()
                    .map(RemotePackageSource::RockspecContent)
            })
            .unwrap_or(RemotePackageSource::Local),
        Some(source_metadata.source_url.clone()),
        hashes,
    );
    // FIXME(vhyrro): We should reconsider our constructors if we have to set
    // these variants ourselves. Either put these in a different place, or resolve
    // these in an earlier step.
    package.spec.pinned = pin;
    package.spec.opt = opt;

    Ok((package, temp_dir))
}
