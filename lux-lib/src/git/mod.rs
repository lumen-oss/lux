use std::fmt::Display;

use crate::{
    git::url::RemoteGitUrl,
    lua_rockspec::{DisplayAsLuaKV, DisplayAsLuaValue, DisplayLuaKV, DisplayLuaValue},
};

pub mod shorthand;
pub mod url;
pub mod utils;

impl DisplayAsLuaValue for RemoteGitUrl {
    fn display_lua_value(&self) -> DisplayLuaValue {
        DisplayLuaValue::String(self.to_string())
    }
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

impl DisplayAsLuaKV for GitSource {
    fn display_lua(&self) -> DisplayLuaKV {
        let mut fields = vec![DisplayLuaKV {
            key: "url".to_string(),
            value: DisplayLuaValue::String(self.url.to_string()),
        }];

        if let Some(git_ref) = &self.git_ref {
            match git_ref {
                GitRef::Tag(tag) => fields.push(DisplayLuaKV {
                    key: "tag".to_string(),
                    value: DisplayLuaValue::String(tag.clone()),
                }),
                GitRef::Branch(branch) => fields.push(DisplayLuaKV {
                    key: "branch".to_string(),
                    value: DisplayLuaValue::String(branch.clone()),
                }),
            }
        }

        DisplayLuaKV {
            key: "source".to_string(),
            value: DisplayLuaValue::Table(fields),
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
