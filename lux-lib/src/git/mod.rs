use std::fmt::Display;

use miette::Diagnostic;
use thiserror::Error;

use crate::{fs, git::url::RemoteGitUrl, rockspec};

pub mod shorthand;
pub mod url;
pub mod utils;

impl rockspec::display::AsLuaValue for RemoteGitUrl {
    fn display_lua_value(&self) -> rockspec::display::LuaValue {
        rockspec::display::LuaValue::String(self.to_string())
    }
}

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error("error initializing temporary bare git repository to fetch metadata")]
    BareRepoInit(#[source] git2::Error),
    #[error("error initializing remote repository '{0}' to fetch metadata")]
    RemoteInit(String, #[source] git2::Error),
    #[error("error fetching from remote repository '{0}'")]
    RemoteFetch(String, #[source] git2::Error),
    #[error("error listing remote refs for '{0}'")]
    RemoteList(String, #[source] git2::Error),
    #[error("could not determine latest tag or commit sha for '{0}'")]
    NoTagOrCommitSha(String),
}
/// Specifies a git reference (tag or branch)
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum GitRef {
    Tag(String),
    Branch(String),
}

/// Specifies a source to be fetched from a git forge
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct GitSource {
    pub url: RemoteGitUrl,
    pub git_ref: Option<GitRef>,
}

impl rockspec::display::AsLuaKV for GitSource {
    fn display_lua(&self) -> rockspec::display::LuaKV {
        let mut fields = vec![rockspec::display::LuaKV {
            key: "url".to_string(),
            value: rockspec::display::LuaValue::String(self.url.to_string()),
        }];

        if let Some(git_ref) = &self.git_ref {
            match git_ref {
                GitRef::Tag(tag) => fields.push(rockspec::display::LuaKV {
                    key: "tag".to_string(),
                    value: rockspec::display::LuaValue::String(tag.clone()),
                }),
                GitRef::Branch(branch) => fields.push(rockspec::display::LuaKV {
                    key: "branch".to_string(),
                    value: rockspec::display::LuaValue::String(branch.clone()),
                }),
            }
        }

        rockspec::display::LuaKV {
            key: "source".to_string(),
            value: rockspec::display::LuaValue::Table(fields),
        }
    }
}

impl Display for GitSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.git_ref {
            Some(GitRef::Tag(tag)) => format!("{}@{}", self.url, tag).fmt(f),
            Some(GitRef::Branch(branch)) => format!("{}@{}", self.url, branch).fmt(f),
            None => self.url.fmt(f),
        }
    }
}
