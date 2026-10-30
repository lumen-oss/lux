use super::super::super::platform;
use crate::{
    rockspec::{display, serde_util, PerPlatform},
    target::c_dylib_extension,
};
use itertools::Itertools;
use miette::Diagnostic;
use path_slash::PathBufExt;
use serde::{de, Deserialize, Deserializer};
use std::{collections::HashMap, convert::Infallible, fmt::Display, path::PathBuf, str::FromStr};
use thiserror::Error;

#[derive(Debug, PartialEq, Deserialize, Default, Clone)]
pub struct Builtin {
    /// Keys are module names in the format normally used by the `require()` function
    pub modules: HashMap<Module, ModuleSpec>,
}

#[derive(Debug, PartialEq, Eq, Deserialize, Default, Clone, Hash)]
pub struct Module(String);

impl Module {
    pub fn to_lua_path(&self) -> PathBuf {
        self.to_file_path(".lua")
    }

    pub fn to_lua_init_path(&self) -> PathBuf {
        self.to_path_buf().join("init.lua")
    }

    pub fn to_luau_path(&self) -> PathBuf {
        self.to_file_path(".luau")
    }

    pub fn to_luau_init_path(&self) -> PathBuf {
        self.to_path_buf().join("init.luau")
    }

    pub fn to_lib_path(&self) -> PathBuf {
        self.to_file_path(&format!(".{}", c_dylib_extension()))
    }

    fn to_path_buf(&self) -> PathBuf {
        PathBuf::from(self.0.replace('.', std::path::MAIN_SEPARATOR_STR))
    }

    pub(crate) fn to_file_path(&self, extension: &str) -> PathBuf {
        PathBuf::from(self.0.replace('.', std::path::MAIN_SEPARATOR_STR) + extension)
    }

    pub fn from_pathbuf(path: PathBuf) -> Result<Self, ParseLuaModuleError> {
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
        if module.is_empty() {
            Err(ParseLuaModuleError::EmptyModule(
                path.to_slash_lossy().to_string(),
            ))
        } else {
            Ok(Module(module))
        }
    }

    pub fn join(&self, other: &Module) -> Module {
        Module(format!("{}.{}", self.0, other.0))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum ParseLuaModuleError {
    #[error("path '{0}' resulted in an empty Lua module.")]
    EmptyModule(String),
    #[error("cannot parse a Lua module from an empty string")]
    EmptyString,
}

impl FromStr for Module {
    type Err = ParseLuaModuleError;

    // NOTE: We may want to add some additional validations
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            Err(ParseLuaModuleError::EmptyString)
        } else {
            Ok(Module(s.into()))
        }
    }
}

impl Display for Module {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl display::AsLuaValue for Module {
    fn display_lua_value(&self) -> display::LuaValue {
        display::LuaValue::String(self.to_string())
    }
}

impl display::AsLuaValue for HashMap<Module, PathBuf> {
    fn display_lua_value(&self) -> display::LuaValue {
        use path_slash::PathBufExt as _;
        display::LuaValue::Table(
            self.iter()
                .map(|(k, v)| display::LuaKV {
                    key: k.to_string(),
                    value: display::LuaValue::String(v.to_slash_lossy().into_owned()),
                })
                .collect_vec(),
        )
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

impl ModuleSpec {
    pub fn from_internal(
        internal: internal::ModuleSpec,
    ) -> Result<ModuleSpec, internal::ModulePathsMissingSources> {
        match internal {
            internal::ModuleSpec::SourcePath(path) => Ok(ModuleSpec::SourcePath(path)),
            internal::ModuleSpec::SourcePaths(paths) => Ok(ModuleSpec::SourcePaths(paths)),
            internal::ModuleSpec::ModulePaths(module_paths) => Ok(ModuleSpec::ModulePaths(
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
        Self::from_internal(internal::ModuleSpec::deserialize(deserializer)?)
            .map_err(de::Error::custom)
    }
}

impl TryFrom<internal::ModuleSpec> for ModuleSpec {
    type Error = internal::ModulePathsMissingSources;

    fn try_from(internal: internal::ModuleSpec) -> Result<Self, Self::Error> {
        Self::from_internal(internal)
    }
}

impl From<ModuleSpec> for internal::ModuleSpec {
    fn from(spec: ModuleSpec) -> Self {
        match spec {
            ModuleSpec::SourcePath(path) => Self::SourcePath(path),
            ModuleSpec::SourcePaths(paths) => Self::SourcePaths(paths),
            ModuleSpec::ModulePaths(paths) => Self::ModulePaths(paths.into()),
        }
    }
}

impl display::AsLuaValue for ModuleSpec {
    fn display_lua_value(&self) -> display::LuaValue {
        internal::ModuleSpec::from(self.clone()).display_lua_value()
    }
}

fn deserialize_definitions<'de, D>(
    deserializer: D,
) -> Result<Vec<(String, Option<String>)>, D::Error>
where
    D: Deserializer<'de>,
{
    let values: Vec<String> = serde_util::deserialize_vec_from_lua_array_or_string(deserializer)?;
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

impl ModulePaths {
    fn from_internal(
        internal: internal::ModulePaths,
    ) -> Result<ModulePaths, internal::ModulePathsMissingSources> {
        if internal.sources.is_empty() {
            Err(internal::ModulePathsMissingSources)
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

impl TryFrom<internal::ModulePaths> for ModulePaths {
    type Error = internal::ModulePathsMissingSources;

    fn try_from(internal: internal::ModulePaths) -> Result<Self, Self::Error> {
        Self::from_internal(internal)
    }
}

impl From<ModulePaths> for internal::ModulePaths {
    fn from(paths: ModulePaths) -> Self {
        Self {
            sources: paths.sources,
            libraries: paths.libraries,
            defines: paths.defines,
            incdirs: paths.incdirs,
            libdirs: paths.libdirs,
        }
    }
}

impl<'de> Deserialize<'de> for ModulePaths {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_internal(internal::ModulePaths::deserialize(deserializer)?)
            .map_err(de::Error::custom)
    }
}

pub(crate) mod internal {
    use super::*;

    #[derive(Debug, PartialEq, Clone)]
    pub enum ModuleSpec {
        SourcePath(PathBuf),
        SourcePaths(Vec<PathBuf>),
        ModulePaths(ModulePaths),
    }

    impl<'de> Deserialize<'de> for ModuleSpec {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value =
                serde_util::normalize_lua_value(serde_value::Value::deserialize(deserializer)?);
            match value {
                serde_value::Value::String(s) => Ok(Self::SourcePath(PathBuf::from(s))),
                serde_value::Value::Seq(_) => {
                    let src_paths: Vec<PathBuf> =
                        value.deserialize_into().map_err(de::Error::custom)?;
                    Ok(Self::SourcePaths(src_paths))
                }
                serde_value::Value::Map(_) => {
                    let module_paths: ModulePaths =
                        value.deserialize_into().map_err(de::Error::custom)?;
                    Ok(Self::ModulePaths(module_paths))
                }
                _ => Err(de::Error::custom(format!(
                    "expected a string, list, or table for module spec, got: {value:?}"
                ))),
            }
        }
    }

    impl display::AsLuaValue for ModuleSpec {
        fn display_lua_value(&self) -> display::LuaValue {
            match self {
                ModuleSpec::SourcePath(path) => {
                    display::LuaValue::String(path.to_string_lossy().into())
                }
                ModuleSpec::SourcePaths(paths) => display::LuaValue::List(
                    paths
                        .iter()
                        .map(|p| display::LuaValue::String(p.to_string_lossy().into()))
                        .collect(),
                ),
                ModuleSpec::ModulePaths(module_paths) => module_paths.display_lua_value(),
            }
        }
    }

    #[derive(Error, Debug, Diagnostic)]
    #[non_exhaustive]
    #[error("cannot resolve ambiguous platform override for `build.modules`.")]
    pub struct ModuleSpecAmbiguousPlatformOverride;

    impl platform::PartialOverride for ModuleSpec {
        type Err = ModuleSpecAmbiguousPlatformOverride;

        fn apply_overrides(&self, override_spec: &Self) -> Result<Self, Self::Err> {
            match (override_spec, self) {
                (ModuleSpec::SourcePath(_), b @ ModuleSpec::SourcePath(_)) => Ok(b.to_owned()),
                (ModuleSpec::SourcePaths(_), b @ ModuleSpec::SourcePaths(_)) => Ok(b.to_owned()),
                (ModuleSpec::ModulePaths(a), ModuleSpec::ModulePaths(b)) => {
                    Ok(ModuleSpec::ModulePaths(a.apply_overrides(b).unwrap()))
                }
                _ => Err(ModuleSpecAmbiguousPlatformOverride),
            }
        }
    }

    #[derive(Error, Debug, Diagnostic)]
    #[non_exhaustive]
    #[error("could not resolve platform override for `build.modules`. THIS IS A BUG!")]
    pub struct BuildModulesPlatformOverride;

    impl platform::Overridable for ModuleSpec {
        type Err = BuildModulesPlatformOverride;

        fn on_nil<T>() -> Result<PerPlatform<T>, <Self as platform::Overridable>::Err>
        where
            T: platform::Overridable,
        {
            Err(BuildModulesPlatformOverride)
        }
    }

    #[derive(Error, Debug, Diagnostic)]
    #[non_exhaustive]
    #[error("missing or empty field `sources`")]
    pub struct ModulePathsMissingSources;

    /// Specification for building a Lua module from various sources
    #[derive(Debug, PartialEq, Deserialize, Clone, Default)]
    pub struct ModulePaths {
        #[serde(
            default,
            deserialize_with = "serde_util::deserialize_vec_from_lua_array_or_string"
        )]
        pub sources: Vec<PathBuf>,
        #[serde(
            default,
            deserialize_with = "serde_util::deserialize_vec_from_lua_array_or_string"
        )]
        pub libraries: Vec<PathBuf>,
        #[serde(default, deserialize_with = "deserialize_definitions")]
        pub defines: Vec<(String, Option<String>)>,
        #[serde(
            default,
            deserialize_with = "serde_util::deserialize_vec_from_lua_array_or_string"
        )]
        pub incdirs: Vec<PathBuf>,
        #[serde(
            default,
            deserialize_with = "serde_util::deserialize_vec_from_lua_array_or_string"
        )]
        pub libdirs: Vec<PathBuf>,
    }

    impl display::AsLuaValue for ModulePaths {
        fn display_lua_value(&self) -> display::LuaValue {
            display::LuaValue::Table(vec![
                display::LuaKV {
                    key: "sources".into(),
                    value: display::LuaValue::List(
                        self.sources
                            .iter()
                            .map(|s| display::LuaValue::String(s.to_string_lossy().into()))
                            .collect(),
                    ),
                },
                display::LuaKV {
                    key: "libraries".into(),
                    value: display::LuaValue::List(
                        self.libraries
                            .iter()
                            .map(|s| display::LuaValue::String(s.to_string_lossy().into()))
                            .collect(),
                    ),
                },
                display::LuaKV {
                    key: "defines".into(),
                    value: display::LuaValue::List(
                        self.defines
                            .iter()
                            .map(|(k, v)| {
                                if let Some(v) = v {
                                    display::LuaValue::String(format!("{k}={v}"))
                                } else {
                                    display::LuaValue::String(k.clone())
                                }
                            })
                            .collect(),
                    ),
                },
                display::LuaKV {
                    key: "incdirs".into(),
                    value: display::LuaValue::List(
                        self.incdirs
                            .iter()
                            .map(|s| display::LuaValue::String(s.to_string_lossy().into()))
                            .collect(),
                    ),
                },
                display::LuaKV {
                    key: "libdirs".into(),
                    value: display::LuaValue::List(
                        self.libdirs
                            .iter()
                            .map(|s| display::LuaValue::String(s.to_string_lossy().into()))
                            .collect(),
                    ),
                },
            ])
        }
    }

    impl platform::PartialOverride for ModulePaths {
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

    impl platform::Overridable for ModulePaths {
        type Err = Infallible;

        fn on_nil<T>() -> Result<PerPlatform<T>, <Self as platform::Overridable>::Err>
        where
            T: platform::Overridable,
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
}

#[cfg(test)]
mod tests {
    use ottavino::{Closure, Executor, Fuel, Lua};
    use ottavino_util::serde::from_value;

    use super::*;

    fn exec_lua<T: serde::de::DeserializeOwned>(
        code: &str,
        key: &'static str,
    ) -> Result<T, ottavino::ExternError> {
        Lua::core().try_enter(|ctx| {
            let closure = Closure::load(ctx, None, code.as_bytes())?;
            let executor = Executor::start(ctx, closure.into(), ());
            executor.step(ctx, &mut Fuel::with(i32::MAX))?;
            from_value(ctx.globals().get_value(ctx, key)).map_err(ottavino::Error::from)
        })
    }

    #[tokio::test]
    pub async fn parse_lua_module_from_path() {
        let lua_module = Module::from_pathbuf("foo/init.lua".into()).unwrap();
        assert_eq!(&lua_module.0, "foo");
        let lua_module = Module::from_pathbuf("foo/bar.lua".into()).unwrap();
        assert_eq!(&lua_module.0, "foo.bar");
        let lua_module = Module::from_pathbuf("foo/bar/init.lua".into()).unwrap();
        assert_eq!(&lua_module.0, "foo.bar");
        let lua_module = Module::from_pathbuf("foo/bar/baz.lua".into()).unwrap();
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
        let build_spec: Builtin = exec_lua(lua_content, "build").unwrap();
        let foo = build_spec
            .modules
            .get(&Module::from_str("foo").unwrap())
            .unwrap();
        assert_eq!(*foo, ModuleSpec::SourcePath("lua/foo/init.lua".into()));
        let bar = build_spec
            .modules
            .get(&Module::from_str("bar").unwrap())
            .unwrap();
        assert_eq!(
            *bar,
            ModuleSpec::SourcePaths(vec!["lua/bar.lua".into(), "lua/bar/internal.lua".into()])
        );
        let baz = build_spec
            .modules
            .get(&Module::from_str("baz").unwrap())
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
        let result: Result<Builtin, _> = exec_lua(lua_content_no_sources, "build");
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
        let build_spec: Builtin = exec_lua(lua_content_complex_defines, "build").unwrap();
        let baz = build_spec
            .modules
            .get(&Module::from_str("baz").unwrap())
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
