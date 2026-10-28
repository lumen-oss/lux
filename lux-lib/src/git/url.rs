use miette::Diagnostic;
use serde::{de, Deserialize, Deserializer, Serialize};
use std::{fmt::Display, hash::Hash, str::FromStr};
use thiserror::Error;
use url::Url;

/// GitUrl represents an input url that is a url used by git
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct RemoteGitUrl {
    pub(crate) url: Url,
    host_str: String,
    /// The raw URL string
    url_str: String,
}

#[derive(Debug, Error, Diagnostic)]
pub enum RemoteGitUrlParseError {
    #[error("error parsing git URL")]
    GitUrlParse(#[from] url::ParseError),
    #[error("unsupported git remote scheme {scheme} in URL {url}")]
    UnsupportedRemoteScheme { scheme: String, url: Url },
    #[error("URL {0} missing host name")]
    MissingHostName(Url),
}

impl RemoteGitUrl {
    /// Get the repo name, as the final component of the path, with any .git
    /// suffix removed, or as the hostname, if there is no final path component,
    /// or as a hash of the whole URL otherwise.
    pub fn repo(&self) -> &str {
        let url = &self.url;
        url.path_segments()
            .into_iter()
            .flatten()
            .next_back()
            .map(|part| part.strip_suffix(".git").unwrap_or(part))
            .unwrap_or(&self.host_str)
    }
    /// Get the repo owner, as second-final component of the path.
    pub fn owner(&self) -> Option<&str> {
        self.url.path_segments().into_iter().flatten().rev().nth(1)
    }
    /// Get the host name
    pub fn host(&self) -> &str {
        &self.host_str
    }
}

impl FromStr for RemoteGitUrl {
    type Err = RemoteGitUrlParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let url: Url = match normalize_git_url(s) {
            Some(url) => url.parse()?,
            None => s.parse()?,
        };
        let scheme = url.scheme();
        if !matches!(scheme, "ssh" | "git" | "http" | "https" | "ftp" | "ftps") {
            return Err(RemoteGitUrlParseError::UnsupportedRemoteScheme {
                scheme: scheme.into(),
                url,
            });
        }
        let Some(host_str) = url.host_str() else {
            return Err(RemoteGitUrlParseError::MissingHostName(url));
        };
        Ok(RemoteGitUrl {
            host_str: String::from(host_str),
            url,
            url_str: String::from(s),
        })
    }
}

/// Converts a Git URL (`git@github.com:nvim-neorg/neorg`) into an `ssh://` URL.
fn normalize_git_url(s: &str) -> Option<String> {
    let (host, path) = s.split_once(':')?;
    // Filter out false positives
    if path.starts_with('/') || !(host.contains('.') || host.contains('@')) {
        return None;
    }
    Some(format!("ssh://{host}/{path}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scp_like_url_ssh() {
        let url: RemoteGitUrl = "git@github.com:lumen-oss/lux.git".parse().unwrap();
        assert_eq!(url.host(), "github.com");
        assert_eq!(url.owner(), Some("lumen-oss"));
        assert_eq!(url.repo(), "lux");
        assert_eq!(url.to_string(), "git@github.com:lumen-oss/lux.git");
        let url: RemoteGitUrl = "github.com:lumen-oss/lux".parse().unwrap();
        assert_eq!(url.host(), "github.com");
        assert_eq!(url.repo(), "lux");
    }

    #[test]
    fn test_regular_urls_still_parse() {
        for raw in [
            "https://github.com/lumen-oss/lux.git",
            "ssh://git@github.com/lumen-oss/lux.git",
        ] {
            let url: RemoteGitUrl = raw.parse().unwrap();
            assert_eq!(url.host(), "github.com");
            assert_eq!(url.repo(), "lux");
        }
    }
}
