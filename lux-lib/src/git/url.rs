use serde::{de, Deserialize, Deserializer, Serialize};
use std::{
    borrow::Cow,
    fmt::Display,
    hash::{DefaultHasher, Hash, Hasher as _},
    str::FromStr,
};
use thiserror::Error;
use url::Url;

/// GitUrl represents an input url that is a url used by git
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct RemoteGitUrl {
    pub(crate) url: Url,
    /// The raw URL string
    url_str: String,
}

#[derive(Debug, Error)]
pub enum RemoteGitUrlParseError {
    #[error("error parsing git URL:\n{0}")]
    GitUrlParse(#[from] url::ParseError),
    #[error("not a remote git URL: {0}")]
    NotARemoteGitUrl(String),
}

impl RemoteGitUrl {
    /// Get the repo name, as the final component of the path, with any .git
    /// suffix removed, or as the hostname, if there is no final path component,
    /// or as a hash of the whole URL otherwise.
    pub fn repo(&self) -> Cow<'_, str> {
        let url = &self.url;
        url.path_segments()
            .into_iter()
            .flatten()
            .rev()
            .next()
            .map(|part| part.strip_suffix(".git").unwrap_or(part))
            .or_else(|| url.host_str())
            .map(Cow::Borrowed)
            .unwrap_or_else(|| {
                let mut hasher = DefaultHasher::new();
                url.hash(&mut hasher);
                let hash = hasher.finish();
                Cow::Owned(format!("lua-{hash}"))
            })
    }
    /// Get the repo owner, as second-final component of the path.
    pub fn owner(&self) -> Option<&str> {
        self.url
            .path_segments()
            .into_iter()
            .flatten()
            .rev()
            .skip(1)
            .next()
    }
}

impl FromStr for RemoteGitUrl {
    type Err = RemoteGitUrlParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let url: Url = s.parse()?;
        Ok(RemoteGitUrl {
            url,
            url_str: String::from(s),
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
