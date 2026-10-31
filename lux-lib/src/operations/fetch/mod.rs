use crate::config::Config;
use crate::fs;
use crate::git;
use crate::hash::HasIntegrity;
use crate::lockfile::RemotePackageSourceUrl;
use crate::package;
use crate::project;
use crate::reqwest::RequestBuilderExt;
use crate::rockspec;
use crate::rockspec::Rockspec;
use crate::wally;
use auth_git2::{GitAuthenticator, Prompter};
use bon::Builder;
use git2::build::RepoBuilder;
use git2::{Direction, FetchOptions, ObjectType, RemoteCallbacks};
use miette::Diagnostic;
use remove_dir_all::remove_dir_all;
use ssri::Integrity;
use std::io;
use std::io::Cursor;
use std::io::Read;
use std::path::{Path, PathBuf};
use thiserror::Error;

mod src_rock;

use crate::operations::unpack;

/// A rocks package source fetcher, providing fine-grained control
/// over how a package should be fetched.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(super) struct SrcRock<'a> {
    #[builder(start_fn)]
    package: &'a package::Spec,
    #[builder(start_fn)]
    dest_dir: &'a Path,
    #[builder(start_fn)]
    config: &'a Config,
}

impl<State> SrcRockBuilder<'_, State>
where
    State: src_rock_builder::State + src_rock_builder::IsComplete,
{
    pub async fn fetch(self) -> Result<RemotePackageSourceMetadata, src_rock::Error> {
        src_rock::do_fetch_src_rock(self._build()).await
    }
}

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
    pub async fn fetch(self) -> Result<(), Error> {
        self.fetch_internal().await?;
        Ok(())
    }

    /// Fetch and unpack the source into the `dest_dir`,
    /// returning the source `Integrity`.
    pub(crate) async fn fetch_internal(self) -> Result<RemotePackageSourceMetadata, Error> {
        let fetch = self._build();
        let rockspec = fetch.rockspec;
        let rock_source = rockspec.source().current_platform();
        let dest_dir = fetch.dest_dir;
        let config = fetch.config;
        // prioritise lockfile source, if present
        let source_spec = match &fetch.source_url {
            Some(source_url) => match source_url {
                RemotePackageSourceUrl::Git {
                    url, checkout_ref, ..
                } => rockspec::source::Spec::Git(git::Source {
                    url: url.parse()?,
                    git_ref: Some(git::Ref::Tag(checkout_ref.clone())),
                }),
                RemotePackageSourceUrl::Url { url } => rockspec::source::Spec::Url(url.clone()),
                RemotePackageSourceUrl::File { path } => rockspec::source::Spec::File(path.clone()),
            },
            None => rock_source.source_spec.clone(),
        };
        match fetch_src_impl(source_spec, rockspec, rock_source, dest_dir, config).await {
            Err(err)
                if fetch
                    .source_url
                    .is_some_and(|url| matches!(url, RemotePackageSourceUrl::File { .. })) =>
            {
                // Don't fall back to downloading .src.rock archives if a local source was specified.
                Err(err)
            }
            Err(err) => match &fetch.rockspec.source().current_platform().source_spec {
                rockspec::source::Spec::Git(_) | rockspec::source::Spec::Url(_) => {
                    // A failed fetch may leave a partial source tree behind, which confuses source
                    // root detection after unpacking the fallback archive.
                    if fetch.dest_dir.exists() {
                        fs::sync::remove_dir_all(fetch.dest_dir)?;
                    }
                    let package = package::Spec::new(
                        fetch.rockspec.package().clone(),
                        fetch.rockspec.version().clone(),
                    );
                    match SrcRock::new(&package, fetch.dest_dir, fetch.config)
                        .fetch()
                        .await
                    {
                        Ok(metadata) => Ok(metadata),
                        Err(fallback_err) => {
                            tracing::error!("fallback .src.rock download failed: {fallback_err:?}");
                            Err(err)
                        }
                    }
                }
                rockspec::source::Spec::File(_) | rockspec::source::Spec::Wally(_) => Err(err),
            },
            Ok(metadata) => Ok(metadata),
        }
    }
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("failed to clone rock source")]
    #[diagnostic(help("check your network connection and verify the git URL is correct."))]
    GitClone(#[from] git2::Error),
    #[error("git operation failed")]
    #[diagnostic(help("this is a bug, please report it."))]
    GitJoin(#[from] tokio::task::JoinError),
    #[error("failed to parse git URL")]
    #[diagnostic(forward(0))]
    GitUrlParse(#[from] git::url::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Request(#[from] crate::reqwest::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Unpack(#[from] unpack::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    SrcRock(#[from] src_rock::Error),
    #[error("unable to remove the '.git' directory")]
    #[diagnostic(help(
        "check that no process is using the directory and you have write permissions."
    ))]
    CleanGitDir(#[source] io::Error),
    #[error("failed to resolve wally package '{0}'")]
    #[diagnostic(
        code(lux_lib::wally::unresolved_source),
        help("this is a bug, please report it.")
    )]
    WallySource(String),
    #[error("unable to compute hash")]
    Hash(#[source] io::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
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

/// Returns the refspecs to try for a pinned git ref.
fn ref_candidates(git_ref: &git::Ref) -> Vec<(String, String)> {
    // Some rockspecs reference a tag in the `branch` field (penlight), so a branch should also try
    // falling back to a tag.
    let tag_candidate = |name: &str| {
        let ref_name = format!("refs/tags/{name}");
        (ref_name.clone(), format!("+{ref_name}:{ref_name}"))
    };
    // Some rockspecs pin a commit hash in the `branch` or `tag` field (e.g. tiniest).
    let sha_candidate = |name: &str| {
        git2::Oid::from_str(name)
            .ok()
            .map(|_| (name.to_string(), name.to_string()))
    };

    match git_ref {
        git::Ref::Branch(branch) => {
            let mut candidates = vec![
                (
                    format!("refs/heads/{branch}"),
                    format!("+refs/heads/{branch}:refs/remotes/origin/{branch}"),
                ),
                tag_candidate(branch),
            ];
            candidates.extend(sha_candidate(branch));
            candidates
        }
        git::Ref::Tag(tag) => {
            let mut candidates = vec![tag_candidate(tag)];
            candidates.extend(sha_candidate(tag));
            candidates
        }
    }
}

/// Initialises an empty repository and shallow-fetches only the necessary ref.
fn fetch_pinned_ref(
    path: &Path,
    url: &str,
    auth: &GitAuthenticator,
    git_config: &git2::Config,
    candidates: &[(String, String)],
) -> Result<git2::Repository, git2::Error> {
    let repo = git2::Repository::init(path)?;

    // Scopes used to prevent the Borrow Checker™ from complaining.
    {
        let mut remote = repo.remote("origin", url)?;
        let mut callbacks = RemoteCallbacks::new();
        callbacks.credentials(auth.credentials(git_config));
        let mut connection = remote.connect_auth(Direction::Fetch, Some(callbacks), None)?;

        let available = connection.list()?;
        let (refspec, oid) = candidates
            .iter()
            .find_map(|(ref_name, refspec)| {
                available
                    .iter()
                    .find(|head| head.name() == ref_name)
                    .map(|head| (refspec.as_str(), head.oid()))
            })
            .or_else(|| {
                // A pinned commit hash isn't advertised as a ref, but the server can fetch it.
                candidates.iter().find_map(|(ref_name, refspec)| {
                    git2::Oid::from_str(ref_name)
                        .ok()
                        .map(|oid| (refspec.as_str(), oid))
                })
            })
            .ok_or_else(|| git2::Error::from_str("no matching branch or tag found"))?;

        let mut fetch_options = FetchOptions::new();
        fetch_options.depth(1);
        fetch_options.update_fetchhead(false);
        connection
            .remote()
            .download(&[refspec], Some(&mut fetch_options))?;

        let commit = repo.find_object(oid, None)?.peel(ObjectType::Commit)?;
        repo.checkout_tree(&commit, None)?;
    };

    Ok(repo)
}

/// Shallow-clones the remote's default branch.
fn clone_default_branch(
    path: &Path,
    url: &str,
    auth: &GitAuthenticator,
    git_config: &git2::Config,
) -> Result<git2::Repository, git2::Error> {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(auth.credentials(git_config));
    let mut fetch_options = FetchOptions::new();
    fetch_options.update_fetchhead(false);
    fetch_options.remote_callbacks(callbacks);
    fetch_options.depth(1);

    RepoBuilder::new()
        .fetch_options(fetch_options)
        .clone(url, path)
}

/// Initialises and checks out git submodules,
/// returning the relative paths of any submodules that were checked out.
fn init_submodules(
    repo: &git2::Repository,
    auth: &GitAuthenticator,
    git_config: &git2::Config,
) -> Result<Vec<PathBuf>, Error> {
    let workdir = repo
        .workdir()
        .ok_or_else(|| git2::Error::from_str("cloned repository has no working directory"))?;
    let index = repo.index()?;
    let mut submodule_paths = Vec::new();
    for mut submodule in repo.submodules()? {
        // `.gitmodules` may list paths that are not gitlinks in the checked-out tree.
        // Updating those does nothing and leaves no `.git` to clean up below.
        if index.get_path(submodule.path(), 0).is_none() {
            continue;
        }
        let mut callbacks = RemoteCallbacks::new();
        callbacks.credentials(auth.credentials(git_config));
        let mut fetch_options = FetchOptions::new();
        fetch_options.remote_callbacks(callbacks);
        let mut options = git2::SubmoduleUpdateOptions::new();
        options.fetch(fetch_options);
        submodule.update(true, Some(&mut options))?;
        // The .git file/directory is not deterministic
        let git_file = workdir.join(submodule.path()).join(".git");
        if git_file.is_dir() {
            fs::sync::remove_dir_all(&git_file)?;
        } else {
            fs::sync::remove_file(&git_file)?;
        }
        submodule_paths.push(submodule.path().to_path_buf());
    }
    Ok(submodule_paths)
}

#[tracing::instrument(
    name = "Fetching source",
    level = "info",
    skip_all,
    fields(location = source_spec.to_string()),
)]
async fn fetch_src_impl<R: Rockspec>(
    mut source_spec: rockspec::source::Spec,
    rockspec: &R,
    rock_source: &rockspec::source::Remote,
    dest_dir: &Path,
    config: &Config,
) -> Result<RemotePackageSourceMetadata, Error> {
    if let Some(vendor_dir) = config.vendor_dir() {
        source_spec = match source_spec {
            // could be a project directory (not vendored) or a local source
            // or a vendored dependency that we have already resolved
            rockspec::source::Spec::File(_) => source_spec,
            _ => {
                let pkg_vendor_dir =
                    vendor_dir.join(format!("{}@{}", rockspec.package(), rockspec.version()));
                rockspec::source::Spec::File(pkg_vendor_dir)
            }
        }
    }
    let metadata = match &source_spec {
        rockspec::source::Spec::Git(git) => {
            let url = git.url.to_string();
            tracing::debug!(message = format!("Cloning {url}").as_str());

            let clone_url = url.clone();
            let clone_dir = dest_dir.to_path_buf();
            let host = git.url.host().to_string();
            let no_prompt = config.no_prompt();
            let access_token = config.access_token(&host);
            let git_ref = git.git_ref.clone();

            let (checkout_ref, has_submodules) = tokio::task::spawn_blocking(move || {
                let mut auth = if no_prompt {
                    GitAuthenticator::default()
                        .try_password_prompt(0)
                        .prompt_ssh_key_password(false)
                        .set_prompter(NullPrompter)
                } else {
                    GitAuthenticator::default()
                };
                if let Some(access_token) = access_token {
                    auth = auth.add_plaintext_credentials(&host, access_token.username(), unsafe {
                        access_token.password()
                    });
                }
                let git_config = git2::Config::open_default()?;
                let repo = match &git_ref {
                    Some(git_ref) => fetch_pinned_ref(
                        &clone_dir,
                        &clone_url,
                        &auth,
                        &git_config,
                        &ref_candidates(git_ref),
                    )?,
                    None => clone_default_branch(&clone_dir, &clone_url, &auth, &git_config)?,
                };

                let checkout_ref = match &git_ref {
                    Some(git_ref) => match git_ref {
                        git::Ref::Tag(name) | git::Ref::Branch(name) => name.clone(),
                    },
                    None => repo.head()?.peel_to_commit()?.id().to_string(),
                };

                let submodule_paths = init_submodules(&repo, &auth, &git_config)?;

                Ok::<_, Error>((checkout_ref, !submodule_paths.is_empty()))
            })
            .await??;

            // The .git directory is not deterministic
            remove_dir_all(dest_dir.join(".git")).map_err(Error::CleanGitDir)?;
            let hash = dest_dir.hash().await.map_err(Error::Hash)?;
            RemotePackageSourceMetadata {
                hash,
                source_url: RemotePackageSourceUrl::Git {
                    url,
                    checkout_ref,
                    submodules: has_submodules,
                },
            }
        }
        rockspec::source::Spec::Url(url) => {
            tracing::debug!(message = format!("Downloading {url}").as_str());

            // NOTE: We don't enforce HTTPS when fetching sources because some rockspecs
            // have HTTP URLs in `source.url`.
            let request = crate::reqwest::http_client(config)?
                .get(url.clone())
                .apply_access_token(config, url);
            let request = wally::apply_content_headers(request, url);
            let response = request.send().await?.error_for_status()?.bytes().await?;
            let hash = response.hash().await.map_err(Error::Hash)?;
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
            unpack::extract(
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
        rockspec::source::Spec::Wally(_) => {
            return Err(Error::WallySource(rockspec.package().to_string()))
        }
        rockspec::source::Spec::File(path) => {
            tracing::debug!(message = format!("Copying {}", path.display()).as_str());

            let hash = if path.is_dir() {
                let hash = path.hash().await.map_err(Error::Hash)?;
                project::copy_dir(path, dest_dir).await?;
                hash
            } else {
                let mut file = fs::sync::open(path)?;
                let mut buffer = Vec::new();
                file.read_to_end(&mut buffer)
                    .map_err(|source| fs::Error::Read {
                        path: path.to_path_buf(),
                        source,
                    })?;
                let mime_type = infer::get(&buffer).map(|file_type| file_type.mime_type());
                let file_name = path
                    .file_name()
                    .map(|os_str| os_str.to_string_lossy())
                    .unwrap_or(path.to_string_lossy())
                    .to_string();
                unpack::extract(
                    mime_type,
                    file,
                    rock_source.unpack_dir.is_none(),
                    file_name,
                    dest_dir,
                )
                .await?;
                path.hash().await.map_err(Error::Hash)?
            };
            RemotePackageSourceMetadata {
                hash,
                source_url: RemotePackageSourceUrl::File { path: path.clone() },
            }
        }
    };
    Ok(metadata)
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Self::Request(err.into())
    }
}
