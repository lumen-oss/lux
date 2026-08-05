use crate::build::utils::recursive_copy_dir;
use crate::config::Config;
use crate::git::url::RemoteGitUrlParseError;
use crate::git::GitSource;
use crate::hash::HasIntegrity;
use crate::lockfile::RemotePackageSourceUrl;
use crate::lua_rockspec::RockSourceSpec;
use crate::package::PackageSpec;
use crate::rockspec::Rockspec;
use crate::{fs, operations};
use auth_git2::{GitAuthenticator, Prompter};
use bon::Builder;
use bytes::Bytes;
use git2::build::RepoBuilder;
use git2::{Direction, FetchOptions, RemoteCallbacks};
use miette::Diagnostic;
use remove_dir_all::remove_dir_all;
use ssri::Integrity;
use std::io;
use std::io::Cursor;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use thiserror::Error;
use tracing::span;

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
    #[builder(setters(vis = "pub(crate)"))]
    source_url: Option<RemotePackageSourceUrl>,
}

#[derive(Debug)]
pub(crate) struct RemotePackageSourceMetadata {
    pub hash: Integrity,
    pub source_url: RemotePackageSourceUrl,
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
            Err(err)
                if fetch
                    .source_url
                    .is_some_and(|url| matches!(url, RemotePackageSourceUrl::File { .. })) =>
            {
                // Don't fall back to downloading .src.rock archives if a local source was specified.
                Err(err)
            }
            Err(err) => match &fetch.rockspec.source().current_platform().source_spec {
                RockSourceSpec::Git(_) | RockSourceSpec::Url(_) => {
                    let package = PackageSpec::new(
                        fetch.rockspec.package().clone(),
                        fetch.rockspec.version().clone(),
                    );
                    let metadata = FetchSrcRock::new(&package, fetch.dest_dir, fetch.config)
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

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum FetchSrcError {
    #[error("failed to clone rock source:\n{0}")]
    #[diagnostic(help("check your network connection and verify the git URL is correct."))]
    GitClone(#[from] git2::Error),
    #[error("failed to parse git URL:\n{0}")]
    #[diagnostic(forward(0))]
    GitUrlParse(#[from] RemoteGitUrlParseError),
    #[error(transparent)]
    #[diagnostic(help("check your network connection."))]
    Request(#[from] reqwest::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Unpack(#[from] UnpackError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    FetchSrcRock(#[from] FetchSrcRockError),
    #[error("unable to remove the '.git' directory:\n{0}")]
    #[diagnostic(help(
        "check that no process is using the directory and you have write permissions."
    ))]
    CleanGitDir(io::Error),
    #[error("unable to compute hash:\n{0}")]
    Hash(io::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::FsError),
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
}

impl<State> FetchSrcRockBuilder<'_, State>
where
    State: fetch_src_rock_builder::State + fetch_src_rock_builder::IsComplete,
{
    pub async fn fetch(self) -> Result<RemotePackageSourceMetadata, FetchSrcRockError> {
        do_fetch_src_rock(self._build()).await
    }
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
#[error(transparent)]
pub enum FetchSrcRockError {
    DownloadSrcRock(#[from] DownloadSrcRockError),
    Unpack(#[from] UnpackError),
    Io(#[from] io::Error),
}

/// A no-prompt implementer for auth_git2's prompter
#[derive(Copy, Clone, Debug)]
struct NullPrompter;

impl Prompter for NullPrompter {
    fn prompt_username_password(&mut self, _: &str, _: &git2::Config) -> Option<(String, String)> {
        None
    }

    fn prompt_password(&mut self, _: &str, _: &str, _: &git2::Config) -> Option<String> {
        None
    }

    fn prompt_ssh_key_passphrase(&mut self, _: &Path, _: &git2::Config) -> Option<String> {
        None
    }
}

async fn do_fetch_src<R: Rockspec>(
    fetch: &FetchSrc<'_, R>,
) -> Result<RemotePackageSourceMetadata, FetchSrcError> {
    let rockspec = fetch.rockspec;
    let rock_source = rockspec.source().current_platform();
    let dest_dir = fetch.dest_dir;
    let config = fetch.config;
    // prioritise lockfile source, if present
    let mut source_spec = match &fetch.source_url {
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
    let span = span!(
        tracing::Level::INFO,
        "Fetching source",
        location = source_spec.to_string(),
    );
    let _enter = span.enter();

    if let Some(vendor_dir) = config.vendor_dir() {
        source_spec = match source_spec {
            // could be a project directory (not vendored) or a local source
            // or a vendored dependency that we have already resolved
            RockSourceSpec::File(_) => source_spec,
            _ => {
                let pkg_vendor_dir =
                    vendor_dir.join(format!("{}@{}", rockspec.package(), rockspec.version()));
                RockSourceSpec::File(pkg_vendor_dir)
            }
        }
    }
    let metadata = match &source_spec {
        RockSourceSpec::Git(git) => {
            let url = git.url.to_string();
            tracing::debug!(message = format!("Cloning {url}").as_str());

            let resolved_ref = resolve_remote_ref(&url, git.checkout_ref.as_deref(), config);

            if let Some(oid) = resolved_ref {
                let cache_dir = source_cache_dir(config, &url).join(oid.to_string());
                if fs::sync::read_dir(&cache_dir).is_ok_and(|mut entries| entries.next().is_some())
                {
                    tracing::debug!("using cached git source");
                    recursive_copy_dir_no_ignore(&cache_dir, dest_dir).await?;
                    let hash = fetch.dest_dir.hash().await.map_err(FetchSrcError::Hash)?;
                    let checkout_ref = git.checkout_ref.clone().unwrap_or(oid.to_string());
                    return Ok(RemotePackageSourceMetadata {
                        hash,
                        source_url: RemotePackageSourceUrl::Git { url, checkout_ref },
                    });
                }
            }
            tracing::debug!("fetching git source");

            let checkout_ref = {
                let auth = if config.no_prompt() {
                    GitAuthenticator::default()
                        .try_password_prompt(0)
                        .prompt_ssh_key_password(false)
                        .set_prompter(NullPrompter)
                } else {
                    GitAuthenticator::default()
                };
                let git_config = git2::Config::open_default()?;
                let mut callbacks = RemoteCallbacks::new();
                callbacks.credentials(auth.credentials(&git_config));
                let mut fetch_options = FetchOptions::new();
                fetch_options.update_fetchhead(false);
                fetch_options.remote_callbacks(callbacks);
                if git.checkout_ref.is_none() {
                    fetch_options.depth(1);
                };
                let mut repo_builder = RepoBuilder::new();
                repo_builder.fetch_options(fetch_options);
                let repo = repo_builder.clone(&url, dest_dir)?;

                match &git.checkout_ref {
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
                }
            };
            // The .git directory is not deterministic
            remove_dir_all(dest_dir.join(".git")).map_err(FetchSrcError::CleanGitDir)?;

            if let Some(oid) = resolved_ref {
                populate_source_cache(
                    dest_dir,
                    &source_cache_dir(config, &url).join(oid.to_string()),
                )
                .await;
            }

            let hash = fetch.dest_dir.hash().await.map_err(FetchSrcError::Hash)?;
            RemotePackageSourceMetadata {
                hash,
                source_url: RemotePackageSourceUrl::Git { url, checkout_ref },
            }
        }
        RockSourceSpec::Url(url) => {
            tracing::debug!(message = format!("📥 Downloading {url}").as_str());

            let cache_path = source_cache_dir(config, url.as_ref()).join("archive");
            let response = match fs::tokio::read(&cache_path).await {
                Ok(bytes) => {
                    tracing::debug!("using cached source archive");
                    Bytes::from(bytes)
                }
                Err(_) => {
                    tracing::debug!("fetching source archive");
                    // NOTE: We don't enforce HTTPS when fetching sources because some rockspecs
                    // have HTTP URLs in `source.url`.
                    let response = crate::reqwest::http_client(config)?
                        .get(url.clone())
                        .send()
                        .await?
                        .error_for_status()?
                        .bytes()
                        .await?;
                    write_source_cache_archive(&cache_path, &response).await;
                    response
                }
            };
            let hash = response.hash().await.map_err(FetchSrcError::Hash)?;
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
            )
            .await?;
            RemotePackageSourceMetadata {
                hash,
                source_url: RemotePackageSourceUrl::Url { url: url.clone() },
            }
        }
        RockSourceSpec::File(path) => {
            tracing::debug!(message = format!("📋 Copying {}", path.display()).as_str());

            let hash = if path.is_dir() {
                recursive_copy_dir(&path.to_path_buf(), dest_dir).await?;
                dest_dir.hash().await.map_err(FetchSrcError::Hash)?
            } else {
                let mut file = fs::sync::open(path)?;
                let mut buffer = Vec::new();
                file.read_to_end(&mut buffer)
                    .map_err(|source| fs::FsError::Read {
                        path: path.to_path_buf(),
                        source,
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
                )
                .await?;
                path.hash().await.map_err(FetchSrcError::Hash)?
            };
            RemotePackageSourceMetadata {
                hash,
                source_url: RemotePackageSourceUrl::File { path: path.clone() },
            }
        }
    };
    Ok(metadata)
}

/// Directory in which fetched sources are cached, keyed by source URL.
fn source_cache_dir(config: &Config, url: &str) -> PathBuf {
    config.cache_dir().join("sources").join(sanitize_url(url))
}

fn sanitize_url(url: &str) -> String {
    url.replace(&[':', '*', '?', '"', '<', '>', '|', '/', '\\'][..], "_")
}

/// Recursively copy a directory.
/// Unlike [`crate::build::utils::recursive_copy_dir`], this does not respect ignore files.
#[tracing::instrument(level = "trace")]
async fn recursive_copy_dir_no_ignore(src: &Path, dest: &Path) -> Result<(), fs::FsError> {
    let mut dirs: Vec<PathBuf> = vec![src.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::sync::read_dir(&dir)?.filter_map(Result::ok) {
            let entry_path = entry.path();
            let relative_path: PathBuf = pathdiff::diff_paths(&entry_path, src)
                .unwrap_or_else(|| unreachable!("diff_path with self"));
            let target = dest.join(relative_path);
            let file_type = entry.file_type().map_err(|source| fs::FsError::Read {
                path: entry_path.clone(),
                source,
            })?;
            if file_type.is_dir() {
                fs::tokio::create_dir_all(&target).await?;
                dirs.push(entry_path);
            } else if file_type.is_file() {
                if let Some(parent) = target.parent() {
                    fs::tokio::create_dir_all(parent).await?;
                }
                fs::tokio::copy(&entry_path, &target).await?;
            }
        }
    }
    Ok(())
}

/// Copy the fetched source into the cache, atomically (best-effort).
#[tracing::instrument(level = "trace")]
async fn populate_source_cache(dest_dir: &Path, cache_dir: &Path) {
    let Some(parent) = cache_dir.parent() else {
        return;
    };
    if fs::tokio::create_dir_all(parent).await.is_err() {
        return;
    }
    let temp = parent.join(format!(
        ".tmp-{}",
        cache_dir.file_name().unwrap_or_default().to_string_lossy()
    ));
    if recursive_copy_dir_no_ignore(dest_dir, &temp)
        .await
        .and_then(|_| fs::sync::rename(&temp, cache_dir))
        .is_err()
    {
        let _ = remove_dir_all(&temp);
        tracing::debug!("failed to populate the source cache");
    }
}

/// Write the downloaded archive to the cache, atomically (best-effort).
#[tracing::instrument(level = "trace")]
async fn write_source_cache_archive(path: &Path, contents: &Bytes) {
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::tokio::create_dir_all(parent).await.is_err() {
        return;
    }
    let temp = parent.join(format!(
        ".tmp-{}",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    if fs::tokio::write(&temp, contents).await.is_err()
        || fs::tokio::rename(&temp, path).await.is_err()
    {
        let _ = fs::tokio::remove_file(&temp).await;
        tracing::debug!("failed to write the source cache");
    }
}

/// Resolve a git `checkout_ref` to an immutable commit SHA without cloning,
/// using the remote's ref advertisement (equivalent to `git ls-remote`).
/// Returns `None` if the ref cannot be resolved.
#[tracing::instrument(level = "trace")]
fn resolve_remote_ref(url: &str, checkout_ref: Option<&str>, config: &Config) -> Option<git2::Oid> {
    if let Some(reference) = checkout_ref {
        if let Ok(oid) = git2::Oid::from_str(reference) {
            return Some(oid);
        }
    }
    let result: Result<git2::Oid, git2::Error> = (|| {
        let auth = if config.no_prompt() {
            GitAuthenticator::default()
                .try_password_prompt(0)
                .prompt_ssh_key_password(false)
                .set_prompter(NullPrompter)
        } else {
            GitAuthenticator::default()
        };
        let git_config = git2::Config::open_default()?;
        let mut callbacks = RemoteCallbacks::new();
        callbacks.credentials(auth.credentials(&git_config));
        let tempdir = fs::tempfile::tempdir().map_err(|err| {
            git2::Error::from_str(&format!("unable to create temporary directory: {err}"))
        })?;
        let repo = git2::Repository::init_bare(tempdir.path())?;
        let mut remote = repo.remote_anonymous(url)?;
        let connection = remote.connect_auth(Direction::Fetch, Some(callbacks), None)?;
        let refs = connection.list()?;
        let oid = match checkout_ref {
            Some(reference) => {
                let candidates = [
                    reference.to_string(),
                    format!("refs/heads/{reference}"),
                    format!("refs/tags/{reference}"),
                ];
                refs.iter()
                    .find(|head| candidates.iter().any(|c| c.as_str() == head.name()))
                    .map(|head| head.oid())
            }
            None => refs
                .iter()
                .find(|head| head.name() == "HEAD")
                .map(|head| head.oid()),
        };
        oid.ok_or_else(|| git2::Error::from_str("no matching ref advertised"))
    })();
    result.ok()
}

async fn do_fetch_src_rock(
    fetch: FetchSrcRock<'_>,
) -> Result<RemotePackageSourceMetadata, FetchSrcRockError> {
    let package = fetch.package;
    let span = span!(
        tracing::Level::INFO,
        "Fetching src.rock",
        package = package.to_string(),
    );
    let _enter = span.enter();

    let dest_dir = fetch.dest_dir;
    let config = fetch.config;
    let src_rock = operations::download_src_rock(package, config.server(), fetch.config).await?;
    let hash = src_rock.bytes.hash().await?;
    let cursor = Cursor::new(src_rock.bytes);
    let mime_type = infer::get(cursor.get_ref()).map(|file_type| file_type.mime_type());
    operations::unpack::unpack(mime_type, cursor, true, src_rock.file_name, dest_dir).await?;
    Ok(RemotePackageSourceMetadata {
        hash,
        source_url: RemotePackageSourceUrl::Url { url: src_rock.url },
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use assert_fs::prelude::*;
    use httptest::{matchers::request, responders::status_code, Expectation, Server};
    use serial_test::serial;

    use crate::config::ConfigBuilder;
    use crate::lua_rockspec::RemoteLuaRockspec;

    use super::*;

    fn source_zip() -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut cursor);
        zip.start_file("test.lua", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"return 1").unwrap();
        zip.finish().unwrap();
        cursor.into_inner()
    }

    fn test_config(cache_dir: std::path::PathBuf) -> Config {
        ConfigBuilder::new()
            .unwrap()
            .cache_dir(Some(cache_dir))
            .no_progress(Some(true))
            .build()
            .unwrap()
    }

    #[tokio::test]
    #[serial]
    async fn fetch_url_source_hits_cache() {
        let cache_dir = assert_fs::TempDir::new().unwrap().to_path_buf();
        let server = Server::run();
        server.expect(
            // We only allow one call, so the second one has to hit the cache or fail
            Expectation::matching(request::path("/source.zip"))
                .times(1)
                .respond_with(status_code(200).body(source_zip())),
        );
        let url = server.url_str("/source.zip");
        let rockspec = RemoteLuaRockspec::new(&format!(
            r#"
rockspec_format = '3.0'
package = 'cached-package'
version = '1.0-1'
description = {{ summary = 'test package' }}
source = {{ url = '{url}', dir = 'source' }}
build = {{ type = 'builtin', modules = {{ ['test'] = 'source/test.lua' }} }}
"#
        ))
        .unwrap();
        let config = test_config(cache_dir);

        let dest_dir = assert_fs::TempDir::new().unwrap();
        let first = FetchSrc::new(dest_dir.path(), &rockspec, &config)
            .fetch_internal()
            .await
            .unwrap();

        let dest_dir = assert_fs::TempDir::new().unwrap();
        let second = FetchSrc::new(dest_dir.path(), &rockspec, &config)
            .fetch_internal()
            .await
            .unwrap();

        assert_eq!(first.hash, second.hash);
    }

    #[tokio::test]
    #[serial]
    async fn resolves_git_ref_to_oid_without_cloning() {
        let repo_dir = assert_fs::TempDir::new().unwrap();
        let bare_path = repo_dir.path().join("repo.git");
        let (commit_id, default_branch) = {
            let bare = git2::Repository::init_bare(&bare_path).unwrap();
            let signature = git2::Signature::now("test", "test@example.com").unwrap();
            let tree_id = {
                let mut builder = bare.treebuilder(None).unwrap();
                let blob = bare.blob(b"hello").unwrap();
                builder.insert("hello.txt", blob, 0o100644).unwrap();
                builder.write().unwrap()
            };
            let tree = bare.find_tree(tree_id).unwrap();
            let commit_id = bare
                .commit(Some("HEAD"), &signature, &signature, "init", &tree, &[])
                .unwrap();
            let head_ref = bare.find_reference("HEAD").unwrap();
            let default_branch = head_ref
                .symbolic_target()
                .unwrap()
                .unwrap()
                .strip_prefix("refs/heads/")
                .unwrap()
                .to_string();
            (commit_id, default_branch)
        };

        let url = format!("file://{}", bare_path.display());
        let config = test_config(assert_fs::TempDir::new().unwrap().to_path_buf());

        assert_eq!(
            resolve_remote_ref(&url, Some(&default_branch), &config).unwrap(),
            commit_id
        );
        assert_eq!(
            resolve_remote_ref(&url, Some(&commit_id.to_string()), &config).unwrap(),
            commit_id
        );
        assert_eq!(resolve_remote_ref(&url, None, &config).unwrap(), commit_id);
    }

    #[tokio::test]
    #[serial]
    async fn copy_dir_recursive_preserves_contents() {
        let src = assert_fs::TempDir::new().unwrap();
        src.child("sub").create_dir_all().unwrap();
        src.child("file.txt").write_str("hello").unwrap();
        src.child("sub/nested.txt").write_str("world").unwrap();
        let dest = assert_fs::TempDir::new().unwrap();
        let dest_path = dest.path().join("copy");
        recursive_copy_dir_no_ignore(src.path(), &dest_path)
            .await
            .unwrap();
        assert_eq!(
            fs::sync::read_to_string(dest_path.join("file.txt")).unwrap(),
            "hello"
        );
        assert_eq!(
            fs::sync::read_to_string(dest_path.join("sub/nested.txt")).unwrap(),
            "world"
        );
    }
}
