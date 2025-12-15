use std::fmt::Display;

use mlua::IntoLua;
use serde::{de, Deserialize, Deserializer, Serialize};
use thiserror::Error;
use url::Url;

use crate::manifest::{luanox::LuanoxRemoteDB, luarocks::LuarocksManifest, RemotePackageDB};

const PLUS: &str = "+";

// NOTE: We don't want to expose the internals to the API,
// because adding variants would be a breaking change.

/// The source of a remote package.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone)]
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

impl IntoLua for RemotePackageSource {
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        let table = lua.create_table()?;

        match self {
            RemotePackageSource::LuarocksRockspec(manifest)
            | RemotePackageSource::LuanoxRockspec(manifest) => {
                table.set("rockspec", manifest.to_string())?
            }
            RemotePackageSource::LuarocksSrcRock(manifest) => {
                table.set("src_rock", manifest.to_string())?
            }
            RemotePackageSource::LuarocksBinaryRock(manifest) => {
                table.set("rock", manifest.to_string())?
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
    pub(crate) fn url(self) -> Option<Url> {
        match self {
            Self::LuarocksRockspec(manifest)
            | Self::LuarocksSrcRock(manifest)
            | Self::LuarocksBinaryRock(manifest)
            | Self::LuanoxRockspec(manifest) => Some(manifest.url()),
            Self::RockspecContent(_) | Self::Local => None,
            #[cfg(test)]
            Self::Test => None,
        }
    }
}

impl Display for RemotePackageSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            RemotePackageSource::LuarocksRockspec(manifest) => {
                format!("luarocks_rockspec{PLUS}{manifest}").fmt(f)
            }
            RemotePackageSource::LuanoxRockspec(manifest) => {
                format!("luanox_rockspec{PLUS}{manifest}").fmt(f)
            }
            RemotePackageSource::LuarocksSrcRock(manifest) => {
                format!("luarocks_src_rock{PLUS}{manifest}").fmt(f)
            }
            RemotePackageSource::LuarocksBinaryRock(manifest) => {
                format!("luarocks_rock{PLUS}{manifest}").fmt(f)
            }
            RemotePackageSource::RockspecContent(content) => {
                format!("rockspec{PLUS}{content}").fmt(f)
            }
            RemotePackageSource::Local => "local".fmt(f),
            #[cfg(test)]
            RemotePackageSource::Test => "test+foo_bar".fmt(f),
        }
    }
}

impl Serialize for RemotePackageSource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        format!("{self}").serialize(serializer)
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

impl TryFrom<String> for RemotePackageSource {
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

impl<'de> Deserialize<'de> for RemotePackageSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(de::Error::custom)
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
    fn luarocks_source_roundtrip() {
        let url = Url::parse("https://luarocks.org/").unwrap();
        let source = RemotePackageSource::LuarocksRockspec(url.clone());
        let roundtripped = RemotePackageSource::try_from(format!("{source}")).unwrap();
        assert_eq!(source, roundtripped);
        let source = RemotePackageSource::LuarocksSrcRock(url.clone());
        let roundtripped = RemotePackageSource::try_from(format!("{source}")).unwrap();
        assert_eq!(source, roundtripped);
        let source = RemotePackageSource::LuarocksBinaryRock(url);
        let roundtripped = RemotePackageSource::try_from(format!("{source}")).unwrap();
        assert_eq!(source, roundtripped)
    }

    #[test]
    fn luanox_source_roundtrip() {
        let url = Url::parse("https://beta.luanox.org/").unwrap();
        let source = RemotePackageSource::LuanoxRockspec(url.clone());
        let roundtripped = RemotePackageSource::try_from(format!("{source}")).unwrap();
        assert_eq!(source, roundtripped)
    }

    #[test]
    fn rockspec_source_roundtrip() {
        let source = RemotePackageSource::RockspecContent(LUAROCKS_ROCKSPEC.into());
        let roundtripped = RemotePackageSource::try_from(format!("{source}")).unwrap();
        assert_eq!(source, roundtripped)
    }
}
