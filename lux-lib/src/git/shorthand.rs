use std::{fmt::Display, str::FromStr};

use chumsky::{prelude::*, Parser};
use serde::{de, Deserialize, Deserializer};
use thiserror::Error;

use crate::git::url::{RemoteGitUrl, RemoteGitUrlParseError};

const GITHUB: &str = "github";
const GITLAB: &str = "gitlab";
const SOURCEHUT: &str = "sourcehut";
const CODEBERG: &str = "codeberg";

#[derive(Debug, Error)]
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

// A more lenient parser that defaults to github: if there is not prefix
fn parser<'a>(
) -> impl Parser<'a, &'a str, RemoteGitUrlShorthand, chumsky::extra::Err<Rich<'a, char>>> {
    let git_host_prefix = just(GITHUB)
        .or(just(GITLAB).or(just(SOURCEHUT).or(just(CODEBERG))))
        .then_ignore(just(":"))
        .or_not()
        .map(|prefix| match prefix {
            Some(GITHUB) => GitHost::Github,
            Some(GITLAB) => GitHost::Gitlab,
            Some(SOURCEHUT) => GitHost::Sourcehut,
            Some(CODEBERG) => GitHost::Codeberg,
            _ => GitHost::default(),
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

impl Display for RemoteGitUrlShorthand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.host == "github.com" {
            format!("{}:{}/{}", GITHUB, self.0.owner, self.0.repo)
        } else if self.0.host == "gitlab.com" {
            format!("{}:{}/{}", GITLAB, self.0.owner, self.0.repo)
        } else if self.0.host == "git.sr.ht" {
            format!(
                "{}:{}/{}",
                SOURCEHUT,
                self.0.owner.replace('~', ""),
                self.0.repo
            )
        } else if self.0.host == "codeberg.org" {
            format!(
                "{}:{}/{}",
                CODEBERG,
                self.0.owner.replace('~', ""),
                self.0.repo
            )
        } else {
            format!("{}", self.0)
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
        assert_eq!(url_shorthand.0.owner, "lumen-oss".to_string());
        assert_eq!(url_shorthand.0.repo, "lux".to_string());
    }

    #[tokio::test]
    async fn github_shorthand() {
        let url_shorthand_str = "github:lumen-oss/lux";
        let url_shorthand: RemoteGitUrlShorthand = url_shorthand_str.parse().unwrap();
        assert_eq!(url_shorthand.0.host, "github.com".to_string());
        assert_eq!(url_shorthand.0.owner, "lumen-oss".to_string());
        assert_eq!(url_shorthand.0.repo, "lux".to_string());
        assert_eq!(url_shorthand.to_string(), url_shorthand_str.to_string());
    }

    #[tokio::test]
    async fn gitlab_shorthand() {
        let url_shorthand_str = "gitlab:lumen-oss/lux";
        let url_shorthand: RemoteGitUrlShorthand = url_shorthand_str.parse().unwrap();
        assert_eq!(url_shorthand.0.host, "gitlab.com".to_string());
        assert_eq!(url_shorthand.0.owner, "lumen-oss".to_string());
        assert_eq!(url_shorthand.0.repo, "lux".to_string());
        assert_eq!(url_shorthand.to_string(), url_shorthand_str.to_string());
    }

    #[tokio::test]
    async fn sourcehut_shorthand() {
        let url_shorthand_str = "sourcehut:lumen-oss/lux";
        let url_shorthand: RemoteGitUrlShorthand = url_shorthand_str.parse().unwrap();
        assert_eq!(url_shorthand.0.host, "git.sr.ht".to_string());
        assert_eq!(url_shorthand.0.owner, "~lumen-oss".to_string());
        assert_eq!(url_shorthand.0.repo, "lux".to_string());
        assert_eq!(url_shorthand.to_string(), url_shorthand_str.to_string());
    }

    #[tokio::test]
    async fn codeberg_shorthand() {
        let url_shorthand_str = "codeberg:lumen-oss/lux";
        let url_shorthand: RemoteGitUrlShorthand = url_shorthand_str.parse().unwrap();
        assert_eq!(url_shorthand.0.host, "codeberg.org".to_string());
        assert_eq!(url_shorthand.0.owner, "~lumen-oss".to_string());
        assert_eq!(url_shorthand.0.repo, "lux".to_string());
        assert_eq!(url_shorthand.to_string(), url_shorthand_str.to_string());
    }

    #[tokio::test]
    async fn regular_https_url() {
        let url_shorthand: RemoteGitUrlShorthand =
            "https://github.com/lumen-oss/lux.git".parse().unwrap();
        assert_eq!(url_shorthand.0.host, "github.com".to_string());
        assert_eq!(url_shorthand.0.owner, "lumen-oss".to_string());
        assert_eq!(url_shorthand.0.repo, "lux".to_string());
        assert_eq!(
            url_shorthand.to_string(),
            "github:lumen-oss/lux".to_string()
        );
    }

    #[tokio::test]
    async fn regular_ssh_url() {
        let url_str = "git@github.com:lumen-oss/lux.git";
        let url_shorthand: RemoteGitUrlShorthand = url_str.parse().unwrap();
        assert_eq!(url_shorthand.0.host, "github.com".to_string());
        assert_eq!(
            url_shorthand.0.owner,
            "git@github.com:lumen-oss".to_string(),
        );
        assert_eq!(url_shorthand.0.repo, "lux".to_string());
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
