use bon::Builder;
use git2::build::RepoBuilder;
use git2::FetchOptions;
use remove_dir_all::remove_dir_all;
use ssri::Integrity;
use std::fs::File;
use std::io;
use std::io::Cursor;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use thiserror::Error;

use crate::build::utils::recursive_copy_dir;
use crate::config::Config;
use crate::git::url::RemoteGitUrlParseError;
use crate::git::GitSource;
use crate::hash::HasIntegrity;
use crate::lockfile::RemotePackageSourceUrl;
use crate::lua_rockspec::RockSourceSpec;
use crate::operations;
use crate::package::PackageSpec;
use crate::progress::Progress;
use crate::progress::ProgressBar;
use crate::rockspec::Rockspec;

use super::DownloadSrcRockError;
use super::UnpackError;

/// A rocks package source fetcher, providing fine-grained control
/// over how a package should be fetched.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct FetchSrc<'a, R: Rockspec> {
    #[builder(start_fn)]
    dest_dir: &'a Path,
    #[builder(start_fn)]
    rockspec: &'a R,
    #[builder(start_fn)]
    config: &'a Config,
    #[builder(start_fn)]
    progress: &'a Progress<ProgressBar>,
    #[builder(setters(vis = "pub(crate)"))]
    source_url: Option<RemotePackageSourceUrl>,
}

#[derive(Debug)]
pub(crate) struct RemotePackageSourceMetadata {
    pub hash: Integrity,
    pub source_url: RemotePackageSourceUrl,
}

impl RemotePackageSourceMetadata {
    pub(crate) fn archive_name(&self) -> Option<PathBuf> {
        self.source_url.archive_name()
    }
}

impl<R: Rockspec, State> FetchSrcBuilder<'_, R, State>
where
    State: fetch_src_builder::State + fetch_src_builder::IsComplete,
{
    /// Fetch and unpack the source into the `dest_dir`.
    pub async fn fetch(self) -> Result<(), FetchSrcError> {
        self.fetch_internal().await?;
        Ok(())
    }

    /// Fetch and unpack the source into the `dest_dir`,
    /// returning the source `Integrity`.
    pub(crate) async fn fetch_internal(self) -> Result<RemotePackageSourceMetadata, FetchSrcError> {
        let fetch = self._build();
        match do_fetch_src(&fetch).await {
            Err(err) => match &fetch.rockspec.source().current_platform().source_spec {
                RockSourceSpec::Git(_) | RockSourceSpec::Url(_) => {
                    let package = PackageSpec::new(
                        fetch.rockspec.package().clone(),
                        fetch.rockspec.version().clone(),
                    );
                    fetch.progress.map(|p| {
                        p.println(format!(
                            "⚠️ WARNING: Failed to fetch source for {}: {}",
                            &package, err
                        ))
                    });
                    fetch.progress.map(|p| {
                        p.println(format!(
                            "⚠️ Falling back to searching for a .src.rock archive on {}",
                            fetch.config.server()
                        ))
                    });
                    let metadata =
                        FetchSrcRock::new(&package, fetch.dest_dir, fetch.config, fetch.progress)
                            .fetch()
                            .await?;
                    Ok(metadata)
                }
                RockSourceSpec::File(_) => Err(err),
            },
            Ok(metadata) => Ok(metadata),
        }
    }
}

#[derive(Error, Debug)]
pub enum FetchSrcError {
    #[error("failed to clone rock source:\n{0}")]
    GitClone(#[from] git2::Error),
    #[error("failed to parse git URL:\n{0}")]
    GitUrlParse(#[from] RemoteGitUrlParseError),
    #[error(transparent)]
    Request(#[from] reqwest::Error),
    #[error(transparent)]
    Unpack(#[from] UnpackError),
    #[error(transparent)]
    FetchSrcRock(#[from] FetchSrcRockError),
    #[error("unable to remove the '.git' directory:\n{0}")]
    CleanGitDir(io::Error),
    #[error("unable to compute hash:\n{0}")]
    Hash(io::Error),
    #[error("unable to copy {src} to {dest}:\n{err}")]
    CopyDir {
        src: PathBuf,
        dest: PathBuf,
        err: io::Error,
    },
    #[error("unable to open {file}:\n{err}")]
    FileOpen { file: PathBuf, err: io::Error },
    #[error("unable to read {file}:\n{err}")]
    FileRead { file: PathBuf, err: io::Error },
}

/// A rocks package source fetcher, providing fine-grained control
/// over how a package should be fetched.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
struct FetchSrcRock<'a> {
    #[builder(start_fn)]
    package: &'a PackageSpec,
    #[builder(start_fn)]
    dest_dir: &'a Path,
    #[builder(start_fn)]
    config: &'a Config,
    #[builder(start_fn)]
    progress: &'a Progress<ProgressBar>,
}

impl<State> FetchSrcRockBuilder<'_, State>
where
    State: fetch_src_rock_builder::State + fetch_src_rock_builder::IsComplete,
{
    pub async fn fetch(self) -> Result<RemotePackageSourceMetadata, FetchSrcRockError> {
        do_fetch_src_rock(self._build()).await
    }
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum FetchSrcRockError {
    DownloadSrcRock(#[from] DownloadSrcRockError),
    Unpack(#[from] UnpackError),
    Io(#[from] io::Error),
}

async fn do_fetch_src<R: Rockspec>(
    fetch: &FetchSrc<'_, R>,
) -> Result<RemotePackageSourceMetadata, FetchSrcError> {
    let rockspec = fetch.rockspec;
    let rock_source = rockspec.source().current_platform();
    let progress = fetch.progress;
    let dest_dir = fetch.dest_dir;
    // prioritise lockfile source, if present
    let source_spec = match &fetch.source_url {
        Some(source_url) => match source_url {
            RemotePackageSourceUrl::Git { url, checkout_ref } => RockSourceSpec::Git(GitSource {
                url: url.parse()?,
                checkout_ref: Some(checkout_ref.clone()),
            }),
            RemotePackageSourceUrl::Url { url } => RockSourceSpec::Url(url.clone()),
            RemotePackageSourceUrl::File { path } => RockSourceSpec::File(path.clone()),
        },
        None => rock_source.source_spec.clone(),
    };
    let metadata = match &source_spec {
        RockSourceSpec::Git(git) => {
            let url = git.url.to_string();
            progress.map(|p| p.set_message(format!("🦠 Cloning {url}")));

            let mut fetch_options = FetchOptions::new();
            fetch_options.update_fetchhead(false);
            if git.checkout_ref.is_none() {
                fetch_options.depth(1);
            };
            let mut repo_builder = RepoBuilder::new();
            repo_builder.fetch_options(fetch_options);
            let repo = repo_builder.clone(&url, dest_dir)?;

            let checkout_ref = match &git.checkout_ref {
                Some(checkout_ref) => {
                    let (object, _) = repo.revparse_ext(checkout_ref)?;
                    repo.checkout_tree(&object, None)?;
                    checkout_ref.clone()
                }
                None => {
                    let head = repo.head()?;
                    let commit = head.peel_to_commit()?;
                    commit.id().to_string()
                }
            };
            // The .git directory is not deterministic
            remove_dir_all(dest_dir.join(".git")).map_err(FetchSrcError::CleanGitDir)?;
            let hash = fetch.dest_dir.hash().map_err(FetchSrcError::Hash)?;
            RemotePackageSourceMetadata {
                hash,
                source_url: RemotePackageSourceUrl::Git { url, checkout_ref },
            }
        }
        RockSourceSpec::Url(url) => {
            progress.map(|p| p.set_message(format!("📥 Downloading {}", url.to_owned())));

            let response = reqwest::Client::new()
                .get(url.clone())
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await?;
            let hash = response.hash().map_err(FetchSrcError::Hash)?;
            let file_name = url
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .and_then(|name| {
                    if name.is_empty() {
                        None
                    } else {
                        Some(name.to_string())
                    }
                })
                .unwrap_or(url.to_string());
            let cursor = Cursor::new(response);
            let mime_type = infer::get(cursor.get_ref()).map(|file_type| file_type.mime_type());
            operations::unpack::unpack(
                mime_type,
                cursor,
                rock_source.unpack_dir.is_none(),
                file_name,
                dest_dir,
                progress,
            )
            .await?;
            RemotePackageSourceMetadata {
                hash,
                source_url: RemotePackageSourceUrl::Url { url: url.clone() },
            }
        }
        RockSourceSpec::File(path) => {
            let hash = if path.is_dir() {
                progress.map(|p| p.set_message(format!("📋 Copying {}", path.display())));
                recursive_copy_dir(&path.to_path_buf(), dest_dir)
                    .await
                    .map_err(|err| FetchSrcError::CopyDir {
                        src: path.to_path_buf(),
                        dest: dest_dir.to_path_buf(),
                        err,
                    })?;
                progress.map(|p| p.finish_and_clear());
                dest_dir.hash().map_err(FetchSrcError::Hash)?
            } else {
                let mut file = File::open(path).map_err(|err| FetchSrcError::FileOpen {
                    file: path.clone(),
                    err,
                })?;
                let mut buffer = Vec::new();
                file.read_to_end(&mut buffer)
                    .map_err(|err| FetchSrcError::FileRead {
                        file: path.clone(),
                        err,
                    })?;
                let mime_type = infer::get(&buffer).map(|file_type| file_type.mime_type());
                let file_name = path
                    .file_name()
                    .map(|os_str| os_str.to_string_lossy())
                    .unwrap_or(path.to_string_lossy())
                    .to_string();
                operations::unpack::unpack(
                    mime_type,
                    file,
                    rock_source.unpack_dir.is_none(),
                    file_name,
                    dest_dir,
                    progress,
                )
                .await?;
                path.hash().map_err(FetchSrcError::Hash)?
            };
            RemotePackageSourceMetadata {
                hash,
                source_url: RemotePackageSourceUrl::File { path: path.clone() },
            }
        }
    };
    Ok(metadata)
}

async fn do_fetch_src_rock(
    fetch: FetchSrcRock<'_>,
) -> Result<RemotePackageSourceMetadata, FetchSrcRockError> {
    let package = fetch.package;
    let dest_dir = fetch.dest_dir;
    let config = fetch.config;
    let progress = fetch.progress;
    let src_rock = operations::download_src_rock(package, config.server(), progress).await?;
    let hash = src_rock.bytes.hash()?;
    let cursor = Cursor::new(src_rock.bytes);
    let mime_type = infer::get(cursor.get_ref()).map(|file_type| file_type.mime_type());
    operations::unpack::unpack(
        mime_type,
        cursor,
        true,
        src_rock.file_name,
        dest_dir,
        progress,
    )
    .await?;
    Ok(RemotePackageSourceMetadata {
        hash,
        source_url: RemotePackageSourceUrl::Url { url: src_rock.url },
    })
}
