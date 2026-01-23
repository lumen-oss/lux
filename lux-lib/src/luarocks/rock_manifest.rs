use itertools::Itertools;
use mlua::{FromLua, Lua, LuaSerdeExt, Table, Value};
use path_slash::PathBufExt;
use serde::Deserialize;
/// Compatibility layer/adapter for the luarocks client
use std::{collections::HashMap, path::PathBuf};
use thiserror::Error;

use crate::lua_rockspec::{DisplayAsLuaKV, DisplayAsLuaValue, DisplayLuaKV, DisplayLuaValue};

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RockManifest {
    pub lib: RockManifestLib,
    pub lua: RockManifestLua,
    pub bin: RockManifestBin,
    pub doc: RockManifestDoc,
    pub conf: RockManifestConf,
    pub root: RockManifestRoot,
}

#[derive(Error, Debug)]
pub enum RockManifestError {
    #[error("could not parse rock_manifest: {0}")]
    MLua(#[from] mlua::Error),
}

impl RockManifest {
    pub fn new(rock_manifest_content: &str) -> Result<Self, RockManifestError> {
        let lua = Lua::new();

        #[cfg(feature = "luau")]
        lua.sandbox(true)?;

        lua.load(rock_manifest_content).exec()?;
        let globals = lua.globals();
        let value = globals.get("rock_manifest")?;
        Ok(Self::from_lua(value, &lua)?)
    }

    pub fn to_lua_string(&self) -> String {
        self.display_lua().to_string()
    }
}

impl FromLua for RockManifest {
    fn from_lua(value: Value, lua: &Lua) -> mlua::Result<Self> {
        match &value {
            Value::Table(rock_manifest) => {
                let lib = RockManifestLib {
                    entries: rock_manifest_dir_or_file_entry_from_lua(rock_manifest, lua, "lib")?,
                };
                let lua_entry = RockManifestLua {
                    entries: rock_manifest_dir_or_file_entry_from_lua(rock_manifest, lua, "lua")?,
                };
                let bin = RockManifestBin {
                    entries: rock_manifest_bin_entry_from_lua(rock_manifest, lua, "bin")?,
                };
                let doc = RockManifestDoc {
                    entries: rock_manifest_dir_or_file_entry_from_lua(rock_manifest, lua, "doc")?,
                };
                let conf = RockManifestConf {
                    entries: rock_manifest_dir_or_file_entry_from_lua(rock_manifest, lua, "conf")?,
                };
                let mut root_entry = HashMap::new();
                rock_manifest.for_each(|key: String, value: Value| {
                    if matches!(key.as_str(), "lib" | "lua" | "bin" | "doc" | "conf") {
                        return Ok(());
                    }
                    if let val @ Value::String(_) = value {
                        root_entry.insert(
                            key.into(),
                            DirOrFileEntry::FileEntry(String::from_lua(val, lua)?),
                        );
                    } else if let Value::Table(_) = value {
                        let entry =
                            rock_manifest_dir_or_file_entry_from_lua(rock_manifest, lua, &key)?;
                        root_entry.insert(key.into(), DirOrFileEntry::DirEntry(entry));
                    }
                    Ok(())
                })?;
                let root = RockManifestRoot {
                    entries: root_entry,
                };
                Ok(Self {
                    lib,
                    lua: lua_entry,
                    bin,
                    doc,
                    conf,
                    root,
                })
            }
            Value::Nil => Ok(Self::default()),
            val => Err(mlua::Error::DeserializeError(format!(
                "Expected rock_manifest to be a table or nil, but got {}",
                val.type_name()
            ))),
        }
    }
}

impl DisplayAsLuaKV for RockManifest {
    fn display_lua(&self) -> DisplayLuaKV {
        DisplayLuaKV {
            key: "rock_manifest".to_string(),
            value: DisplayLuaValue::Table(
                vec![
                    self.lua.display_lua(),
                    self.lib.display_lua(),
                    self.doc.display_lua(),
                    self.conf.display_lua(),
                    self.bin.display_lua(),
                ]
                .into_iter()
                .chain(self.root.entries.iter().map(|(key, entry)| DisplayLuaKV {
                    key: key.to_slash_lossy().to_string(),
                    value: entry.display_lua_value(),
                }))
                .collect_vec(),
            ),
        }
    }
}

impl DisplayAsLuaKV for (&PathBuf, &String) {
    fn display_lua(&self) -> DisplayLuaKV {
        DisplayLuaKV {
            key: format!("{}", self.0.display()),
            value: DisplayLuaValue::String(self.1.clone()),
        }
    }
}

impl DisplayAsLuaValue for HashMap<PathBuf, String> {
    fn display_lua_value(&self) -> DisplayLuaValue {
        DisplayLuaValue::Table(self.iter().map(|it| it.display_lua()).collect_vec())
    }
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub(crate) enum DirOrFileEntry {
    DirEntry(HashMap<PathBuf, DirOrFileEntry>),
    FileEntry(String),
}

impl DisplayAsLuaValue for DirOrFileEntry {
    fn display_lua_value(&self) -> DisplayLuaValue {
        match self {
            Self::DirEntry(dir_map) => DisplayLuaValue::Table(to_lua_kv_vec(dir_map)),
            Self::FileEntry(md5sum) => DisplayLuaValue::String(md5sum.clone()),
        }
    }
}

impl DisplayAsLuaValue for HashMap<PathBuf, DirOrFileEntry> {
    fn display_lua_value(&self) -> DisplayLuaValue {
        DisplayLuaValue::Table(to_lua_kv_vec(self))
    }
}

fn to_lua_kv_vec(dir_map: &HashMap<PathBuf, DirOrFileEntry>) -> Vec<DisplayLuaKV> {
    dir_map
        .iter()
        .map(|(k, v)| DisplayLuaKV {
            key: k.to_slash_lossy().to_string(),
            value: v.display_lua_value(),
        })
        .collect_vec()
}

impl From<&str> for DirOrFileEntry {
    fn from(value: &str) -> Self {
        Self::FileEntry(value.into())
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RockManifestLua {
    pub entries: HashMap<PathBuf, DirOrFileEntry>,
}

impl DisplayAsLuaKV for RockManifestLua {
    fn display_lua(&self) -> DisplayLuaKV {
        DisplayLuaKV {
            key: "lua".to_string(),
            value: self.entries.display_lua_value(),
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RockManifestLib {
    pub entries: HashMap<PathBuf, DirOrFileEntry>,
}

impl DisplayAsLuaKV for RockManifestLib {
    fn display_lua(&self) -> crate::lua_rockspec::DisplayLuaKV {
        DisplayLuaKV {
            key: "lib".to_string(),
            value: self.entries.display_lua_value(),
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RockManifestBin {
    pub entries: HashMap<PathBuf, String>,
}

impl DisplayAsLuaKV for RockManifestBin {
    fn display_lua(&self) -> crate::lua_rockspec::DisplayLuaKV {
        DisplayLuaKV {
            key: "bin".to_string(),
            value: self.entries.display_lua_value(),
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RockManifestDoc {
    pub entries: HashMap<PathBuf, DirOrFileEntry>,
}

impl DisplayAsLuaKV for RockManifestDoc {
    fn display_lua(&self) -> crate::lua_rockspec::DisplayLuaKV {
        DisplayLuaKV {
            key: "doc".to_string(),
            value: self.entries.display_lua_value(),
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RockManifestConf {
    pub entries: HashMap<PathBuf, DirOrFileEntry>,
}

impl DisplayAsLuaKV for RockManifestConf {
    fn display_lua(&self) -> crate::lua_rockspec::DisplayLuaKV {
        DisplayLuaKV {
            key: "conf".to_string(),
            value: self.entries.display_lua_value(),
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RockManifestRoot {
    pub entries: HashMap<PathBuf, DirOrFileEntry>,
}

fn rock_manifest_dir_or_file_entry_from_lua(
    tbl: &Table,
    lua: &Lua,
    key: &str,
) -> mlua::Result<HashMap<PathBuf, DirOrFileEntry>> {
    if tbl.contains_key(key)? {
        lua.from_value(tbl.get(key)?)
    } else {
        Ok(HashMap::default())
    }
}

fn rock_manifest_bin_entry_from_lua(
    rock_manifest: &Table,
    lua: &Lua,
    key: &str,
) -> mlua::Result<HashMap<PathBuf, String>> {
    if rock_manifest.contains_key(key)? {
        lua.from_value(rock_manifest.get(key)?)
    } else {
        Ok(HashMap::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    pub async fn rock_manifest_from_lua() {
        let rock_manifest_content = "
rock_manifest = {
   doc = {
      ['CHANGELOG.md'] = 'adbf3f997070946a5e61955d70bfadb2',
      LICENSE = '6bcb3636a93bdb8304439a4ff57e979c',
      ['README.md'] = '842bd0b364e36d982f02e22abee7742d'
   },
   conf = {
      ['config.toml'] = '8cbb3637a94bdb8304440a5ff58e980d',
   },
   lib = {
      ['toml_edit.so'] = '504d63aea7bb341a688ef28f1232fa9b',
   },
   plugin = {
      ['foo.lua'] = '506d61aea8bb340a688ef29f1235fa8c',
   },
   ['toml-edit-0.6.1-1.rockspec'] = 'fcdd3b0066632dec36cd5510e00bc55e'
}
        ";
        let rock_manifest = RockManifest::new(rock_manifest_content).unwrap();
        assert_eq!(
            rock_manifest,
            RockManifest {
                lib: RockManifestLib {
                    entries: HashMap::from_iter(vec![(
                        "toml_edit.so".into(),
                        "504d63aea7bb341a688ef28f1232fa9b".into()
                    )])
                },
                lua: RockManifestLua::default(),
                bin: RockManifestBin::default(),
                doc: RockManifestDoc {
                    entries: HashMap::from_iter(vec![
                        (
                            "CHANGELOG.md".into(),
                            "adbf3f997070946a5e61955d70bfadb2".into()
                        ),
                        ("LICENSE".into(), "6bcb3636a93bdb8304439a4ff57e979c".into()),
                        (
                            "README.md".into(),
                            "842bd0b364e36d982f02e22abee7742d".into()
                        ),
                    ])
                },
                conf: RockManifestConf {
                    entries: HashMap::from_iter(vec![(
                        "config.toml".into(),
                        "8cbb3637a94bdb8304440a5ff58e980d".into()
                    ),])
                },
                root: RockManifestRoot {
                    entries: HashMap::from_iter(vec![
                        (
                            "toml-edit-0.6.1-1.rockspec".into(),
                            "fcdd3b0066632dec36cd5510e00bc55e".into()
                        ),
                        (
                            "plugin".into(),
                            DirOrFileEntry::DirEntry(HashMap::from_iter(vec![(
                                "foo.lua".into(),
                                "506d61aea8bb340a688ef29f1235fa8c".into()
                            )])),
                        ),
                    ])
                },
            }
        );
    }

    #[tokio::test]
    pub async fn regression_http_rock_manifest_from_lua() {
        let content = String::from_utf8(
            tokio::fs::read("resources/test/http-0.4-0-rock_manifest")
                .await
                .unwrap(),
        )
        .unwrap();
        RockManifest::new(&content).unwrap();
    }
}
