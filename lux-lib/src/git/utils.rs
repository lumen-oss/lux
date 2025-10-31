use std::io;

use git2::{AutotagOption, Cred, FetchOptions, RemoteCallbacks, Repository};
use itertools::Itertools;
use tempfile::tempdir;
use thiserror::Error;

use crate::git::url::RemoteGitUrl;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("error creating temporary directory to checkout git repositotory: {0}")]
    CreateTempDir(io::Error),
    #[error("error initializing temporary bare git repository to fetch metadata: {0}")]
    BareRepoInit(git2::Error),
    #[error("error initializing remote repository '{0}' to fetch metadata: {1}")]
    RemoteInit(String, git2::Error),
    #[error("error fetching from remote repository '{0}': {1}")]
    RemoteFetch(String, git2::Error),
    #[error("error listing remote refs for '{0}': {1}")]
    RemoteList(String, git2::Error),
    #[error("could not determine latest tag or commit sha for {0}")]
    NoTagOrCommitSha(String),
}

pub(crate) enum SemVerTagOrSha {
    SemVerTag(String),
    CommitSha(String),
}

pub(crate) fn latest_semver_tag_or_commit_sha(
    url: &RemoteGitUrl,
) -> Result<SemVerTagOrSha, GitError> {
    match latest_semver_tag(url)? {
        Some(tag) => Ok(SemVerTagOrSha::SemVerTag(tag)),
        None => {
            let sha = latest_commit_sha(url)?.ok_or(GitError::NoTagOrCommitSha(url.to_string()))?;
            Ok(SemVerTagOrSha::CommitSha(sha))
        }
    }
}

fn latest_semver_tag(url: &RemoteGitUrl) -> Result<Option<String>, GitError> {
    let temp_dir = tempdir().map_err(GitError::CreateTempDir)?;

    let url_str = url.to_string();
    let repo = Repository::init_bare(&temp_dir).map_err(GitError::BareRepoInit)?;
    let mut remote = repo
        .remote_anonymous(&url_str)
        .map_err(|err| GitError::RemoteInit(url_str.clone(), err))?;
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_url, username_from_url, _allowed_types| {
        Cred::ssh_key_from_agent(username_from_url.unwrap_or("git"))
    });
    let mut fetch_opts = FetchOptions::new();
    fetch_opts.download_tags(AutotagOption::All);
    fetch_opts.remote_callbacks(callbacks);
    remote
        .fetch(&[] as &[&str], Some(&mut fetch_opts), None)
        .map_err(|err| GitError::RemoteFetch(url_str.clone(), err))?;
    let refs = remote
        .list()
        .map_err(|err| GitError::RemoteList(url_str.clone(), err))?;
    Ok(refs
        .iter()
        .filter_map(|head| {
            let tag_name = head.name().strip_prefix("refs/tags/")?;
            let version_str = tag_name.strip_prefix('v').unwrap_or(tag_name);
            if let Ok(version) = semver::Version::parse(version_str) {
                Some((tag_name.to_string(), version))
            } else {
                None
            }
        })
        .sorted_by(|(_, a), (_, b)| b.cmp(a))
        .map(|(version_str, _)| version_str)
        .collect_vec()
        .first()
        .cloned())
}

fn latest_commit_sha(url: &RemoteGitUrl) -> Result<Option<String>, GitError> {
    let temp_dir = tempdir().map_err(GitError::CreateTempDir)?;
    let url_str = url.to_string();
    let repo = Repository::init_bare(&temp_dir).map_err(GitError::BareRepoInit)?;
    let mut remote = repo
        .remote_anonymous(&url_str)
        .map_err(|err| GitError::RemoteInit(url_str.clone(), err))?;
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_url, username_from_url, _allowed_types| {
        Cred::ssh_key_from_agent(username_from_url.unwrap_or("git"))
    });
    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(callbacks);
    remote
        .fetch(&[] as &[&str], Some(&mut fetch_opts), None)
        .map_err(|err| GitError::RemoteFetch(url_str.clone(), err))?;
    let refs = remote
        .list()
        .map_err(|err| GitError::RemoteList(url_str.clone(), err))?;
    Ok(refs.iter().find_map(|head| match head.name() {
        "refs/heads/HEAD" => Some(head.oid().to_string()),
        "refs/heads/main" => Some(head.oid().to_string()),
        "refs/heads/master" => Some(head.oid().to_string()),
        _ => None,
    }))
}

#[cfg(test)]
mod tests {

    use super::*;

    #[tokio::test]
    async fn test_latest_semver_tag_http() {
        if std::env::var("LUX_SKIP_IMPURE_TESTS").unwrap_or("0".into()) == "1" {
            println!("Skipping impure test");
            return;
        }
        let url = "https://github.com/lumen-oss/lux.git".parse().unwrap();
        assert!(latest_semver_tag(&url).unwrap().is_some());
    }

    #[tokio::test]
    async fn test_latest_semver_tag_ssh_user() {
        if std::env::var("LUX_SKIP_IMPURE_TESTS").unwrap_or("0".into()) == "1" {
            println!("Skipping impure test");
            return;
        }
        let url = "git@github.com:lumen-oss/lux.git".parse().unwrap();
        assert!(latest_semver_tag(&url).unwrap().is_some());
    }

    #[tokio::test]
    async fn test_latest_commit_sha_http() {
        if std::env::var("LUX_SKIP_IMPURE_TESTS").unwrap_or("0".into()) == "1" {
            println!("Skipping impure test");
            return;
        }
        let url = "https://github.com/lumen-oss/lux.git".parse().unwrap();
        assert!(latest_commit_sha(&url).unwrap().is_some());
    }

    #[tokio::test]
    async fn test_latest_commit_sha_ssh_user() {
        if std::env::var("LUX_SKIP_IMPURE_TESTS").unwrap_or("0".into()) == "1" {
            println!("Skipping impure test");
            return;
        }
        let url = "git@github.com:lumen-oss/lux.git".parse().unwrap();
        assert!(latest_commit_sha(&url).unwrap().is_some());
    }
}
