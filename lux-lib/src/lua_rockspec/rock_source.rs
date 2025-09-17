use git_url_parse::{GitUrl, GitUrlParseError};
use mlua::{FromLua, IntoLua, Lua, UserData, Value};
use path_slash::PathBufExt;
use reqwest::Url;
use serde::{de, Deserialize, Deserializer};
use std::{convert::Infallible, fs, io, ops::Deref, path::PathBuf, str::FromStr};
use thiserror::Error;

use crate::git::GitSource;

use super::{
    DisplayAsLuaKV, DisplayLuaKV, DisplayLuaValue, FromPlatformOverridable, PartialOverride,
    PerPlatform, PerPlatformWrapper, PlatformOverridable,
};

#[derive(Default, Deserialize, Clone, Debug, PartialEq)]
pub struct LocalRockSource {
    pub archive_name: Option<PathBuf>,
    pub unpack_dir: Option<PathBuf>,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct RemoteRockSource {
    pub(crate) local: LocalRockSource,
    pub source_spec: RockSourceSpec,
}

impl From<RockSourceSpec> for RemoteRockSource {
    fn from(source_spec: RockSourceSpec) -> Self {
        Self {
            local: LocalRockSource::default(),
            source_spec,
        }
    }
}

impl UserData for RemoteRockSource {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("source_spec", |_, this, _: ()| Ok(this.source_spec.clone()));
        methods.add_method("archive_name", |_, this, _: ()| {
            Ok(this.local.archive_name.clone())
        });
        methods.add_method("unpack_dir", |_, this, _: ()| {
            Ok(this.local.unpack_dir.clone())
        });
    }
}

impl Deref for RemoteRockSource {
    type Target = LocalRockSource;

    fn deref(&self) -> &Self::Target {
        &self.local
    }
}

#[derive(Error, Debug)]
pub enum RockSourceError {
    #[error("invalid rockspec source field combination")]
    InvalidCombination,
    #[error(transparent)]
    SourceUrl(#[from] SourceUrlError),
    #[error("source URL missing")]
    SourceUrlMissing,
}

impl FromPlatformOverridable<RockSourceInternal, Self> for LocalRockSource {
    type Err = Infallible;

    fn from_platform_overridable(internal: RockSourceInternal) -> Result<Self, Self::Err> {
        Ok(LocalRockSource {
            archive_name: internal.file,
            unpack_dir: internal.dir,
        })
    }
}

impl FromPlatformOverridable<RockSourceInternal, Self> for RemoteRockSource {
    type Err = RockSourceError;

    fn from_platform_overridable(internal: RockSourceInternal) -> Result<Self, Self::Err> {
        let local = LocalRockSource::from_platform_overridable(internal.clone()).unwrap();

        // The rockspec.source table allows invalid combinations
        // This ensures that invalid combinations are caught while parsing.
        let url = SourceUrl::from_str(&internal.url.ok_or(RockSourceError::SourceUrlMissing)?)?;

        let source_spec = match (url, internal.tag, internal.branch) {
            (source, None, None) => Ok(RockSourceSpec::default_from_source_url(source)),
            (SourceUrl::Git(url), Some(tag), None) => Ok(RockSourceSpec::Git(GitSource {
                url,
                checkout_ref: Some(tag),
            })),
            (SourceUrl::Git(url), None, Some(branch)) => Ok(RockSourceSpec::Git(GitSource {
                url,
                checkout_ref: Some(branch),
            })),
            _ => Err(RockSourceError::InvalidCombination),
        }?;

        Ok(RemoteRockSource { source_spec, local })
    }
}

impl FromLua for PerPlatform<RemoteRockSource> {
    fn from_lua(value: Value, lua: &Lua) -> mlua::Result<Self> {
        let wrapper = PerPlatformWrapper::from_lua(value, lua)?;
        Ok(wrapper.un_per_platform)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum RockSourceSpec {
    Git(GitSource),
    File(PathBuf),
    Url(Url),
}

impl IntoLua for RockSourceSpec {
    fn into_lua(self, lua: &Lua) -> mlua::Result<Value> {
        let table = lua.create_table()?;

        match self {
            RockSourceSpec::Git(git) => {
                table.set("git", git.into_lua(lua)?)?;
            }
            RockSourceSpec::File(path) => {
                table.set("file", path.to_string_lossy().to_string())?;
            }
            RockSourceSpec::Url(url) => {
                table.set("url", url.to_string())?;
            }
        };

        Ok(Value::Table(table))
    }
}

impl RockSourceSpec {
    fn default_from_source_url(url: SourceUrl) -> Self {
        match url {
            SourceUrl::File(path) => Self::File(path),
            SourceUrl::Url(url) => Self::Url(url),
            SourceUrl::Git(url) => Self::Git(GitSource {
                url,
                checkout_ref: None,
            }),
        }
    }
}

impl<'de> Deserialize<'de> for RockSourceSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let url = String::deserialize(deserializer)?;
        Ok(RockSourceSpec::default_from_source_url(
            url.parse().map_err(de::Error::custom)?,
        ))
    }
}

impl DisplayAsLuaKV for RockSourceSpec {
    fn display_lua(&self) -> DisplayLuaKV {
        match self {
            RockSourceSpec::Git(git_source) => git_source.display_lua(),
            RockSourceSpec::File(path) => {
                let mut source_tbl = Vec::new();
                source_tbl.push(DisplayLuaKV {
                    key: "url".to_string(),
                    value: DisplayLuaValue::String(format!("file:://{}", path.display())),
                });
                DisplayLuaKV {
                    key: "source".to_string(),
                    value: DisplayLuaValue::Table(source_tbl),
                }
            }
            RockSourceSpec::Url(url) => {
                let mut source_tbl = Vec::new();
                source_tbl.push(DisplayLuaKV {
                    key: "url".to_string(),
                    value: DisplayLuaValue::String(format!("{url}")),
                });
                DisplayLuaKV {
                    key: "source".to_string(),
                    value: DisplayLuaValue::Table(source_tbl),
                }
            }
        }
    }
}

/// Used as a helper for Deserialize,
/// because the Rockspec schema allows invalid rockspecs (╯°□°)╯︵ ┻━┻
#[derive(Debug, PartialEq, Deserialize, Clone, Default)]
pub(crate) struct RockSourceInternal {
    #[serde(default)]
    pub(crate) url: Option<String>,
    pub(crate) file: Option<PathBuf>,
    pub(crate) dir: Option<PathBuf>,
    pub(crate) tag: Option<String>,
    pub(crate) branch: Option<String>,
}

impl PartialOverride for RockSourceInternal {
    type Err = Infallible;

    fn apply_overrides(&self, override_spec: &Self) -> Result<Self, Self::Err> {
        Ok(Self {
            url: override_opt(override_spec.url.as_ref(), self.url.as_ref()),
            file: override_opt(override_spec.file.as_ref(), self.file.as_ref()),
            dir: override_opt(override_spec.dir.as_ref(), self.dir.as_ref()),
            tag: match &override_spec.branch {
                None => override_opt(override_spec.tag.as_ref(), self.tag.as_ref()),
                _ => None,
            },
            branch: match &override_spec.tag {
                None => override_opt(override_spec.branch.as_ref(), self.branch.as_ref()),
                _ => None,
            },
        })
    }
}

impl DisplayAsLuaKV for RockSourceInternal {
    fn display_lua(&self) -> DisplayLuaKV {
        let mut result = Vec::new();

        if let Some(url) = &self.url {
            result.push(DisplayLuaKV {
                key: "url".to_string(),
                value: DisplayLuaValue::String(url.clone()),
            });
        }
        if let Some(file) = &self.file {
            result.push(DisplayLuaKV {
                key: "file".to_string(),
                value: DisplayLuaValue::String(file.to_slash_lossy().to_string()),
            });
        }
        if let Some(dir) = &self.dir {
            result.push(DisplayLuaKV {
                key: "dir".to_string(),
                value: DisplayLuaValue::String(dir.to_slash_lossy().to_string()),
            });
        }
        if let Some(tag) = &self.tag {
            result.push(DisplayLuaKV {
                key: "tag".to_string(),
                value: DisplayLuaValue::String(tag.clone()),
            });
        }
        if let Some(branch) = &self.branch {
            result.push(DisplayLuaKV {
                key: "branch".to_string(),
                value: DisplayLuaValue::String(branch.clone()),
            });
        }

        DisplayLuaKV {
            key: "source".to_string(),
            value: DisplayLuaValue::Table(result),
        }
    }
}

#[derive(Error, Debug)]
#[error("missing source")]
pub struct RockSourceMissingSource;

impl PlatformOverridable for RockSourceInternal {
    type Err = RockSourceMissingSource;

    fn on_nil<T>() -> Result<PerPlatform<T>, <Self as PlatformOverridable>::Err>
    where
        T: PlatformOverridable,
    {
        Err(RockSourceMissingSource)
    }
}

fn override_opt<T: Clone>(override_opt: Option<&T>, base: Option<&T>) -> Option<T> {
    override_opt.or(base).cloned()
}

/// Internal helper for parsing
#[derive(Debug, PartialEq, Clone)]
pub(crate) enum SourceUrl {
    /// For URLs in the local filesystem
    File(PathBuf),
    /// Web URLs
    Url(Url),
    /// For the Git source control manager
    Git(GitUrl),
}

#[derive(Error, Debug)]
#[error("failed to parse source url: {0}")]
pub enum SourceUrlError {
    Io(#[from] io::Error),
    Git(#[from] GitUrlParseError),
    Url(#[source] <Url as FromStr>::Err),
    #[error("lux does not support rockspecs with CVS sources.")]
    CVS,
    #[error("lux does not support rockspecs with mercurial sources.")]
    Mercurial,
    #[error("lux does not support rockspecs with SSCM sources.")]
    SSCM,
    #[error("lux does not support rockspecs with SVN sources.")]
    SVN,
    #[error("unsupported source URL prefix: '{0}+' in URL {1}")]
    UnsupportedPrefix(String, String),
    #[error("unsupported source URL: {0}")]
    Unsupported(String),
}

impl FromStr for SourceUrl {
    type Err = SourceUrlError;

    fn from_str(str: &str) -> Result<Self, Self::Err> {
        match str.split_once("+") {
            Some(("git" | "gitrec", url)) => Ok(Self::Git(url.parse()?)),
            Some((prefix, _)) => Err(SourceUrlError::UnsupportedPrefix(
                prefix.to_string(),
                str.to_string(),
            )),
            None => match str {
                s if s.starts_with("file://") => {
                    let path_buf: PathBuf = s.trim_start_matches("file://").into();
                    let path = fs::canonicalize(&path_buf)?;
                    Ok(Self::File(path))
                }
                s if s.starts_with("git://") => {
                    Ok(Self::Git(s.replacen("git", "https", 1).parse()?))
                }
                s if s.ends_with(".git") => Ok(Self::Git(s.parse()?)),
                s if starts_with_any(s, ["https://", "http://", "ftp://"].into()) => {
                    Ok(Self::Url(s.parse().map_err(SourceUrlError::Url)?))
                }
                s if s.starts_with("cvs://") => Err(SourceUrlError::CVS),
                s if starts_with_any(
                    s,
                    ["hg://", "hg+http://", "hg+https://", "hg+ssh://"].into(),
                ) =>
                {
                    Err(SourceUrlError::Mercurial)
                }
                s if s.starts_with("sscm://") => Err(SourceUrlError::SSCM),
                s if s.starts_with("svn://") => Err(SourceUrlError::SVN),
                s => Err(SourceUrlError::Unsupported(s.to_string())),
            },
        }
    }
}

impl<'de> Deserialize<'de> for SourceUrl {
    fn deserialize<D>(deserializer: D) -> Result<SourceUrl, D::Error>
    where
        D: Deserializer<'de>,
    {
        SourceUrl::from_str(&String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

fn starts_with_any(str: &str, prefixes: Vec<&str>) -> bool {
    prefixes.iter().any(|&prefix| str.starts_with(prefix))
}

#[cfg(test)]
mod tests {

    use assert_fs::TempDir;

    use super::*;

    #[tokio::test]
    async fn parse_source_url() {
        let dir = TempDir::new().unwrap();
        let url: SourceUrl = format!("file://{}", dir.to_string_lossy()).parse().unwrap();
        assert_eq!(url, SourceUrl::File(dir.path().to_path_buf()));
        let url: SourceUrl = "ftp://example.com/foo".parse().unwrap();
        assert!(matches!(url, SourceUrl::Url { .. }));
        let url: SourceUrl = "git://example.com/foo".parse().unwrap();
        assert!(matches!(url, SourceUrl::Git { .. }));
        let url: SourceUrl = "git+file://example.com/foo".parse().unwrap();
        assert!(matches!(url, SourceUrl::Git { .. }));
        let url: SourceUrl = "git+http://example.com/foo".parse().unwrap();
        assert!(matches!(url, SourceUrl::Git { .. }));
        let url: SourceUrl = "git+https://example.com/foo".parse().unwrap();
        assert!(matches!(url, SourceUrl::Git { .. }));
        let url: SourceUrl = "git+ssh://example.com/foo".parse().unwrap();
        assert!(matches!(url, SourceUrl::Git { .. }));
        let url: SourceUrl = "gitrec+https://example.com/foo".parse().unwrap();
        assert!(matches!(url, SourceUrl::Git { .. }));
        let url: SourceUrl = "https://example.com/foo".parse().unwrap();
        assert!(matches!(url, SourceUrl::Url { .. }));
        let url: SourceUrl = "http://example.com/foo".parse().unwrap();
        assert!(matches!(url, SourceUrl::Url { .. }));
    }
}
