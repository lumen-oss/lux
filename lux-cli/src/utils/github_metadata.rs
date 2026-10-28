use git2::Repository;
use lux_lib::config::access_tokens::AccessToken;
use lux_lib::config::Config;
use lux_lib::git::url::RemoteGitUrl;
use miette::{IntoDiagnostic, Result};
use path_absolutize::Absolutize as _;
use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde::Deserialize;
use std::io;
use std::path::Path;
use std::path::PathBuf;

const GITHUB_HOST: &str = "github.com";
const GITHUB_API: &str = "https://api.github.com";

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

#[derive(Deserialize)]
struct GitHubRepo {
    name: String,
    description: Option<String>,
    license: Option<GitHubLicense>,
    topics: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct GitHubLicense {
    name: String,
}

#[derive(Deserialize)]
struct GitHubContributor {
    login: String,
}

/// Retrieves metadata for a given directory
pub async fn get_metadata_for(
    directory: Option<&PathBuf>,
    config: &Config,
) -> Result<Option<RepoMetadata>> {
    let repo = match directory {
        Some(path) => Repository::open(path).into_diagnostic()?,
        None => Repository::open_from_env().into_diagnostic()?,
    };

    match repo
        .find_remote("origin")
        .ok()
        .and_then(|remote| remote.url().map(str::to_owned).ok())
    {
        Some(url) => {
            let parsed_url: RemoteGitUrl = url.parse().into_diagnostic()?;

            match (parsed_url.host(), parsed_url.owner()) {
                // NOTE: Only github is supported for now.
                (GITHUB_HOST, Some(owner)) => {
                    let client = lux_lib::reqwest::https_client(config).into_diagnostic()?;
                    let token = config.access_token(GITHUB_HOST);

                    fetch_metadata(client, GITHUB_API, owner, parsed_url.repo(), token)
                        .await
                        .into_diagnostic()
                        .map(Some)
                }
                _ => Ok(None),
            }
        }
        None => Ok(None),
    }
}

async fn fetch_metadata(
    client: &reqwest::Client,
    url: &str,
    owner: &str,
    repo: &str,
    token: Option<AccessToken>,
) -> Result<RepoMetadata, reqwest::Error> {
    let get = |path: String| {
        let request = client
            .get(format!("{url}{path}"))
            .header(ACCEPT, "application/vnd.github+json");
        match &token {
            Some(token) => request.header(
                AUTHORIZATION,
                format!("token {}", unsafe { token.password() }),
            ),
            None => request,
        }
    };

    let repo_data: GitHubRepo = get(format!("/repos/{owner}/{repo}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    // Only the most prominent contributor is used, as the default maintainer.
    let contributors: Vec<String> =
        match get(format!("/repos/{owner}/{repo}/contributors?per_page=1"))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
        {
            Ok(response) => response
                .json::<Vec<GitHubContributor>>()
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|contributor| contributor.login)
                .collect(),
            Err(_) => Vec::new(),
        };

    Ok(RepoMetadata {
        name: repo_data.name,
        description: repo_data.description,
        license: repo_data.license.map(|license| license.name),
        labels: repo_data.topics,
        contributors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use httptest::{matchers::request, responders::json_encoded, Expectation, Server};

    #[tokio::test]
    async fn test_fetch_metadata() {
        let server = Server::run();
        server.expect(
            Expectation::matching(request::method_path("GET", "/repos/someone/some-repo"))
                .respond_with(json_encoded(serde_json::json!({
                    "name": "some-repo",
                    "description": "A test repo",
                    "license": { "name": "MIT License" },
                    "topics": ["lua", "rocks"],
                }))),
        );
        server.expect(
            Expectation::matching(request::method_path(
                "GET",
                "/repos/someone/some-repo/contributors",
            ))
            .respond_with(json_encoded(
                serde_json::json!([{ "login": "someone", "contributions": 42 }]),
            )),
        );

        let metadata = fetch_metadata(
            &reqwest::Client::new(),
            server.url("/").to_string().trim_end_matches('/'),
            "someone",
            "some-repo",
            None,
        )
        .await
        .unwrap();

        assert_eq!(metadata.name, "some-repo");
        assert_eq!(metadata.description.as_deref(), Some("A test repo"));
        assert_eq!(metadata.license.as_deref(), Some("MIT License"));
        assert_eq!(
            metadata.labels.as_deref(),
            Some(&["lua".to_string(), "rocks".to_string()][..])
        );
        assert_eq!(metadata.contributors, vec!["someone".to_string()]);
    }

    #[tokio::test]
    async fn test_fetch_metadata_without_contributors() {
        let server = Server::run();
        server.expect(
            Expectation::matching(request::method_path("GET", "/repos/someone/empty"))
                .respond_with(json_encoded(serde_json::json!({ "name": "empty" }))),
        );
        server.expect(
            Expectation::matching(request::method_path(
                "GET",
                "/repos/someone/empty/contributors",
            ))
            .respond_with(httptest::responders::status_code(204)),
        );

        let metadata = fetch_metadata(
            &reqwest::Client::new(),
            server.url("/").to_string().trim_end_matches('/'),
            "someone",
            "empty",
            None,
        )
        .await
        .unwrap();

        assert_eq!(metadata.name, "empty");
        assert!(metadata.contributors.is_empty());
    }
}
