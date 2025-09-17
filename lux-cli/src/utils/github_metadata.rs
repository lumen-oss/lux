use eyre::eyre;
use eyre::Result;
use git2::Repository;
use git_url_parse::GitUrl;
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
            contributors: vec![whoami::realname()],
            labels: None,
        })
    }
}

/// Retrieves metadata for a given directory
pub async fn get_metadata_for(directory: Option<&PathBuf>) -> Result<Option<RepoMetadata>> {
    let repo = match directory {
        Some(path) => Repository::open(path)?,
        None => Repository::open_from_env()?,
    };

    // NOTE(vhyrro): Temporary value is required. Thank the borrow checker.
    let ret = if let Some(remote) = repo.find_remote("origin")?.url() {
        let parsed_url = GitUrl::parse(remote)?;

        let (owner, name) = match (parsed_url.owner, parsed_url.name) {
            (Some(owner), name) => (owner, name),
            _ => return Err(eyre!("unable to parse remote `origin` - it's likely that your upstream remote is malformed!")),
        };

        let octocrab = octocrab::instance();
        let repo_handler = octocrab.repos(owner, name);

        let contributors = repo_handler.list_contributors().send().await?;
        let repo_data = repo_handler.get().await?;

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
