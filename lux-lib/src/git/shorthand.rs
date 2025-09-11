use std::{fmt::Display, str::FromStr};

use chumsky::{prelude::*, Parser};
use git_url_parse::{GitUrl, GitUrlParseError};
use serde::{de, Deserialize, Deserializer};
use thiserror::Error;

const GITHUB: &str = "github";
const GITLAB: &str = "gitlab";
const SOURCEHUT: &str = "sourcehut";
const CODEBERG: &str = "codeberg";

#[derive(Debug, Error)]
#[error("error parsing git source: {0:#?}")]
pub struct ParseError(Vec<String>);

/// Helper for parsing Git URLs from shorthands, e.g. "gitlab:owner/repo"
#[derive(Debug, Clone)]
pub struct GitUrlShorthand(GitUrl);

impl GitUrlShorthand {
    pub fn parse_with_prefix(s: &str) -> Result<Self, ParseError> {
        prefix_parser()
            .parse(s)
            .into_result()
            .map_err(|err| ParseError(err.into_iter().map(|e| e.to_string()).collect()))
    }
    pub fn repo_name() {}
}

impl FromStr for GitUrlShorthand {
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

impl<'de> Deserialize<'de> for GitUrlShorthand {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

impl From<GitUrl> for GitUrlShorthand {
    fn from(value: GitUrl) -> Self {
        Self(value)
    }
}

impl From<GitUrlShorthand> for GitUrl {
    fn from(value: GitUrlShorthand) -> Self {
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
) -> Result<GitUrlShorthand, GitUrlParseError> {
    let url_str = match host {
        GitHost::Github => format!("https://github.com/{owner}/{repo}.git"),
        GitHost::Gitlab => format!("https://gitlab.com/{owner}/{repo}.git"),
        GitHost::Sourcehut => format!("https://git.sr.ht/~{owner}/{repo}"),
        GitHost::Codeberg => format!("https://codeberg.org/~{owner}/{repo}.git"),
    };
    let url = url_str.parse()?;
    Ok(GitUrlShorthand(url))
}

fn to_tuple<T>(v: Vec<T>) -> (T, T)
where
    T: Clone,
{
    (v[0].clone(), v[1].clone())
}

// A parser that expects a prefix
fn prefix_parser<'a>(
) -> impl Parser<'a, &'a str, GitUrlShorthand, chumsky::extra::Err<Rich<'a, char>>> {
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
fn parser<'a>() -> impl Parser<'a, &'a str, GitUrlShorthand, chumsky::extra::Err<Rich<'a, char>>> {
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

impl Display for GitUrlShorthand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.0.host, &self.0.owner) {
            (Some(host), Some(owner)) if host == "github.com" => {
                format!("{}:{}/{}", GITHUB, owner, self.0.name)
            }
            (Some(host), Some(owner)) if host == "gitlab.com" => {
                format!("{}:{}/{}", GITLAB, owner, self.0.name)
            }
            (Some(host), Some(owner)) if host == "git.sr.ht" => {
                format!("{}:{}/{}", SOURCEHUT, owner.replace('~', ""), self.0.name)
            }
            (Some(host), Some(owner)) if host == "codeberg.org" => {
                format!("{}:{}/{}", CODEBERG, owner.replace('~', ""), self.0.name)
            }
            _ => format!("{}", self.0),
        }
        .fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn owner_repo_shorthand() {
        let url_shorthand: GitUrlShorthand = "lumen-oss/lux".parse().unwrap();
        assert_eq!(url_shorthand.0.owner, Some("lumen-oss".to_string()));
        assert_eq!(url_shorthand.0.name, "lux".to_string());
    }

    #[tokio::test]
    async fn github_shorthand() {
        let url_shorthand_str = "github:lumen-oss/lux";
        let url_shorthand: GitUrlShorthand = url_shorthand_str.parse().unwrap();
        assert_eq!(url_shorthand.0.host, Some("github.com".to_string()));
        assert_eq!(url_shorthand.0.owner, Some("lumen-oss".to_string()));
        assert_eq!(url_shorthand.0.name, "lux".to_string());
        assert_eq!(url_shorthand.to_string(), url_shorthand_str.to_string());
    }

    #[tokio::test]
    async fn gitlab_shorthand() {
        let url_shorthand_str = "gitlab:lumen-oss/lux";
        let url_shorthand: GitUrlShorthand = url_shorthand_str.parse().unwrap();
        assert_eq!(url_shorthand.0.host, Some("gitlab.com".to_string()));
        assert_eq!(url_shorthand.0.owner, Some("lumen-oss".to_string()));
        assert_eq!(url_shorthand.0.name, "lux".to_string());
        assert_eq!(url_shorthand.to_string(), url_shorthand_str.to_string());
    }

    #[tokio::test]
    async fn sourcehut_shorthand() {
        let url_shorthand_str = "sourcehut:lumen-oss/lux";
        let url_shorthand: GitUrlShorthand = url_shorthand_str.parse().unwrap();
        assert_eq!(url_shorthand.0.host, Some("git.sr.ht".to_string()));
        assert_eq!(url_shorthand.0.owner, Some("~lumen-oss".to_string()));
        assert_eq!(url_shorthand.0.name, "lux".to_string());
        assert_eq!(url_shorthand.to_string(), url_shorthand_str.to_string());
    }

    #[tokio::test]
    async fn codeberg_shorthand() {
        let url_shorthand_str = "codeberg:lumen-oss/lux";
        let url_shorthand: GitUrlShorthand = url_shorthand_str.parse().unwrap();
        assert_eq!(url_shorthand.0.host, Some("codeberg.org".to_string()));
        assert_eq!(url_shorthand.0.owner, Some("~lumen-oss".to_string()));
        assert_eq!(url_shorthand.0.name, "lux".to_string());
        assert_eq!(url_shorthand.to_string(), url_shorthand_str.to_string());
    }

    #[tokio::test]
    async fn regular_https_url() {
        let url_shorthand: GitUrlShorthand =
            "https://github.com/lumen-oss/lux.git".parse().unwrap();
        assert_eq!(url_shorthand.0.host, Some("github.com".to_string()));
        assert_eq!(url_shorthand.0.owner, Some("lumen-oss".to_string()));
        assert_eq!(url_shorthand.0.name, "lux".to_string());
        assert_eq!(
            url_shorthand.to_string(),
            "github:lumen-oss/lux".to_string()
        );
    }

    #[tokio::test]
    async fn regular_ssh_url() {
        let url_str = "git@github.com:lumen-oss/lux.git";
        let url_shorthand: GitUrlShorthand = url_str.parse().unwrap();
        assert_eq!(url_shorthand.0.host, Some("github.com".to_string()));
        assert_eq!(
            url_shorthand.0.owner,
            Some("git@github.com:lumen-oss".to_string())
        );
        assert_eq!(url_shorthand.0.name, "lux".to_string());
    }

    #[tokio::test]
    async fn parse_with_prefix() {
        GitUrlShorthand::parse_with_prefix("lumen-oss/lux").unwrap_err();
        GitUrlShorthand::parse_with_prefix("github:lumen-oss/lux").unwrap();
        GitUrlShorthand::parse_with_prefix("gitlab:lumen-oss/lux").unwrap();
        GitUrlShorthand::parse_with_prefix("sourcehut:lumen-oss/lux").unwrap();
        GitUrlShorthand::parse_with_prefix("codeberg:lumen-oss/lux").unwrap();
        GitUrlShorthand::parse_with_prefix("bla:lumen-oss/lux").unwrap_err();
    }
}
