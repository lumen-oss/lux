use crate::{
    git::url::RemoteGitUrl,
    lua_rockspec::{DisplayAsLuaValue, DisplayLuaValue},
};

pub mod shorthand;
pub mod url;
pub mod utils;

impl DisplayAsLuaValue for RemoteGitUrl {
    fn display_lua_value(&self) -> DisplayLuaValue {
        DisplayLuaValue::String(self.to_string())
    }
}

/// Specifies a source to be fetched from a git forge
#[derive(Debug, PartialEq, Eq, Hash, Clone, lux_macros::DisplayAsLuaKV)]
#[display_lua(key = "source")]
pub struct GitSource {
    pub url: RemoteGitUrl,
    #[display_lua(rename = "tag")]
    pub checkout_ref: Option<String>,
}
