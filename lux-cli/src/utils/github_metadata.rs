use git2::Repository;
use lux_lib::git::url::RemoteGitUrl;
use miette::{IntoDiagnostic, Result};
use path_absolutize::Absolutize as _;
use std::io;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug)]
pub struct RepoMetadata {
    pub name: String,
    pub description: Option<String>,
    pub license: Option<String>,
    pub labels: Option<Vec<String>>,
    pub contributors: Vec<String>,
}

impl RepoMetadata {
    pub fn default(path: &Path) -> io::Result<Self> {
        Ok(RepoMetadata {
            name: path
                .absolutize()?
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            description: None,
            license: None,
            contributors: whoami::realname()
                .map(|realname| vec![realname])
                .unwrap_or_default(),
            labels: None,
        })
    }
}

/// Retrieves metadata for a given directory
pub async fn get_metadata_for(directory: Option<&PathBuf>) -> Result<Option<RepoMetadata>> {
    let repo = match directory {
        Some(path) => Repository::open(path).into_diagnostic()?,
        None => Repository::open_from_env().into_diagnostic()?,
    };

    // NOTE(vhyrro): Temporary value is required. Thank the borrow checker.
    let ret = if let Ok(remote) = repo.find_remote("origin").into_diagnostic()?.url() {
        let parsed_url: RemoteGitUrl = remote.parse().into_diagnostic()?;

        let owner = match parsed_url.owner() {
            Some(owner) => owner.to_owned(),
            None => return Ok(None),
        };

        let repo = parsed_url.repo().to_string();

        let octocrab = octocrab::instance();
        let repo_handler = octocrab.repos(owner, repo);

        let contributors = repo_handler
            .list_contributors()
            .send()
            .await
            .into_diagnostic()?;
        let repo_data = repo_handler.get().await.into_diagnostic()?;

        Ok(Some(RepoMetadata {
            name: repo_data.name,
            description: repo_data.description,
            license: repo_data.license.map(|license| license.name),
            labels: repo_data.topics,
            contributors: contributors
                .items
                .into_iter()
                .map(|contributor| contributor.author.login)
                .collect(),
        }))
    } else {
        Ok(None)
    };

    ret
}
