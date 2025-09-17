use serde::{de, Deserialize, Deserializer, Serialize};
use std::{fmt::Display, str::FromStr};
use thiserror::Error;

// NOTE: This module implements a basic `GitUrl` struct with only what we need.
// This is so that we don't have to expose `git_url_parse::GitUrl`, which is  highly unstable,
// via our API.

/// GitUrl represents an input url that is a url used by git
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RemoteGitUrl {
    /// The fully qualified domain name (FQDN) or IP of the repo
    pub(crate) host: String,
    /// The name of the repo
    pub(crate) repo: String,
    /// The owner/account/project name
    pub(crate) owner: String,
    /// The raw URL string
    url_str: String,
}

#[derive(Debug, Error)]
pub enum RemoteGitUrlParseError {
    #[error("error parsing git URL:\n{0}")]
    GitUrlParse(String),
    #[error("not a remote git URL: {0}")]
    NotARemoteGitUrl(String),
}

impl FromStr for RemoteGitUrl {
    type Err = RemoteGitUrlParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let url = git_url_parse::GitUrl::parse(s)
            .map_err(|err| RemoteGitUrlParseError::GitUrlParse(err.to_string()))?;
        let host = url
            .host()
            .ok_or_else(|| RemoteGitUrlParseError::NotARemoteGitUrl(url.to_string()))?;
        let provider: Result<
            git_url_parse::types::provider::GenericProvider,
            git_url_parse::GitUrlParseError,
        > = url.provider_info();
        let provider =
            provider.map_err(|_err| RemoteGitUrlParseError::NotARemoteGitUrl(s.to_string()))?;
        Ok(RemoteGitUrl {
            host: host.to_string(),
            repo: provider.repo().to_string(),
            owner: provider.owner().to_string(),
            url_str: url.to_string(),
        })
    }
}

impl Display for RemoteGitUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.url_str.fmt(f)
    }
}

impl<'de> Deserialize<'de> for RemoteGitUrl {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

impl Serialize for RemoteGitUrl {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_string().serialize(serializer)
    }
}
