use mlua::UserData;

use crate::{
    git::url::GitUrl,
    lua_rockspec::{DisplayAsLuaKV, DisplayLuaKV, DisplayLuaValue},
};

pub mod shorthand;
pub mod url;
pub mod utils;

#[derive(Debug, PartialEq, Clone)]
pub struct GitSource {
    pub url: GitUrl,
    pub checkout_ref: Option<String>,
}

impl UserData for GitSource {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("url", |_, this, _: ()| Ok(this.url.to_string()));
        methods.add_method("checkout_ref", |_, this, _: ()| {
            Ok(this.checkout_ref.clone())
        });
    }
}

impl DisplayAsLuaKV for GitSource {
    fn display_lua(&self) -> DisplayLuaKV {
        let mut source_tbl = Vec::new();
        source_tbl.push(DisplayLuaKV {
            key: "url".to_string(),
            value: DisplayLuaValue::String(format!("{}", self.url)),
        });
        if let Some(checkout_ref) = &self.checkout_ref {
            source_tbl.push(DisplayLuaKV {
                // branches are not reproducible, so we will only ever generate tags.
                // lux can also fetch revisions.
                key: "tag".to_string(),
                value: DisplayLuaValue::String(checkout_ref.to_string()),
            });
        }
        DisplayLuaKV {
            key: "source".to_string(),
            value: DisplayLuaValue::Table(source_tbl),
        }
    }
}
