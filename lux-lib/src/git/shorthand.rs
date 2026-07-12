use std::{fmt::Display, str::FromStr};

use crate::git::url::{RemoteGitUrl, RemoteGitUrlParseError};
use chumsky::{prelude::*, Parser};
use miette::Diagnostic;
use serde::{de, Deserialize, Deserializer};
use thiserror::Error;

const GITHUB: &str = "github";
const GITLAB: &str = "gitlab";
const SOURCEHUT: &str = "sourcehut";
const CODEBERG: &str = "codeberg";

#[derive(Debug, Error, Diagnostic)]
#[error("error parsing git source: {0:#?}")]
pub struct ParseError(Vec<String>);

/// Helper for parsing Git URLs from shorthands, e.g. "gitlab:owner/repo"
#[derive(Debug, Clone)]
pub struct RemoteGitUrlShorthand(RemoteGitUrl);

impl RemoteGitUrlShorthand {
    pub fn parse_with_prefix(s: &str) -> Result<Self, ParseError> {
        prefix_parser()
            .parse(s)
            .into_result()
            .map_err(|err| ParseError(err.into_iter().map(|e| e.to_string()).collect()))
    }
    pub fn repo_name() {}
}

impl FromStr for RemoteGitUrlShorthand {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match parser()
            .parse(s)
            .into_result()
            .map_err(|err| ParseError(err.into_iter().map(|e| e.to_string()).collect()))
        {
            Ok(url) => Ok(url),
            Err(err) => match s.parse() {
                // fall back to parsing the URL directly
                Ok(url) => Ok(Self(url)),
                Err(_) => Err(err),
            },
        }
    }
}

impl<'de> Deserialize<'de> for RemoteGitUrlShorthand {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

impl From<RemoteGitUrl> for RemoteGitUrlShorthand {
    fn from(value: RemoteGitUrl) -> Self {
        Self(value)
    }
}

impl From<RemoteGitUrlShorthand> for RemoteGitUrl {
    fn from(value: RemoteGitUrlShorthand) -> Self {
        value.0
    }
}

#[derive(Debug, Default)]
enum GitHost {
    #[default]
    Github,
    Gitlab,
    Sourcehut,
    Codeberg,
}

fn url_from_git_host(
    host: GitHost,
    owner: String,
    repo: String,
) -> Result<RemoteGitUrlShorthand, RemoteGitUrlParseError> {
    let url_str = match host {
        GitHost::Github => format!("https://github.com/{owner}/{repo}.git"),
        GitHost::Gitlab => format!("https://gitlab.com/{owner}/{repo}.git"),
        GitHost::Sourcehut => format!("https://git.sr.ht/~{owner}/{repo}"),
        GitHost::Codeberg => format!("https://codeberg.org/~{owner}/{repo}.git"),
    };
    let url = url_str.parse()?;
    Ok(RemoteGitUrlShorthand(url))
}

fn to_tuple<T>(v: Vec<T>) -> (T, T)
where
    T: Clone,
{
    (v[0].clone(), v[1].clone())
}

// A parser that expects a prefix
fn prefix_parser<'a>(
) -> impl Parser<'a, &'a str, RemoteGitUrlShorthand, chumsky::extra::Err<Rich<'a, char>>> {
    let git_host_prefix = just(GITHUB)
        .or(just(GITLAB).or(just(SOURCEHUT).or(just(CODEBERG))))
        .then_ignore(just(":"))
        .map(|prefix| match prefix {
            GITHUB => GitHost::Github,
            GITLAB => GitHost::Gitlab,
            SOURCEHUT => GitHost::Sourcehut,
            CODEBERG => GitHost::Codeberg,
            _ => unreachable!(),
        })
        .map_err(|err: Rich<'a, char>| {
            let span = *err.span();
            Rich::custom(span, "missing git host prefix. Expected 'github:', 'gitlab:', 'sourcehut:' or 'codeberg:'.")
        });
    let owner_repo = none_of('/')
        .repeated()
        .collect::<String>()
        .separated_by(just('/'))
        .exactly(2)
        .collect::<Vec<String>>()
        .map(to_tuple);
    git_host_prefix
        .then(owner_repo)
        .try_map(|(host, (owner, repo)), span| {
            let url = url_from_git_host(host, owner, repo).map_err(|err| {
                Rich::custom(span, format!("error parsing git url shorthand: {err}"))
            })?;
            Ok(url)
        })
}

// A parser for scp-style git URLs, converting them into ssh: URLs.
fn scp_style_parser<'a>(
) -> impl Parser<'a, &'a str, RemoteGitUrlShorthand, chumsky::extra::Err<Rich<'a, char>>> {
    none_of(":/")
        .repeated()
        .collect::<String>()
        .then(
            just(':')
                .ignore_then(just("//").not())
                .ignore_then(any().repeated().collect::<String>()),
        )
        .try_map(|(host, path), span| {
            let inner = format!("ssh://{host}/{path}")
                .parse::<RemoteGitUrl>()
                .map_err(|err| {
                    Rich::custom(span, format!("error parsing scp style git url: {err}"))
                })?;
            Ok(RemoteGitUrlShorthand(inner))
        })
}

// A parser that tries to parse as such:
//
// * If it can be parsed exactly as one of our shorthand prefixes, it is expanded into that host reference.
// * If it can be parsed as a simple owner/repo pair, it is considered to be a github shorthand reference.
// * If the string has at least one colon, and the first colon does not follow any slashes and is not immediately followed by two slashes, it is considered to be in scp style.
fn parser<'a>(
) -> impl Parser<'a, &'a str, RemoteGitUrlShorthand, chumsky::extra::Err<Rich<'a, char>>> {
    let owner_repo = none_of(":/")
        .repeated()
        .collect::<String>()
        .separated_by(just('/'))
        .exactly(2)
        .collect::<Vec<String>>()
        .map(to_tuple);
    owner_repo
        .try_map(|(owner, repo), span| {
            let url = url_from_git_host(GitHost::default(), owner, repo).map_err(|err| {
                Rich::custom(span, format!("error parsing git url shorthand: {err}"))
            })?;
            Ok(url)
        })
        .or(prefix_parser())
        .or(scp_style_parser())
}

impl Display for RemoteGitUrlShorthand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.0.url.host_str(), self.0.owner()) {
            (Some("github.com"), Some(owner)) => {
                format!("{}:{}/{}", GITHUB, owner, self.0.repo())
            }
            (Some("gitlab.com"), Some(owner)) => {
                format!("{}:{}/{}", GITLAB, owner, self.0.repo())
            }
            (Some("git.sr.ht"), Some(owner)) => {
                format!("{}:{}/{}", SOURCEHUT, owner.replace('~', ""), self.0.repo())
            }
            (Some("codeberg.org"), Some(owner)) => {
                format!("{}:{}/{}", CODEBERG, owner.replace('~', ""), self.0.repo())
            }
            _ => {
                format!("{}", self.0)
            }
        }
        .fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn owner_repo_shorthand() {
        let url_shorthand: RemoteGitUrlShorthand = "lumen-oss/lux".parse().unwrap();
        assert_eq!(url_shorthand.0.owner(), Some("lumen-oss"));
        assert_eq!(url_shorthand.0.repo(), "lux");
    }

    #[tokio::test]
    async fn github_shorthand() {
        let url_shorthand_str = "github:lumen-oss/lux";
        let url_shorthand: RemoteGitUrlShorthand = url_shorthand_str.parse().unwrap();
        assert_eq!(url_shorthand.0.url.host_str(), Some("github.com"));
        assert_eq!(url_shorthand.0.owner(), Some("lumen-oss"));
        assert_eq!(url_shorthand.0.repo(), "lux");
        assert_eq!(url_shorthand.to_string(), url_shorthand_str.to_string());
    }

    #[tokio::test]
    async fn gitlab_shorthand() {
        let url_shorthand_str = "gitlab:lumen-oss/lux";
        let url_shorthand: RemoteGitUrlShorthand = url_shorthand_str.parse().unwrap();
        assert_eq!(url_shorthand.0.url.host_str(), Some("gitlab.com"));
        assert_eq!(url_shorthand.0.owner(), Some("lumen-oss"));
        assert_eq!(url_shorthand.0.repo(), "lux");
        assert_eq!(url_shorthand.to_string(), url_shorthand_str.to_string());
    }

    #[tokio::test]
    async fn sourcehut_shorthand() {
        let url_shorthand_str = "sourcehut:lumen-oss/lux";
        let url_shorthand: RemoteGitUrlShorthand = url_shorthand_str.parse().unwrap();
        assert_eq!(url_shorthand.0.url.host_str(), Some("git.sr.ht"));
        assert_eq!(url_shorthand.0.owner(), Some("~lumen-oss"));
        assert_eq!(url_shorthand.0.repo(), "lux");
        assert_eq!(url_shorthand.to_string(), url_shorthand_str.to_string());
    }

    #[tokio::test]
    async fn codeberg_shorthand() {
        let url_shorthand_str = "codeberg:lumen-oss/lux";
        let url_shorthand: RemoteGitUrlShorthand = url_shorthand_str.parse().unwrap();
        assert_eq!(url_shorthand.0.url.host_str(), Some("codeberg.org"));
        assert_eq!(url_shorthand.0.owner(), Some("~lumen-oss"));
        assert_eq!(url_shorthand.0.repo(), "lux");
        assert_eq!(url_shorthand.to_string(), url_shorthand_str.to_string());
    }

    #[tokio::test]
    async fn regular_https_url() {
        let url_shorthand: RemoteGitUrlShorthand =
            "https://github.com/lumen-oss/lux.git".parse().unwrap();
        assert_eq!(url_shorthand.0.url.host_str(), Some("github.com"));
        assert_eq!(url_shorthand.0.owner(), Some("lumen-oss"));
        assert_eq!(url_shorthand.0.repo(), "lux");
        assert_eq!(
            url_shorthand.to_string(),
            "github:lumen-oss/lux".to_string()
        );
    }

    #[tokio::test]
    async fn regular_http_url() {
        let url_shorthand: RemoteGitUrlShorthand =
            "http://github.com/lumen-oss/lux.git".parse().unwrap();
        assert_eq!(url_shorthand.0.url.host_str(), Some("github.com"));
        assert_eq!(url_shorthand.0.owner(), Some("lumen-oss"));
        assert_eq!(url_shorthand.0.repo(), "lux");
        assert_eq!(
            url_shorthand.to_string(),
            "github:lumen-oss/lux".to_string()
        );
    }

    #[tokio::test]
    async fn regular_ssh_url() {
        let url_shorthand: RemoteGitUrlShorthand =
            "ssh://git@github.com/lumen-oss/lux.git".parse().unwrap();
        assert_eq!(url_shorthand.0.url.host_str(), Some("github.com"));
        assert_eq!(url_shorthand.0.owner(), Some("lumen-oss"));
        assert_eq!(url_shorthand.0.repo(), "lux");
        assert_eq!(
            url_shorthand.to_string(),
            "github:lumen-oss/lux".to_string()
        );
    }

    #[tokio::test]
    async fn regular_ftp_url() {
        let url_shorthand: RemoteGitUrlShorthand =
            "ftp://github.com/lumen-oss/lux.git".parse().unwrap();
        assert_eq!(url_shorthand.0.url.host_str(), Some("github.com"));
        assert_eq!(url_shorthand.0.owner(), Some("lumen-oss"));
        assert_eq!(url_shorthand.0.repo(), "lux");
        assert_eq!(
            url_shorthand.to_string(),
            "github:lumen-oss/lux".to_string()
        );
    }

    #[tokio::test]
    async fn regular_ftps_url() {
        let url_shorthand: RemoteGitUrlShorthand =
            "ftps://github.com/lumen-oss/lux.git".parse().unwrap();
        assert_eq!(url_shorthand.0.url.host_str(), Some("github.com"));
        assert_eq!(url_shorthand.0.owner(), Some("lumen-oss"));
        assert_eq!(url_shorthand.0.repo(), "lux");
        assert_eq!(
            url_shorthand.to_string(),
            "github:lumen-oss/lux".to_string()
        );
    }

    #[tokio::test]
    async fn illegal_scheme_url() {
        RemoteGitUrlShorthand::from_str("git+https://github.com/lumen-oss/lux.git")
            .expect_err("git+ handling should be done in an outer layer.");
        RemoteGitUrlShorthand::from_str("file:///lumen-oss/lux.git")
            .expect_err("local filesystems are not supported as a Remote URL.");
        RemoteGitUrlShorthand::from_str("xyz:///lumen-oss/lux.git")
            .expect_err("Unknown schemes are rejected");
    }

    #[tokio::test]
    async fn git_scheme_url() {
        let url_shorthand: RemoteGitUrlShorthand =
            "git://git@github.com/lumen-oss/lux.git".parse().unwrap();
        assert_eq!(url_shorthand.0.url.host_str(), Some("github.com"));
        assert_eq!(url_shorthand.0.owner(), Some("lumen-oss"));
        assert_eq!(url_shorthand.0.repo(), "lux");
        assert_eq!(
            url_shorthand.to_string(),
            "github:lumen-oss/lux".to_string()
        );
    }

    #[tokio::test]
    async fn scp_style_url() {
        let url_str = "git@github.com:lumen-oss/lux.git";
        let url_shorthand: RemoteGitUrlShorthand = url_str.parse().unwrap();
        assert_eq!(url_shorthand.0.url.host_str(), Some("github.com"));
        assert_eq!(url_shorthand.0.owner(), Some("lumen-oss"));
        assert_eq!(url_shorthand.0.repo(), "lux");
    }

    #[tokio::test]
    async fn parse_with_prefix() {
        RemoteGitUrlShorthand::parse_with_prefix("lumen-oss/lux").unwrap_err();
        RemoteGitUrlShorthand::parse_with_prefix("github:lumen-oss/lux").unwrap();
        RemoteGitUrlShorthand::parse_with_prefix("gitlab:lumen-oss/lux").unwrap();
        RemoteGitUrlShorthand::parse_with_prefix("sourcehut:lumen-oss/lux").unwrap();
        RemoteGitUrlShorthand::parse_with_prefix("codeberg:lumen-oss/lux").unwrap();
        RemoteGitUrlShorthand::parse_with_prefix("bla:lumen-oss/lux").unwrap_err();
    }
}
