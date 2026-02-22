use itertools::Itertools;
use mlua::{IntoLua, UserData};
use serde::{de, Deserialize, Deserializer};
use std::{collections::HashMap, convert::Infallible, fmt::Display, path::PathBuf, str::FromStr};
use thiserror::Error;

use crate::{
    build::utils::c_dylib_extension,
    lua_rockspec::{
        deserialize_vec_from_lua_array_or_string, DisplayAsLuaValue, PartialOverride, PerPlatform,
        PlatformOverridable,
    },
};

use super::{DisplayLuaKV, DisplayLuaValue};

#[derive(Debug, PartialEq, Deserialize, Default, Clone)]
pub struct BuiltinBuildSpec {
    /// Keys are module names in the format normally used by the `require()` function
    pub modules: HashMap<LuaModule, ModuleSpec>,
}

impl IntoLua for BuiltinBuildSpec {
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        self.modules.into_lua(lua)
    }
}

#[derive(Debug, PartialEq, Eq, Deserialize, Default, Clone, Hash)]
pub struct LuaModule(String);

impl IntoLua for LuaModule {
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        self.0.into_lua(lua)
    }
}

impl LuaModule {
    pub fn to_lua_path(&self) -> PathBuf {
        self.to_file_path(".lua")
    }

    pub fn to_lua_init_path(&self) -> PathBuf {
        self.to_path_buf().join("init.lua")
    }

    pub fn to_lib_path(&self) -> PathBuf {
        self.to_file_path(&format!(".{}", c_dylib_extension()))
    }

    fn to_path_buf(&self) -> PathBuf {
        PathBuf::from(self.0.replace('.', std::path::MAIN_SEPARATOR_STR))
    }

    fn to_file_path(&self, extension: &str) -> PathBuf {
        PathBuf::from(self.0.replace('.', std::path::MAIN_SEPARATOR_STR) + extension)
    }

    pub fn from_pathbuf(path: PathBuf) -> Self {
        let extension = path
            .extension()
            .map(|ext| ext.to_string_lossy().to_string())
            .unwrap_or("".into());
        let module = path
            .to_string_lossy()
            .trim_end_matches(format!("init.{extension}").as_str())
            .trim_end_matches(format!(".{extension}").as_str())
            .trim_end_matches(std::path::MAIN_SEPARATOR_STR)
            .replace(std::path::MAIN_SEPARATOR_STR, ".");
        LuaModule(module)
    }

    pub fn join(&self, other: &LuaModule) -> LuaModule {
        LuaModule(format!("{}.{}", self.0, other.0))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Error, Debug)]
#[error("could not parse lua module from {0}.")]
pub struct ParseLuaModuleError(String);

impl FromStr for LuaModule {
    type Err = ParseLuaModuleError;

    // NOTE: We may want to add some validations
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(LuaModule(s.into()))
    }
}

impl Display for LuaModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum ModuleSpec {
    /// Pathnames of Lua files or C sources, for modules based on a single source file.
    SourcePath(PathBuf),
    /// Pathnames of C sources of a simple module written in C composed of multiple files.
    SourcePaths(Vec<PathBuf>),
    ModulePaths(ModulePaths),
}

impl IntoLua for ModuleSpec {
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        let table = lua.create_table()?;

        match self {
            ModuleSpec::SourcePath(path_buf) => table.set("source", path_buf)?,
            ModuleSpec::SourcePaths(path_bufs) => table.set("sources", path_bufs)?,
            ModuleSpec::ModulePaths(module_paths) => table.set("modules", module_paths)?,
        }

        Ok(mlua::Value::Table(table))
    }
}

impl ModuleSpec {
    pub fn from_internal(
        internal: ModuleSpecInternal,
    ) -> Result<ModuleSpec, ModulePathsMissingSources> {
        match internal {
            ModuleSpecInternal::SourcePath(path) => Ok(ModuleSpec::SourcePath(path)),
            ModuleSpecInternal::SourcePaths(paths) => Ok(ModuleSpec::SourcePaths(paths)),
            ModuleSpecInternal::ModulePaths(module_paths) => Ok(ModuleSpec::ModulePaths(
                ModulePaths::from_internal(module_paths)?,
            )),
        }
    }
}

impl<'de> Deserialize<'de> for ModuleSpec {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_internal(ModuleSpecInternal::deserialize(deserializer)?)
            .map_err(de::Error::custom)
    }
}

impl TryFrom<ModuleSpecInternal> for ModuleSpec {
    type Error = ModulePathsMissingSources;

    fn try_from(internal: ModuleSpecInternal) -> Result<Self, Self::Error> {
        Self::from_internal(internal)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum ModuleSpecInternal {
    SourcePath(PathBuf),
    SourcePaths(Vec<PathBuf>),
    ModulePaths(ModulePathsInternal),
}

impl<'de> Deserialize<'de> for ModuleSpecInternal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.is_string() {
            let src_path = serde_json::from_value(value).map_err(de::Error::custom)?;
            Ok(Self::SourcePath(src_path))
        } else if value.is_array() {
            let src_paths = serde_json::from_value(value).map_err(de::Error::custom)?;
            Ok(Self::SourcePaths(src_paths))
        } else {
            let module_paths = serde_json::from_value(value).map_err(de::Error::custom)?;
            Ok(Self::ModulePaths(module_paths))
        }
    }
}

impl DisplayAsLuaValue for ModuleSpecInternal {
    fn display_lua_value(&self) -> DisplayLuaValue {
        match self {
            ModuleSpecInternal::SourcePath(path) => {
                DisplayLuaValue::String(path.to_string_lossy().into())
            }
            ModuleSpecInternal::SourcePaths(paths) => DisplayLuaValue::List(
                paths
                    .iter()
                    .map(|p| DisplayLuaValue::String(p.to_string_lossy().into()))
                    .collect(),
            ),
            ModuleSpecInternal::ModulePaths(module_paths) => module_paths.display_lua_value(),
        }
    }
}

fn deserialize_definitions<'de, D>(
    deserializer: D,
) -> Result<Vec<(String, Option<String>)>, D::Error>
where
    D: Deserializer<'de>,
{
    let values: Vec<String> = deserialize_vec_from_lua_array_or_string(deserializer)?;
    values
        .iter()
        .map(|val| {
            if let Some((key, value)) = val.split_once('=') {
                Ok((key.into(), Some(value.into())))
            } else {
                Ok((val.into(), None))
            }
        })
        .try_collect()
}

#[derive(Error, Debug)]
#[error("cannot resolve ambiguous platform override for `build.modules`.")]
pub struct ModuleSpecAmbiguousPlatformOverride;

impl PartialOverride for ModuleSpecInternal {
    type Err = ModuleSpecAmbiguousPlatformOverride;

    fn apply_overrides(&self, override_spec: &Self) -> Result<Self, Self::Err> {
        match (override_spec, self) {
            (ModuleSpecInternal::SourcePath(_), b @ ModuleSpecInternal::SourcePath(_)) => {
                Ok(b.to_owned())
            }
            (ModuleSpecInternal::SourcePaths(_), b @ ModuleSpecInternal::SourcePaths(_)) => {
                Ok(b.to_owned())
            }
            (ModuleSpecInternal::ModulePaths(a), ModuleSpecInternal::ModulePaths(b)) => Ok(
                ModuleSpecInternal::ModulePaths(a.apply_overrides(b).unwrap()),
            ),
            _ => Err(ModuleSpecAmbiguousPlatformOverride),
        }
    }
}

#[derive(Error, Debug)]
#[error("could not resolve platform override for `build.modules`. THIS IS A BUG!")]
pub struct BuildModulesPlatformOverride;

impl PlatformOverridable for ModuleSpecInternal {
    type Err = BuildModulesPlatformOverride;

    fn on_nil<T>() -> Result<PerPlatform<T>, <Self as PlatformOverridable>::Err>
    where
        T: PlatformOverridable,
    {
        Err(BuildModulesPlatformOverride)
    }
}

#[derive(Error, Debug)]
#[error("missing or empty field `sources`")]
pub struct ModulePathsMissingSources;

#[derive(Debug, PartialEq, Clone)]
pub struct ModulePaths {
    /// Path names of C sources, mandatory field
    pub sources: Vec<PathBuf>,
    /// External libraries to be linked
    pub libraries: Vec<PathBuf>,
    /// C defines, e.g. { "FOO=bar", "USE_BLA" }
    pub defines: Vec<(String, Option<String>)>,
    /// Directories to be added to the compiler's headers lookup directory list.
    pub incdirs: Vec<PathBuf>,
    /// Directories to be added to the linker's library lookup directory list.
    pub libdirs: Vec<PathBuf>,
}

impl UserData for ModulePaths {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("sources", |_, this, _: ()| Ok(this.sources.clone()));
        methods.add_method("libraries", |_, this, _: ()| Ok(this.libraries.clone()));
        methods.add_method("defines", |_, this, _: ()| {
            Ok(this
                .defines
                .iter()
                .cloned()
                .collect::<HashMap<_, Option<_>>>())
        });
        methods.add_method("incdirs", |_, this, _: ()| Ok(this.incdirs.clone()));
        methods.add_method("libdirs", |_, this, _: ()| Ok(this.libdirs.clone()));
    }
}

impl ModulePaths {
    fn from_internal(
        internal: ModulePathsInternal,
    ) -> Result<ModulePaths, ModulePathsMissingSources> {
        if internal.sources.is_empty() {
            Err(ModulePathsMissingSources)
        } else {
            Ok(ModulePaths {
                sources: internal.sources,
                libraries: internal.libraries,
                defines: internal.defines,
                incdirs: internal.incdirs,
                libdirs: internal.libdirs,
            })
        }
    }
}

impl<'de> Deserialize<'de> for ModulePaths {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let internal = ModulePathsInternal::deserialize(deserializer)?;
        Self::from_internal(internal).map_err(de::Error::custom)
    }
}

#[derive(Debug, PartialEq, Deserialize, Clone, Default)]
pub struct ModulePathsInternal {
    #[serde(default, deserialize_with = "deserialize_vec_from_lua_array_or_string")]
    pub sources: Vec<PathBuf>,
    #[serde(default, deserialize_with = "deserialize_vec_from_lua_array_or_string")]
    pub libraries: Vec<PathBuf>,
    #[serde(default, deserialize_with = "deserialize_definitions")]
    pub defines: Vec<(String, Option<String>)>,
    #[serde(default, deserialize_with = "deserialize_vec_from_lua_array_or_string")]
    pub incdirs: Vec<PathBuf>,
    #[serde(default, deserialize_with = "deserialize_vec_from_lua_array_or_string")]
    pub libdirs: Vec<PathBuf>,
}

impl DisplayAsLuaValue for ModulePathsInternal {
    fn display_lua_value(&self) -> DisplayLuaValue {
        DisplayLuaValue::Table(vec![
            DisplayLuaKV {
                key: "sources".into(),
                value: DisplayLuaValue::List(
                    self.sources
                        .iter()
                        .map(|s| DisplayLuaValue::String(s.to_string_lossy().into()))
                        .collect(),
                ),
            },
            DisplayLuaKV {
                key: "libraries".into(),
                value: DisplayLuaValue::List(
                    self.libraries
                        .iter()
                        .map(|s| DisplayLuaValue::String(s.to_string_lossy().into()))
                        .collect(),
                ),
            },
            DisplayLuaKV {
                key: "defines".into(),
                value: DisplayLuaValue::List(
                    self.defines
                        .iter()
                        .map(|(k, v)| {
                            if let Some(v) = v {
                                DisplayLuaValue::String(format!("{k}={v}"))
                            } else {
                                DisplayLuaValue::String(k.clone())
                            }
                        })
                        .collect(),
                ),
            },
            DisplayLuaKV {
                key: "incdirs".into(),
                value: DisplayLuaValue::List(
                    self.incdirs
                        .iter()
                        .map(|s| DisplayLuaValue::String(s.to_string_lossy().into()))
                        .collect(),
                ),
            },
            DisplayLuaKV {
                key: "libdirs".into(),
                value: DisplayLuaValue::List(
                    self.libdirs
                        .iter()
                        .map(|s| DisplayLuaValue::String(s.to_string_lossy().into()))
                        .collect(),
                ),
            },
        ])
    }
}

impl PartialOverride for ModulePathsInternal {
    type Err = Infallible;

    fn apply_overrides(&self, override_spec: &Self) -> Result<Self, Self::Err> {
        Ok(Self {
            sources: override_vec(override_spec.sources.as_ref(), self.sources.as_ref()),
            libraries: override_vec(override_spec.libraries.as_ref(), self.libraries.as_ref()),
            defines: override_vec(override_spec.defines.as_ref(), self.defines.as_ref()),
            incdirs: override_vec(override_spec.incdirs.as_ref(), self.incdirs.as_ref()),
            libdirs: override_vec(override_spec.libdirs.as_ref(), self.libdirs.as_ref()),
        })
    }
}

impl PlatformOverridable for ModulePathsInternal {
    type Err = Infallible;

    fn on_nil<T>() -> Result<PerPlatform<T>, <Self as PlatformOverridable>::Err>
    where
        T: PlatformOverridable,
        T: Default,
    {
        Ok(PerPlatform::default())
    }
}

fn override_vec<T: Clone>(override_vec: &[T], base: &[T]) -> Vec<T> {
    if override_vec.is_empty() {
        return base.to_owned();
    }
    override_vec.to_owned()
}

#[cfg(test)]
mod tests {
    use mlua::{Lua, LuaSerdeExt};

    use super::*;

    #[tokio::test]
    pub async fn parse_lua_module_from_path() {
        let lua_module = LuaModule::from_pathbuf("foo/init.lua".into());
        assert_eq!(&lua_module.0, "foo");
        let lua_module = LuaModule::from_pathbuf("foo/bar.lua".into());
        assert_eq!(&lua_module.0, "foo.bar");
        let lua_module = LuaModule::from_pathbuf("foo/bar/init.lua".into());
        assert_eq!(&lua_module.0, "foo.bar");
        let lua_module = LuaModule::from_pathbuf("foo/bar/baz.lua".into());
        assert_eq!(&lua_module.0, "foo.bar.baz");
    }

    #[tokio::test]
    pub async fn modules_spec_from_lua() {
        let lua_content = "
        build = {\n
            modules = {\n
                foo = 'lua/foo/init.lua',\n
                bar = {\n
                  'lua/bar.lua',\n
                  'lua/bar/internal.lua',\n
                },\n
                baz = {\n
                    sources = {\n
                        'lua/baz.lua',\n
                    },\n
                    defines = { 'USE_BAZ' },\n
                },\n
                foo = 'lua/foo/init.lua',
            },\n
        }\n
        ";
        let lua = Lua::new();
        lua.load(lua_content).exec().unwrap();
        let build_spec: BuiltinBuildSpec =
            lua.from_value(lua.globals().get("build").unwrap()).unwrap();
        let foo = build_spec
            .modules
            .get(&LuaModule::from_str("foo").unwrap())
            .unwrap();
        assert_eq!(*foo, ModuleSpec::SourcePath("lua/foo/init.lua".into()));
        let bar = build_spec
            .modules
            .get(&LuaModule::from_str("bar").unwrap())
            .unwrap();
        assert_eq!(
            *bar,
            ModuleSpec::SourcePaths(vec!["lua/bar.lua".into(), "lua/bar/internal.lua".into()])
        );
        let baz = build_spec
            .modules
            .get(&LuaModule::from_str("baz").unwrap())
            .unwrap();
        assert!(matches!(baz, ModuleSpec::ModulePaths { .. }));
        let lua_content_no_sources = "
        build = {\n
            modules = {\n
                baz = {\n
                    defines = { 'USE_BAZ' },\n
                },\n
            },\n
        }\n
        ";
        lua.load(lua_content_no_sources).exec().unwrap();
        let result: mlua::Result<BuiltinBuildSpec> =
            lua.from_value(lua.globals().get("build").unwrap());
        let _err = result.unwrap_err();
        let lua_content_complex_defines = "
        build = {\n
            modules = {\n
                baz = {\n
                    sources = {\n
                        'lua/baz.lua',\n
                    },\n
                    defines = { 'USE_BAZ=1', 'ENABLE_LOGGING=true', 'LINK_STATIC' },\n
                },\n
            },\n
        }\n
        ";
        lua.load(lua_content_complex_defines).exec().unwrap();
        let build_spec: BuiltinBuildSpec =
            lua.from_value(lua.globals().get("build").unwrap()).unwrap();
        let baz = build_spec
            .modules
            .get(&LuaModule::from_str("baz").unwrap())
            .unwrap();
        match baz {
            ModuleSpec::ModulePaths(paths) => assert_eq!(
                paths.defines,
                vec![
                    ("USE_BAZ".into(), Some("1".into())),
                    ("ENABLE_LOGGING".into(), Some("true".into())),
                    ("LINK_STATIC".into(), None)
                ]
            ),
            _ => panic!(),
        }
    }
}
