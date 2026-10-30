use super::platform;
use crate::{
    git::{
        url::{RemoteGitUrl, RemoteGitUrlParseError},
        GitSource,
    },
    rockspec::platform::per_platform_from_intermediate,
    wally,
};

pub use crate::git::GitRef;
use miette::Diagnostic;
use reqwest::Url;
use serde::{de, Deserialize, Deserializer};
use std::{
    convert::Infallible,
    fmt::Display,
    fs, io,
    ops::Deref,
    path::{Path, PathBuf},
    str::FromStr,
};
use thiserror::Error;

use super::{display, PerPlatform};

/// Specifies the source of a rock that has been fetched
#[derive(Default, Deserialize, Clone, Debug, PartialEq)]
pub struct Local {
    pub archive_name: Option<PathBuf>,
    pub unpack_dir: Option<PathBuf>,
}

/// Specifies the source of a remote rock to be fetched
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Remote {
    pub(crate) local: Local,
    pub source_spec: Spec,
}

impl Remote {
    /// Builds a remote rock source, normalising a Git rock's `source.dir` to its
    /// effective source root.
    ///
    /// The repository name is the final path component of the Git URL, with any
    /// `.git` suffix removed.
    ///
    /// Luarocks clones Git sources into a `<repository>` directory and resolves
    /// `source.dir` relative to its parent.
    /// Lux clones directly into the fetch root, which results in the same
    /// NAR hash as is used by `fetchgit` (Nix).
    ///
    /// For a repository named `foo`:
    ///
    /// | input `source.dir` | resulting directory |
    /// |--------------------|---------------------|
    /// | `nil`              | `.`                 |
    /// | `foo`              | `.`                 |
    /// | `foo/src`          | `src`               |
    /// | `src`              | `src`               |
    pub(crate) fn new(mut local: Local, source_spec: Spec) -> Self {
        if let Spec::Git(git) = &source_spec {
            let repo = git.url.repo();
            let normalised = match local.unpack_dir.take() {
                Some(dir) if dir == Path::new(repo) => ".".into(),
                Some(dir) => match dir.strip_prefix(repo) {
                    Ok(stripped) => stripped.to_path_buf(),
                    Err(_) => dir,
                },
                None => PathBuf::from("."),
            };
            local.unpack_dir = Some(normalised);
        }
        Self { local, source_spec }
    }
}

impl From<Spec> for Remote {
    fn from(source_spec: Spec) -> Self {
        Self::new(Local::default(), source_spec)
    }
}

impl Deref for Remote {
    type Target = Local;

    fn deref(&self) -> &Self::Target {
        &self.local
    }
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("invalid rockspec source field combination")]
    InvalidCombination,
    #[error(transparent)]
    #[diagnostic(transparent)]
    SourceUrl(#[from] UrlError),
    #[error("source URL missing")]
    SourceUrlMissing,
}

impl From<internal::Spec> for Local {
    fn from(internal: internal::Spec) -> Self {
        Local {
            archive_name: internal.file,
            unpack_dir: internal.dir,
        }
    }
}

impl TryFrom<internal::Spec> for Remote {
    type Error = Error;

    fn try_from(internal: internal::Spec) -> Result<Self, Self::Error> {
        let local = Local::from(internal.clone());

        // The rockspec.source table allows invalid combinations
        // This ensures that invalid combinations are caught while parsing.
        let url = internal::SourceUrl::from_str(&internal.url.ok_or(Error::SourceUrlMissing)?)?;

        let source_spec = match (url, internal.tag, internal.branch) {
            (source, None, None) => Ok(Spec::default_from_source_url(source)),
            (internal::SourceUrl::Git(url), Some(tag), None) => Ok(Spec::Git(GitSource {
                url,
                git_ref: Some(GitRef::Tag(tag)),
            })),
            (internal::SourceUrl::Git(url), None, Some(branch)) => Ok(Spec::Git(GitSource {
                url,
                git_ref: Some(GitRef::Branch(branch)),
            })),
            _ => Err(Error::InvalidCombination),
        }?;

        Ok(Remote::new(local, source_spec))
    }
}

impl<'de> Deserialize<'de> for PerPlatform<Remote> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        per_platform_from_intermediate::<_, internal::Spec, _>(deserializer)
    }
}

/// Specifies a rock's source
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum Spec {
    Git(GitSource),
    File(PathBuf),
    Url(Url),
    Wally(wally::package::Req),
}

impl Spec {
    fn default_from_source_url(url: internal::SourceUrl) -> Self {
        match url {
            internal::SourceUrl::File(path) => Self::File(path),
            internal::SourceUrl::Url(url) => Self::Url(url),
            internal::SourceUrl::Git(url) => Self::Git(GitSource { url, git_ref: None }),
        }
    }
}

impl Display for Spec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Spec::Git(git_source) => git_source.fmt(f),
            Spec::File(path_buf) => path_buf.display().fmt(f),
            Spec::Url(url) => url.fmt(f),
            Spec::Wally(req) => req.fmt(f),
        }
    }
}

impl<'de> Deserialize<'de> for Spec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let url = String::deserialize(deserializer)?;
        Ok(Spec::default_from_source_url(
            url.parse().map_err(de::Error::custom)?,
        ))
    }
}

impl display::AsLuaKV for Spec {
    fn display_lua(&self) -> display::LuaKV {
        match self {
            Spec::Git(git_source) => git_source.display_lua(),
            Spec::File(path) => {
                let mut source_tbl = Vec::new();
                source_tbl.push(display::LuaKV {
                    key: "url".to_string(),
                    value: display::LuaValue::String(format!("file://{}", path.display())),
                });
                display::LuaKV {
                    key: "source".to_string(),
                    value: display::LuaValue::Table(source_tbl),
                }
            }
            Spec::Url(url) => {
                let mut source_tbl = Vec::new();
                source_tbl.push(display::LuaKV {
                    key: "url".to_string(),
                    value: display::LuaValue::String(format!("{url}")),
                });
                display::LuaKV {
                    key: "source".to_string(),
                    value: display::LuaValue::Table(source_tbl),
                }
            }
            Spec::Wally(req) => req.display_lua(),
        }
    }
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
#[error("missing source")]
pub struct MissingSource;

const SUPPORTED_URL_FORMATS_HELP: &str = r#"supported formats are:
- 'git://...'
- 'git+file://...'
- 'git+https://...'
- 'git+http://...'
- 'git+ssh://...'
- 'https://...'
- 'http://...'
- 'ftp://...'
- 'file://...'
"#;

const LUX_ISSUES_URL: &str = "https://github.com/lumen-oss/lux/issues";
const LUAROCKS_BUILD_RULES_URL: &str =
    "https://github.com/luarocks/luarocks/blob/v3.13.0/docs/rockspec_format.md#build-rules";

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
#[error("failed to parse source url")]
pub enum UrlError {
    Io(#[from] io::Error),
    Git(#[from] RemoteGitUrlParseError),
    Url(#[source] <Url as FromStr>::Err),
    #[error("lux does not support rockspecs with CVS sources.")]
    #[diagnostic(
        help("if you need support for CVS sources, open an issue"),
        url("{LUX_ISSUES_URL}")
    )]
    CVS,
    #[error("lux does not support rockspecs with mercurial sources.")]
    #[diagnostic(
        help("if you need support for Mercurial sources, open an issue"),
        url("{LUX_ISSUES_URL}")
    )]
    Mercurial,
    #[error("lux does not support rockspecs with SSCM sources.")]
    #[diagnostic(
        help("if you need support for SSCM sources, open an issue"),
        url("{LUX_ISSUES_URL}")
    )]
    SSCM,
    #[error("lux does not support rockspecs with SVN sources.")]
    #[diagnostic(
        help("if you need support for SVN sources, open an issue"),
        url("{LUX_ISSUES_URL}")
    )]
    SVN,
    #[error("unsupported source URL prefix: '{0}+' in URL '{1}'")]
    #[diagnostic(
        help("{SUPPORTED_URL_FORMATS_HELP}"),
        url("{LUAROCKS_BUILD_RULES_URL}")
    )]
    UnsupportedPrefix(String, String),
    #[error("unsupported source URL: '{0}'")]
    #[diagnostic(
        help("{SUPPORTED_URL_FORMATS_HELP}"),
        url("{LUAROCKS_BUILD_RULES_URL}")
    )]
    Unsupported(String),
}

pub(crate) mod internal {
    use super::*;

    /// Used as a helper for Deserialize,
    /// because the Rockspec schema allows invalid rockspecs (╯°□°)╯︵ ┻━┻
    #[derive(Debug, PartialEq, Deserialize, Clone, Default, lux_macros::AsLuaKV)]
    #[display_lua(key = "source")]
    pub(crate) struct Spec {
        #[serde(default)]
        pub(crate) url: Option<String>,
        pub(crate) file: Option<PathBuf>,
        pub(crate) dir: Option<PathBuf>,
        pub(crate) tag: Option<String>,
        pub(crate) branch: Option<String>,
    }

    impl platform::PartialOverride for Spec {
        type Err = Infallible;

        fn apply_overrides(&self, override_spec: &Self) -> Result<Self, Self::Err> {
            let (tag, branch) = match (&override_spec.tag, &override_spec.branch) {
                (Some(tag), _) => (Some(tag.clone()), None),
                (_, Some(branch)) => (None, Some(branch.clone())),
                (None, None) => (self.tag.clone(), self.branch.clone()),
            };

            Ok(Self {
                url: override_opt(override_spec.url.as_ref(), self.url.as_ref()),
                file: override_opt(override_spec.file.as_ref(), self.file.as_ref()),
                dir: override_opt(override_spec.dir.as_ref(), self.dir.as_ref()),
                tag,
                branch,
            })
        }
    }

    impl platform::Overridable for Spec {
        type Err = MissingSource;

        fn on_nil<T>() -> Result<PerPlatform<T>, <Self as platform::Overridable>::Err>
        where
            T: platform::Overridable,
        {
            Err(MissingSource)
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
        Git(RemoteGitUrl),
    }

    impl FromStr for SourceUrl {
        type Err = UrlError;

        fn from_str(str: &str) -> Result<Self, Self::Err> {
            match str.split_once("+") {
                Some(("git" | "gitrec", url)) => Ok(Self::Git(url.parse()?)),
                Some((prefix, _)) => Err(UrlError::UnsupportedPrefix(
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
                        Ok(Self::Url(s.parse().map_err(UrlError::Url)?))
                    }
                    s if s.starts_with("cvs://") => Err(UrlError::CVS),
                    s if starts_with_any(
                        s,
                        ["hg://", "hg+http://", "hg+https://", "hg+ssh://"].into(),
                    ) =>
                    {
                        Err(UrlError::Mercurial)
                    }
                    s if s.starts_with("sscm://") => Err(UrlError::SSCM),
                    s if s.starts_with("svn://") => Err(UrlError::SVN),
                    s => Err(UrlError::Unsupported(s.to_string())),
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
}

#[cfg(test)]
mod tests {
    use super::display::AsLuaKV;

    use assert_fs::TempDir;

    use super::*;

    fn eval_lua_global<T: serde::de::DeserializeOwned>(code: &str, key: &'static str) -> T {
        use ottavino::{Closure, Executor, Fuel, Lua};
        use ottavino_util::serde::from_value;
        Lua::core()
            .try_enter(|ctx| {
                let closure = Closure::load(ctx, None, code.as_bytes())?;
                let executor = Executor::start(ctx, closure.into(), ());
                executor.step(ctx, &mut Fuel::with(i32::MAX))?;
                from_value(ctx.globals().get_value(ctx, key)).map_err(ottavino::Error::from)
            })
            .unwrap()
    }

    #[test]
    pub fn rock_source_internal_roundtrip() {
        let source = internal::Spec {
            url: Some("https://github.com/example/repo/archive/v1.0.tar.gz".into()),
            file: Some("repo-1.0.tar.gz".into()),
            dir: Some("repo-1.0".into()),
            tag: Some("v1.0".into()),
            branch: None,
        };
        let lua = source.display_lua().to_string();
        let restored: internal::Spec = eval_lua_global(&lua, "source");
        assert_eq!(source, restored);
    }

    #[test]
    pub fn rock_source_internal_branch_roundtrip() {
        let source = internal::Spec {
            url: Some("git+https://github.com/example/repo.git".into()),
            file: None,
            dir: None,
            tag: None,
            branch: Some("main".into()),
        };
        let lua = source.display_lua().to_string();
        let restored: internal::Spec = eval_lua_global(&lua, "source");
        assert_eq!(source, restored);
    }

    #[test]
    pub fn normalises_git_source_dir() {
        let parse = |url: &str, dir: Option<&str>| {
            Remote::try_from(internal::Spec {
                url: Some(url.into()),
                file: None,
                dir: dir.map(Into::into),
                tag: None,
                branch: None,
            })
            .unwrap()
            .local
            .unpack_dir
        };
        let git = "git+https://github.com/lumen-oss/foo";
        assert_eq!(parse(git, None), Some(".".into()));
        assert_eq!(parse(git, Some("foo")), Some(".".into()));
        assert_eq!(parse(git, Some("foo/src")), Some("src".into()));
        assert_eq!(parse(git, Some("src")), Some("src".into()));
        assert_eq!(
            parse("https://example.com/repo-1.0.tar.gz", Some("repo-1.0")),
            Some("repo-1.0".into())
        );
    }

    #[tokio::test]
    async fn parse_source_url() {
        let dir = TempDir::new().unwrap();
        let url: internal::SourceUrl = format!("file://{}", dir.to_string_lossy()).parse().unwrap();
        assert_eq!(url, internal::SourceUrl::File(dir.path().to_path_buf()));
        let url: internal::SourceUrl = "ftp://example.com/foo/bar".parse().unwrap();
        assert!(matches!(url, internal::SourceUrl::Url { .. }));
        let url: internal::SourceUrl = "git://example.com/foo/bar".parse().unwrap();
        assert!(matches!(url, internal::SourceUrl::Git { .. }));
        // We don't support file-like URLs, as they are not remote.
        internal::SourceUrl::from_str("git+file:///path/to/repo.git").unwrap_err();
        let url: internal::SourceUrl = "git+http://example.com/foo/bar".parse().unwrap();
        assert!(matches!(url, internal::SourceUrl::Git { .. }));
        let url: internal::SourceUrl = "git+https://example.com/foo/bar".parse().unwrap();
        assert!(matches!(url, internal::SourceUrl::Git { .. }));
        let url: internal::SourceUrl = "git+ssh://example.com/foo/bar".parse().unwrap();
        assert!(matches!(url, internal::SourceUrl::Git { .. }));
        let url: internal::SourceUrl = "gitrec+https://example.com/foo/bar".parse().unwrap();
        assert!(matches!(url, internal::SourceUrl::Git { .. }));
        let url: internal::SourceUrl = "https://example.com/foo/bar".parse().unwrap();
        assert!(matches!(url, internal::SourceUrl::Url { .. }));
        let url: internal::SourceUrl = "http://example.com/foo/bar".parse().unwrap();
        assert!(matches!(url, internal::SourceUrl::Url { .. }));
    }
}
