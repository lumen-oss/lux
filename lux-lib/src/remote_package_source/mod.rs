use std::{fmt::Display, hash::Hash};

use mlua::IntoLua;
use serde::{
    de::{self},
    Deserialize, Deserializer, Serialize,
};
use thiserror::Error;
use url::Url;

use crate::{
    config::Config,
    manifest::{
        luanox::LuanoxRemoteDB,
        luarocks::{LuarocksManifest, ManifestError},
        RemotePackageDB,
    },
    progress::{Progress, ProgressBar},
};

const PLUS: &str = "+";

// NOTE: We don't want to expose the internals to the API,
// because adding variants would be a breaking change.

/// The source of a remote package.
#[derive(Debug, Clone)]
pub(crate) enum RemotePackageSource {
    // Remote luarocks-compatible source
    LuarocksRockspec(LuarocksManifest),
    LuarocksSrcRock(LuarocksManifest),
    LuarocksBinaryRock(LuarocksManifest),
    // Remote luanox-compatible source
    LuanoxRockspec(LuanoxRemoteDB),
    RockspecContent(String),
    Local,
    #[cfg(test)]
    Test,
}

impl Hash for RemotePackageSource {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            RemotePackageSource::LuarocksRockspec(manifest)
            | RemotePackageSource::LuarocksSrcRock(manifest)
            | RemotePackageSource::LuarocksBinaryRock(manifest) => {
                manifest.url().hash(state);
            }
            RemotePackageSource::LuanoxRockspec(remote) => {
                remote.url().hash(state);
            }
            RemotePackageSource::RockspecContent(content) => {
                content.hash(state);
            }
            RemotePackageSource::Local => {
                "local".hash(state);
            }
            #[cfg(test)]
            RemotePackageSource::Test => {
                "test".hash(state);
            }
        }
    }
}

impl PartialEq for RemotePackageSource {
    fn eq(&self, other: &Self) -> bool {
        self.url() == other.url()
    }
}

impl Eq for RemotePackageSource {}

impl PartialOrd for RemotePackageSource {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.url().cmp(&other.url()))
    }
}

impl Ord for RemotePackageSource {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.url().cmp(&other.url())
    }
}

impl IntoLua for RemotePackageSource {
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        let table = lua.create_table()?;

        match self {
            RemotePackageSource::LuarocksRockspec(manifest) => {
                table.set("rockspec", manifest.url().to_string())?
            }
            RemotePackageSource::LuarocksSrcRock(manifest) => {
                table.set("src_rock", manifest.url().to_string())?
            }
            RemotePackageSource::LuarocksBinaryRock(manifest) => {
                table.set("rock", manifest.url().to_string())?
            }
            RemotePackageSource::LuanoxRockspec(remote) => {
                table.set("rockspec", remote.url().to_string())?
            }
            RemotePackageSource::RockspecContent(content) => {
                table.set("rockspec_content", content)?
            }
            RemotePackageSource::Local => table.set("local", true)?,
            #[cfg(test)]
            RemotePackageSource::Test => unreachable!(),
        };

        Ok(mlua::Value::Table(table))
    }
}

impl RemotePackageSource {
    pub(crate) fn url(&self) -> Option<Url> {
        match self {
            Self::LuarocksRockspec(manifest)
            | Self::LuarocksSrcRock(manifest)
            | Self::LuarocksBinaryRock(manifest) => Some(manifest.url().clone()),
            Self::LuanoxRockspec(remote) => Some(remote.url().clone()),
            Self::RockspecContent(_) | Self::Local => None,
            #[cfg(test)]
            Self::Test => None,
        }
    }

    pub(crate) async fn from_lockfile_url(
        src: IntermediateRemotePackageSource,
        config: &Config,
        progress: &Progress<ProgressBar>,
    ) -> Result<Self, ManifestError> {
        match src {
            IntermediateRemotePackageSource::LuarocksRockspec(url)
            | IntermediateRemotePackageSource::LuarocksSrcRock(url)
            | IntermediateRemotePackageSource::LuarocksBinaryRock(url) => {
                Ok(RemotePackageSource::LuarocksRockspec(
                    LuarocksManifest::from_config(url, config, progress).await?,
                ))
            }
            IntermediateRemotePackageSource::LuanoxRockspec(url) => Ok(
                RemotePackageSource::LuanoxRockspec(LuanoxRemoteDB::new(url)),
            ),
            IntermediateRemotePackageSource::RockspecContent(_) => todo!(),
            IntermediateRemotePackageSource::Local => todo!(),
            #[cfg(test)]
            IntermediateRemotePackageSource::Test => Ok(RemotePackageSource::Test),
        }
    }
}

impl Serialize for RemotePackageSource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        format!(
            "{}",
            self.clone()
                .url()
                .ok_or_else(|| serde::ser::Error::custom("no URL"))?
        )
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RemotePackageSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let intermediate = IntermediateRemotePackageSource::deserialize(deserializer)?;
        Err(de::Error::custom(format!(
            "cannot deserialize RemotePackageSource from intermediate source: {intermediate:?}"
        )))
    }
}

#[derive(Error, Debug)]
pub enum RemotePackageSourceError {
    #[error("error parsing remote source URL {0}. Missing URL.")]
    MissingUrl(String),
    #[error("error parsing remote source URL {0}. Expected <source_type>+<url>.")]
    MissingSeparator(String),
    #[error("error parsing remote source type {0}. Expected 'luarocks' or 'rockspec'.")]
    UnknownRemoteSourceType(String),
    #[error(transparent)]
    Url(#[from] url::ParseError),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum IntermediateRemotePackageSource {
    LuarocksRockspec(Url),
    LuarocksSrcRock(Url),
    LuarocksBinaryRock(Url),
    LuanoxRockspec(Url),
    RockspecContent(String),
    Local,
    #[cfg(test)]
    Test,
}

impl TryFrom<String> for IntermediateRemotePackageSource {
    type Error = RemotePackageSourceError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if let Some(pos) = value.find(PLUS) {
            if let Some(str) = value.get(pos + 1..) {
                let remote_source_type = value[..pos].into();
                match remote_source_type {
                    "luarocks_rockspec" => Ok(Self::LuarocksRockspec(Url::parse(str)?)),
                    "luarocks_src_rock" => Ok(Self::LuarocksSrcRock(Url::parse(str)?)),
                    "luarocks_rock" => Ok(Self::LuarocksBinaryRock(Url::parse(str)?)),
                    "luanox_rockspec" => Ok(Self::LuanoxRockspec(Url::parse(str)?)),
                    "rockspec" => Ok(Self::RockspecContent(str.into())),
                    _ => Err(RemotePackageSourceError::UnknownRemoteSourceType(
                        remote_source_type.into(),
                    )),
                }
            } else {
                Err(RemotePackageSourceError::MissingUrl(value))
            }
        } else if value == "local" {
            Ok(Self::Local)
        } else {
            Err(RemotePackageSourceError::MissingSeparator(value))
        }
    }
}

impl<'de> Deserialize<'de> for IntermediateRemotePackageSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(de::Error::custom)
    }
}

impl Display for IntermediateRemotePackageSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            IntermediateRemotePackageSource::LuarocksRockspec(manifest) => {
                format!("luarocks_rockspec{PLUS}{manifest}").fmt(f)
            }
            IntermediateRemotePackageSource::LuanoxRockspec(manifest) => {
                format!("luanox_rockspec{PLUS}{manifest}").fmt(f)
            }
            IntermediateRemotePackageSource::LuarocksSrcRock(manifest) => {
                format!("luarocks_src_rock{PLUS}{manifest}").fmt(f)
            }
            IntermediateRemotePackageSource::LuarocksBinaryRock(manifest) => {
                format!("luarocks_rock{PLUS}{manifest}").fmt(f)
            }
            IntermediateRemotePackageSource::RockspecContent(content) => {
                format!("rockspec{PLUS}{content}").fmt(f)
            }
            IntermediateRemotePackageSource::Local => "local".fmt(f),
            #[cfg(test)]
            IntermediateRemotePackageSource::Test => "test+foo_bar".fmt(f),
        }
    }
}

#[cfg(test)]
mod test {
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
    fn luarocks_intermediate_source_roundtrip() {
        let url = Url::parse("https://luarocks.org/").unwrap();
        let source = IntermediateRemotePackageSource::LuarocksRockspec(url.clone());
        let roundtripped = IntermediateRemotePackageSource::try_from(format!("{source}")).unwrap();
        assert_eq!(source, roundtripped);
        let source = IntermediateRemotePackageSource::LuarocksSrcRock(url.clone());
        let roundtripped = IntermediateRemotePackageSource::try_from(format!("{source}")).unwrap();
        assert_eq!(source, roundtripped);
        let source = IntermediateRemotePackageSource::LuarocksBinaryRock(url);
        let roundtripped = IntermediateRemotePackageSource::try_from(format!("{source}")).unwrap();
        assert_eq!(source, roundtripped)
    }

    #[test]
    fn luanox_intermediate_source_roundtrip() {
        let url = Url::parse("https://beta.luanox.org/").unwrap();
        let source = IntermediateRemotePackageSource::LuanoxRockspec(url.clone());
        let roundtripped = IntermediateRemotePackageSource::try_from(format!("{source}")).unwrap();
        assert_eq!(source, roundtripped)
    }

    #[test]
    fn rockspec_intermediate_source_roundtrip() {
        let source = IntermediateRemotePackageSource::RockspecContent(LUAROCKS_ROCKSPEC.into());
        let roundtripped = IntermediateRemotePackageSource::try_from(format!("{source}")).unwrap();
        assert_eq!(source, roundtripped)
    }
}
