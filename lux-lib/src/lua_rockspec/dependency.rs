use std::{collections::HashMap, convert::Infallible, path::PathBuf};

use mlua::FromLua;
use path_slash::PathBufExt;
use serde::Deserialize;

use super::{
    DisplayAsLuaKV, DisplayLuaKV, DisplayLuaValue, PartialOverride, PerPlatform,
    PlatformOverridable,
};

/// Can be defined in a [platform-agnostic](https://github.com/luarocks/luarocks/wiki/platform-agnostic-external-dependencies) manner
#[derive(Debug, PartialEq, Clone, Deserialize, Default)]
pub struct ExternalDependencySpec {
    /// A header file, e.g. "foo.h"
    pub(crate) header: Option<PathBuf>,
    /// A library file, e.g. "libfoo.so"
    pub(crate) library: Option<PathBuf>,
}

/*
impl IntoLua for ExternalDependencySpec {
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        let table = lua.create_table()?;
        if let Some(path) = self.header {
            table.set("header", path.to_slash_lossy().to_string())?;
        }
        if let Some(path) = self.library {
            table.set("library", path.to_slash_lossy().to_string())?;
        }
        Ok(mlua::Value::Table(table))
    }
}
*/

impl FromLua for ExternalDependencySpec {
    fn from_lua(value: mlua::Value, _lua: &mlua::Lua) -> mlua::Result<Self> {
        if let mlua::Value::Table(table) = value {
            let header = table.get("header")?;
            let library = table.get("library")?;

            Ok(Self { header, library })
        } else {
            Err(mlua::Error::FromLuaConversionError {
                from: "ExternalDependencySpec",
                to: "table".to_string(),
                message: Some("Expected a table".to_string()),
            })
        }
    }
}

impl PartialOverride for ExternalDependencySpec {
    type Err = Infallible;

    fn apply_overrides(&self, override_val: &Self) -> Result<Self, Self::Err> {
        Ok(Self {
            header: override_val.header.clone().or(self.header.clone()),
            library: override_val.library.clone().or(self.header.clone()),
        })
    }
}

impl PartialOverride for HashMap<String, ExternalDependencySpec> {
    type Err = Infallible;

    fn apply_overrides(&self, override_map: &Self) -> Result<Self, Self::Err> {
        let mut result = Self::new();
        for (key, value) in self {
            result.insert(
                key.clone(),
                override_map
                    .get(key)
                    .map(|override_val| value.apply_overrides(override_val).expect("infallible"))
                    .unwrap_or(value.clone()),
            );
        }
        for (key, value) in override_map {
            if !result.contains_key(key) {
                result.insert(key.clone(), value.clone());
            }
        }
        Ok(result)
    }
}

impl PlatformOverridable for HashMap<String, ExternalDependencySpec> {
    type Err = Infallible;

    fn on_nil<T>() -> Result<super::PerPlatform<T>, <Self as PlatformOverridable>::Err>
    where
        T: PlatformOverridable,
        T: Default,
    {
        Ok(PerPlatform::default())
    }
}

pub(crate) struct ExternalDependencies<'a>(pub(crate) &'a HashMap<String, ExternalDependencySpec>);

impl DisplayAsLuaKV for ExternalDependencies<'_> {
    fn display_lua(&self) -> DisplayLuaKV {
        DisplayLuaKV {
            key: "external_dependencies".to_string(),
            value: DisplayLuaValue::Table(
                self.0
                    .iter()
                    .map(|(key, value)| {
                        let mut value_entries = Vec::new();
                        if let Some(path) = &value.header {
                            value_entries.push(DisplayLuaKV {
                                key: "header".to_string(),
                                value: DisplayLuaValue::String(path.to_slash_lossy().to_string()),
                            });
                        }
                        if let Some(path) = &value.library {
                            value_entries.push(DisplayLuaKV {
                                key: "library".to_string(),
                                value: DisplayLuaValue::String(path.to_slash_lossy().to_string()),
                            });
                        }
                        DisplayLuaKV {
                            key: key.clone(),
                            value: DisplayLuaValue::Table(value_entries),
                        }
                    })
                    .collect(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_external_dependency_spec_from_lua() {
        let lua = mlua::Lua::new();
        let lua_code = r#"
            return {
                foo = { header = "foo.h", library = "libfoo.so" },
                bar = { header = "bar.h" },
                baz = { library = "libbaz.so" },
            }
        "#;
        let value = lua.load(lua_code).eval().unwrap();
        let deps: HashMap<String, ExternalDependencySpec> = FromLua::from_lua(value, &lua).unwrap();
        assert_eq!(deps.len(), 3);
        assert_eq!(deps["foo"].header.as_ref().unwrap().to_slash_lossy(), "foo.h");
        assert_eq!(deps["foo"].library.as_ref().unwrap().to_slash_lossy(), "libfoo.so");

        assert_eq!(deps["bar"].header.as_ref().unwrap().to_slash_lossy(), "bar.h");
        assert!(deps["bar"].library.is_none());

        assert!(deps["baz"].header.is_none());
        assert_eq!(deps["baz"].library.as_ref().unwrap().to_slash_lossy(), "libbaz.so");
    }
}
