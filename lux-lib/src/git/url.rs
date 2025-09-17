use serde::{de, Deserialize, Deserializer, Serialize};
use std::{fmt::Display, str::FromStr};
use thiserror::Error;

// NOTE: This module implements a basic `GitUrl` struct with only what we need.
// This is so that we don't have to expose `git_url_parse::GitUrl`, which is  highly unstable,
// via our API.

/// GitUrl represents an input url that is a url used by git
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct GitUrl {
    /// The fully qualified domain name (FQDN) or IP of the repo
    pub(crate) host: Option<String>,
    /// The name of the repo
    pub(crate) name: String,
    /// The owner/account/project name
    pub(crate) owner: Option<String>,
    /// The raw URL string
    url_str: String,
}

#[derive(Debug, Error)]
#[error("error parsing git URL:\n{0}")]
pub struct GitUrlParseError(String);

impl FromStr for GitUrl {
    type Err = GitUrlParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let url =
            git_url_parse::GitUrl::parse(s).map_err(|err| GitUrlParseError(err.to_string()))?;
        Ok(GitUrl {
            host: url.host.clone(),
            name: url.name.clone(),
            owner: url.owner.clone(),
            url_str: url.to_string(),
        })
    }
}

impl Display for GitUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.url_str.fmt(f)
    }
}

impl<'de> Deserialize<'de> for GitUrl {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

impl Serialize for GitUrl {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_string().serialize(serializer)
    }
}
