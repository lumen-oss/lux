use std::fmt::Display;

use miette::Diagnostic;
use serde::{de, Deserialize, Deserializer, Serialize};
use thiserror::Error;
use url::Url;

const PLUS: &str = "+";

// NOTE: We don't want to expose the internals to the API,
// because adding variants would be a breaking change.

/// The source of a remote package.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone)]
pub(crate) enum Source {
    LuarocksRockspec(Url),
    LuarocksSrcRock(Url),
    LuarocksBinaryRock(Url),
    Wally(Url),
    RockspecContent(String),
    Local,
    #[cfg(test)]
    Test,
}

impl Source {
    pub(crate) fn url(self) -> Option<Url> {
        match self {
            Self::LuarocksRockspec(url)
            | Self::LuarocksSrcRock(url)
            | Self::LuarocksBinaryRock(url)
            | Self::Wally(url) => Some(url),
            Self::RockspecContent(_) | Self::Local => None,
            #[cfg(test)]
            Self::Test => None,
        }
    }
}

impl Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            Source::LuarocksRockspec(url) => format!("luarocks_rockspec{PLUS}{url}").fmt(f),
            Source::LuarocksSrcRock(url) => format!("luarocks_src_rock{PLUS}{url}").fmt(f),
            Source::LuarocksBinaryRock(url) => format!("luarocks_rock{PLUS}{url}").fmt(f),
            Source::Wally(url) => format!("wally{PLUS}{url}").fmt(f),
            Source::RockspecContent(content) => format!("rockspec{PLUS}{content}").fmt(f),
            Source::Local => "local".fmt(f),
            #[cfg(test)]
            Source::Test => "test+foo_bar".fmt(f),
        }
    }
}

impl Serialize for Source {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        format!("{self}").serialize(serializer)
    }
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("error parsing remote source URL {0}. Missing URL.")]
    MissingUrl(String),
    #[error("error parsing remote source URL {0}. Expected <source_type>+<url>.")]
    MissingSeparator(String),
    #[error("error parsing remote source type {0}. Expected 'luarocks' or 'rockspec'.")]
    UnknownRemoteSourceType(String),
    #[error(transparent)]
    Url(#[from] url::ParseError),
}

impl TryFrom<String> for Source {
    type Error = Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if let Some(pos) = value.find(PLUS) {
            if let Some(str) = value.get(pos + 1..) {
                let remote_source_type = value[..pos].into();
                match remote_source_type {
                    "luarocks_rockspec" => Ok(Self::LuarocksRockspec(Url::parse(str)?)),
                    "luarocks_src_rock" => Ok(Self::LuarocksSrcRock(Url::parse(str)?)),
                    "luarocks_rock" => Ok(Self::LuarocksBinaryRock(Url::parse(str)?)),
                    "wally" => Ok(Self::Wally(Url::parse(str)?)),
                    "rockspec" => Ok(Self::RockspecContent(str.into())),
                    #[cfg(test)]
                    "test" => Ok(Self::Test),
                    _ => Err(Error::UnknownRemoteSourceType(remote_source_type.into())),
                }
            } else {
                Err(Error::MissingUrl(value))
            }
        } else if value == "local" {
            Ok(Self::Local)
        } else {
            Err(Error::MissingSeparator(value))
        }
    }
}

impl<'de> Deserialize<'de> for Source {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LUAROCKS_ROCKSPEC: &str = "
rockspec_format = \"3.0\"
package = 'luarocks'
version = '3.11.1-1'
source = {
   url = 'git+https://github.com/luarocks/luarocks',
   tag = 'v3.11.1'
}
";

    #[test]
    fn luarocks_source_roundtrip() {
        let url = Url::parse("https://luarocks.org/").unwrap();
        let source = Source::LuarocksRockspec(url.clone());
        let roundtripped = Source::try_from(format!("{source}")).unwrap();
        assert_eq!(source, roundtripped);
        let source = Source::LuarocksSrcRock(url.clone());
        let roundtripped = Source::try_from(format!("{source}")).unwrap();
        assert_eq!(source, roundtripped);
        let source = Source::LuarocksBinaryRock(url);
        let roundtripped = Source::try_from(format!("{source}")).unwrap();
        assert_eq!(source, roundtripped)
    }

    #[test]
    fn wally_source_roundtrip() {
        let url = Url::parse("https://github.com/UpliftGames/wally-index").unwrap();
        let source = Source::Wally(url);
        let roundtripped = Source::try_from(format!("{source}")).unwrap();
        assert_eq!(source, roundtripped);
    }

    #[test]
    fn rockspec_source_roundtrip() {
        let source = Source::RockspecContent(LUAROCKS_ROCKSPEC.into());
        let roundtripped = Source::try_from(format!("{source}")).unwrap();
        assert_eq!(source, roundtripped)
    }
}
