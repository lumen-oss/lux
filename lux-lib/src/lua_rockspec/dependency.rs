use std::{collections::HashMap, convert::Infallible, path::PathBuf};

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
    pub header: Option<PathBuf>,
    /// A library file, e.g. "libfoo.so"
    pub library: Option<PathBuf>,
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
    use piccolo::{Closure, Executor, Fuel, Lua, Value};
    use piccolo_util::serde::from_value;

    use super::*;

    fn eval_lua<T: serde::de::DeserializeOwned>(code: &str) -> Result<T, piccolo::StaticError> {
        Lua::core().try_enter(|ctx| {
            let closure = Closure::load(ctx, None, code.as_bytes())?;
            let executor = Executor::start(ctx, closure.into(), ());
            executor.step(ctx, &mut Fuel::with(i32::MAX));
            from_value(executor.take_result::<Value<'_>>(ctx)??).map_err(piccolo::Error::from)
        })
    }

    #[test]
    fn test_external_dependency_spec_from_lua() {
        let lua_code = r#"
            return {
                foo = { header = "foo.h", library = "libfoo.so" },
                bar = { header = "bar.h" },
                baz = { library = "libbaz.so" },
            }
        "#;
        let deps: HashMap<String, ExternalDependencySpec> = eval_lua(lua_code).unwrap();
        assert_eq!(deps.len(), 3);
        assert_eq!(
            deps["foo"].header.as_ref().unwrap().to_slash_lossy(),
            "foo.h"
        );
        assert_eq!(
            deps["foo"].library.as_ref().unwrap().to_slash_lossy(),
            "libfoo.so"
        );

        assert_eq!(
            deps["bar"].header.as_ref().unwrap().to_slash_lossy(),
            "bar.h"
        );
        assert!(deps["bar"].library.is_none());

        assert!(deps["baz"].header.is_none());
        assert_eq!(
            deps["baz"].library.as_ref().unwrap().to_slash_lossy(),
            "libbaz.so"
        );
    }
}
