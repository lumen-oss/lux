use std::collections::HashMap;
use std::io;
use std::io::Cursor;
use std::path::PathBuf;

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
        LockedPackage, LockedPackageHashes, LockedPackageId, LockedPackageSpec,
        RemotePackageSourceUrl,
    },
    lua_rockspec::RemoteLuaRockspec,
    operations::{FetchSrc, RemotePackageSourceMetadata, unpack_src_rock},
    remote_package_source::RemotePackageSource,
    rockspec::Rockspec,
    tree::EntryType,
};

use super::{
    Artifacts,
    resolve::{ResolvedArtifacts, ResolvedPackage},
};

#[derive(Clone, Debug)]
pub(crate) enum PackageSource {
    SourceDir(PathBuf),
    PackedRock(Bytes),
}

pub(crate) struct DownloadedPackage {
    pub(crate) package: LockedPackage,
    pub(crate) rockspec: RemoteLuaRockspec,
    pub(crate) entry_type: EntryType,
    pub(crate) artifact: PackageSource,
    pub(crate) temp_dir: Option<TempDir>,
}

pub(crate) type DownloadSourcesAndHashArtifacts =
    Artifacts<HashMap<LockedPackageId, DownloadedPackage>>;

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum DownloadSourcesAndHashError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] FsError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Build(#[from] Box<BuildError>),
    #[error("failed to hash the source for '{0}'")]
    Hash(String, #[source] io::Error),
    #[error("missing source url for '{0}'")]
    MissingSourceUrl(String),
}

impl From<BuildError> for DownloadSourcesAndHashError {
    fn from(source: BuildError) -> Self {
        Self::Build(Box::new(source))
    }
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

        // Preserve which sections were requested (even if empty), then run every
        // download as a single bounded-concurrency stream and fold the results
        // back into their sections. `buffered` keeps at most `max_jobs`
        // downloads in flight; hashing is already `spawn_blocking`, so it runs
        // across the blocking thread pool.
        let artifacts: DownloadSourcesAndHashArtifacts = args
            .resolved
            .iter()
            .map(|(section, packages)| (section, packages.map(|_| HashMap::new())))
            .collect();

        let jobs = args.resolved.into_iter().flat_map(|(section, packages)| {
            packages
                .into_iter()
                .flatten()
                .map(move |(id, package)| (section, id, package))
        });

        futures::stream::iter(jobs)
            .map(|(section, id, package)| async move {
                download_sources_and_hash(config, package)
                    .await
                    .map(|package| (section, id, package))
            })
            .buffered(config.max_jobs())
            .try_fold(
                artifacts,
                |mut artifacts, (section, id, package)| async move {
                    artifacts
                        .get_mut(section)
                        .get_or_insert_with(HashMap::new)
                        .insert(id, package);
                    Ok(artifacts)
                },
            )
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

    let name = spec.name().to_string();

    let (package, artifact, temp_dir) = match artifact {
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
                let package = LockedPackage::new(
                    spec,
                    source,
                    Some(RemotePackageSourceUrl::Url { url: binary_url }),
                    hashes,
                );
                (package, PackageSource::PackedRock(bytes), None)
            }
            _ => {
                let source_url = source_url
                    .clone()
                    .ok_or_else(|| DownloadSourcesAndHashError::MissingSourceUrl(name.clone()))?;
                let (package, dir) = fetch_and_hash_source(
                    &rockspec,
                    spec,
                    Some(RemotePackageSourceSpec::SrcRock(SrcRockSource {
                        bytes,
                        source_url,
                    })),
                    Some(source),
                    config,
                )
                .await?;
                let artifact = PackageSource::SourceDir(dir.path().to_path_buf());
                (package, artifact, Some(dir))
            }
        },
        Some(PackageSource::SourceDir(path)) => {
            let rockspec_hash = rockspec
                .hash()
                .await
                .map_err(|err| DownloadSourcesAndHashError::Hash(name.clone(), err))?;
            let source_hash = path
                .hash()
                .await
                .map_err(|err| DownloadSourcesAndHashError::Hash(name.clone(), err))?;
            let hashes = LockedPackageHashes {
                rockspec: rockspec_hash,
                source: source_hash,
            };
            let package = LockedPackage::new(spec, source, source_url, hashes);
            (package, PackageSource::SourceDir(path), None)
        }
        None => {
            let (package, dir) = fetch_and_hash_source(
                &rockspec,
                spec,
                Some(RemotePackageSourceSpec::RockSpec(source_url)),
                Some(source),
                config,
            )
            .await?;
            let artifact = PackageSource::SourceDir(dir.path().to_path_buf());
            (package, artifact, Some(dir))
        }
    };

    Ok(DownloadedPackage {
        package,
        rockspec,
        entry_type,
        artifact,
        temp_dir,
    })
}

pub(crate) async fn fetch_and_hash_source<R: Rockspec + HasIntegrity>(
    rockspec: &R,
    spec: LockedPackageSpec,
    source_spec: Option<RemotePackageSourceSpec>,
    source: Option<RemotePackageSource>,
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

    let package = LockedPackage::new(
        spec,
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

    Ok((package, temp_dir))
}
