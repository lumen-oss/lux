pub mod backend;
pub(crate) mod internal;

use crate::package;
use path_slash::PathBufExt;

use itertools::Itertools;

use miette::Diagnostic;
use serde::{de, de::IntoDeserializer, Deserialize, Deserializer};
use std::{
    collections::HashMap, convert::Infallible, env::consts::DLL_EXTENSION, env::consts::DLL_PREFIX,
    fmt::Display, path::PathBuf, str::FromStr,
};
use thiserror::Error;

use crate::rockspec::platform::per_platform_from_intermediate;

use super::display::{AsLuaKV, AsLuaValue};
use super::{display, lua_dependency, serde_util, PerPlatform};

/// The build specification for a given rock, serialized from `build = { ... }`.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    /// Determines the build backend to use.
    pub backend: Option<backend::Spec>,
    /// A set of instructions on how/where to copy files from the project.
    // TODO(vhyrro): While we may want to support this, we also may want to supercede this in our
    // new Lua project rewrite.
    pub install: Install,
    /// A list of directories that should be copied as-is into the resulting rock.
    pub copy_directories: Vec<PathBuf>,
    /// A list of patches to apply to the project before packaging it.
    // NOTE: This cannot be a diffy::Patch<'a, str>
    // because Lua::from_value requires a DeserializeOwned
    pub patches: HashMap<PathBuf, String>,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            backend: Some(backend::Spec::default()),
            install: Install::default(),
            copy_directories: Vec::default(),
            patches: HashMap::default(),
        }
    }
}

impl Spec {
    pub(crate) fn from_internal_spec(internal: internal::Spec) -> Result<Self, internal::Error> {
        let backend = match internal.build_type.unwrap_or_default() {
            internal::Type::Builtin => {
                Some(backend::Spec::Builtin(
                    backend::Builtin {
                        modules: internal
                            .builtin_spec
                            .unwrap_or_default()
                            .into_iter()
                            .map(|(key, module_spec_internal)| {
                                let key_str = match key {
                                    serde_util::LuaTableKey::IntKey(_) => {
                                        Err(internal::Error::ModulesHaveListElements)
                                    }
                                    serde_util::LuaTableKey::StringKey(str) => {
                                        Ok(backend::Module::from_str(str.as_str())?)
                                    }
                                }?;
                                match backend::ModuleSpec::from_internal(module_spec_internal) {
                                    Ok(module_spec) => Ok((key_str, module_spec)),
                                    Err(err) => Err(err.into()),
                                }
                            })
                            .collect::<Result<
                                HashMap<backend::Module, backend::ModuleSpec>,
                                internal::Error,
                            >>()?,
                    },
                ))
            }
            internal::Type::Make => {
                let default = backend::Make::default();
                Some(backend::Spec::Make(backend::Make {
                    makefile: internal.makefile.unwrap_or(default.makefile),
                    build_target: internal.make_build_target,
                    build_pass: internal.build_pass.unwrap_or(default.build_pass),
                    install_target: internal
                        .make_install_target
                        .unwrap_or(default.install_target),
                    install_pass: internal.install_pass.unwrap_or(default.install_pass),
                    build_variables: internal.make_build_variables.unwrap_or_default(),
                    install_variables: internal.make_install_variables.unwrap_or_default(),
                    variables: internal.variables.unwrap_or_default(),
                }))
            }
            internal::Type::CMake => {
                let default = backend::CMake::default();
                Some(backend::Spec::CMake(backend::CMake {
                    cmake_lists_content: internal.cmake_lists_content,
                    build_pass: internal.build_pass.unwrap_or(default.build_pass),
                    install_pass: internal.install_pass.unwrap_or(default.install_pass),
                    variables: internal.variables.unwrap_or_default(),
                }))
            }
            internal::Type::Command => Some(backend::Spec::Command(backend::Command {
                build_command: internal.build_command,
                install_command: internal.install_command,
            })),
            internal::Type::None => None,
            internal::Type::LuaRock(s) => Some(backend::Spec::LuaRock(s)),
            internal::Type::RustMlua => Some(backend::Spec::RustMlua(backend::RustMlua {
                modules: internal
                    .builtin_spec
                    .ok_or(internal::Error::NoModulesSpecified)?
                    .into_iter()
                    .map(|(key, value)| match (key, value) {
                        (
                            serde_util::LuaTableKey::IntKey(_),
                            backend::builtin::internal::ModuleSpec::SourcePath(module),
                        ) => {
                            let mut rust_lib: PathBuf =
                                format!("{DLL_PREFIX}{}", module.display()).into();
                            rust_lib.set_extension(DLL_EXTENSION);
                            Ok((module.to_string_lossy().to_string(), rust_lib))
                        }
                        (
                            serde_util::LuaTableKey::StringKey(module_name),
                            backend::builtin::internal::ModuleSpec::SourcePath(module),
                        ) => {
                            let mut rust_lib: PathBuf =
                                format!("{DLL_PREFIX}{}", module.display()).into();
                            rust_lib.set_extension(DLL_EXTENSION);
                            Ok((module_name, rust_lib))
                        }
                        _ => Err(internal::Error::InvalidRustMLuaFormat),
                    })
                    .try_collect()?,
                target_path: internal.target_path.unwrap_or("target".into()),
                default_features: internal.default_features.unwrap_or(true),
                features: internal.features.unwrap_or_default(),
                cargo_extra_args: internal.cargo_extra_args.unwrap_or_default(),
                include: internal
                    .include
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(key, dest)| match key {
                        serde_util::LuaTableKey::IntKey(_) => (dest.clone(), dest),
                        serde_util::LuaTableKey::StringKey(src) => (src.into(), dest),
                    })
                    .collect(),
            })),
            internal::Type::RustBinary => Some(backend::Spec::RustBinary(backend::RustBinary {
                package: internal.package,
                features: internal.features.unwrap_or_default(),
            })),
            internal::Type::TreesitterParser => {
                Some(backend::Spec::TreesitterParser(backend::Treesitter {
                    lang: internal
                        .lang
                        .ok_or(internal::Error::NoTreesitterParserLanguageSpecified)?,
                    parser: internal.parser.unwrap_or(true),
                    generate: internal.generate.unwrap_or(false),
                    location: internal.location,
                    queries: internal.queries.unwrap_or_default(),
                }))
            }
            internal::Type::Source => Some(backend::Spec::Source),
        };
        Ok(Self {
            backend,
            install: internal.install.unwrap_or_default(),
            copy_directories: internal.copy_directories.unwrap_or_default(),
            patches: internal.patches.unwrap_or_default(),
        })
    }
}

impl TryFrom<internal::Spec> for Spec {
    type Error = internal::Error;

    fn try_from(internal: internal::Spec) -> Result<Self, Self::Error> {
        Spec::from_internal_spec(internal)
    }
}

impl<'de> Deserialize<'de> for Spec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let internal = internal::Spec::deserialize(deserializer)?;
        Spec::from_internal_spec(internal).map_err(de::Error::custom)
    }
}

// TODO(vhyrro): Remove this when we migrate to deepmerge.
// This is a hacky implementation that would work normally with just the above deserialization
// strategy however since there is no PlatformOevrridable implemented for this struct this is
// necessary.
impl<'de> Deserialize<'de> for PerPlatform<Spec> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        per_platform_from_intermediate::<_, internal::Spec, _>(deserializer)
    }
}

#[derive(Clone, Debug)]
struct LuaPathBufTable(HashMap<serde_util::LuaTableKey, PathBuf>);

impl<'de> Deserialize<'de> for LuaPathBufTable {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(LuaPathBufTable(
            deserialize_map_or_seq(deserializer)?.unwrap_or_default(),
        ))
    }
}

impl LuaPathBufTable {
    fn coerce<S>(self) -> Result<HashMap<S, PathBuf>, S::Err>
    where
        S: FromStr + Eq + std::hash::Hash,
    {
        self.0
            .into_iter()
            .map(|(key, value)| {
                let key = match key {
                    serde_util::LuaTableKey::IntKey(_) => value
                        .with_extension("")
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                    serde_util::LuaTableKey::StringKey(key) => key,
                };
                Ok((S::from_str(&key)?, value))
            })
            .try_collect()
    }
}

#[derive(Clone, Debug)]
struct LibPathBufTable(HashMap<serde_util::LuaTableKey, PathBuf>);

impl<'de> Deserialize<'de> for LibPathBufTable {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(LibPathBufTable(
            deserialize_map_or_seq(deserializer)?.unwrap_or_default(),
        ))
    }
}

impl LibPathBufTable {
    fn coerce<S>(self) -> Result<HashMap<S, PathBuf>, S::Err>
    where
        S: FromStr + Eq + std::hash::Hash,
    {
        self.0
            .into_iter()
            .map(|(key, value)| {
                let key = match key {
                    serde_util::LuaTableKey::IntKey(_) => value
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                    serde_util::LuaTableKey::StringKey(key) => key,
                };
                Ok((S::from_str(&key)?, value))
            })
            .try_collect()
    }
}

/// For packages which don't provide means to install modules
/// and expect the user to copy the .lua or library files by hand to the proper locations.
/// This struct contains categories of files. Each category is itself a table,
/// where the array part is a list of filenames to be copied.
/// For module directories only, in the hash part, other keys are identifiers in Lua module format,
/// to indicate which subdirectory the file should be copied to.
/// For example, lua = {["foo.bar"] = "src/bar.lua"} will copy src/bar.lua
/// to the foo directory under the rock's Lua files directory.
#[derive(Debug, PartialEq, Default, Deserialize, Clone, lux_macros::AsLuaKV)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[display_lua(key = "install")]
pub struct Install {
    /// Lua modules written in Lua.
    #[serde(default, deserialize_with = "deserialize_module_path_map")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "std::collections::BTreeMap<String, String>")
    )]
    pub lua: HashMap<backend::Module, PathBuf>,
    /// Dynamic libraries implemented compiled Lua modules.
    #[serde(default, deserialize_with = "deserialize_file_name_path_map")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "std::collections::BTreeMap<String, String>")
    )]
    pub lib: HashMap<String, PathBuf>,
    /// Configuration files.
    #[serde(default)]
    pub conf: HashMap<String, PathBuf>,
    /// Lua command-line scripts.
    // TODO(vhyrro): The String component should be checked to ensure that it consists of a single
    // path component, such that targets like `my.binary` are not allowed.
    #[serde(default, deserialize_with = "deserialize_file_name_path_map")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "std::collections::BTreeMap<String, String>")
    )]
    pub bin: HashMap<String, PathBuf>,
}

fn deserialize_module_path_map<'de, D>(
    deserializer: D,
) -> Result<HashMap<backend::Module, PathBuf>, D::Error>
where
    D: Deserializer<'de>,
{
    let modules = LuaPathBufTable::deserialize(deserializer)?;
    modules.coerce().map_err(de::Error::custom)
}

fn deserialize_file_name_path_map<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, PathBuf>, D::Error>
where
    D: Deserializer<'de>,
{
    let binaries = LibPathBufTable::deserialize(deserializer)?;
    binaries.coerce().map_err(de::Error::custom)
}

fn deserialize_copy_directories<'de, D>(deserializer: D) -> Result<Option<Vec<PathBuf>>, D::Error>
where
    D: Deserializer<'de>,
{
    let value: Option<serde_value::Value> = Option::deserialize(deserializer)?;
    let copy_directories: Option<Vec<String>> = match value {
        Some(value) => Some(value.deserialize_into().map_err(de::Error::custom)?),
        None => None,
    };
    let special_directories: Vec<String> = vec!["lua".into(), "lib".into(), "rock_manifest".into()];
    match special_directories
        .into_iter()
        .find(|dir| copy_directories.clone().unwrap_or_default().contains(dir))
    {
        // NOTE(mrcjkb): There also shouldn't be a directory named the same as the rockspec,
        // but I'm not sure how to (or if it makes sense to) enforce this here.
        Some(d) => Err(format!(
            "directory '{d}' in copy_directories clashes with the .rock format", // TODO(vhyrro): More informative error message.
        )),
        _ => Ok(copy_directories.map(|vec| vec.into_iter().map(PathBuf::from).collect())),
    }
    .map_err(de::Error::custom)
}

/// Deserializes a map that may be represented as a sequence (integer-indexed Lua array).
fn deserialize_map_or_seq<'de, D, V>(
    deserializer: D,
) -> Result<Option<HashMap<serde_util::LuaTableKey, V>>, D::Error>
where
    D: Deserializer<'de>,
    V: de::DeserializeOwned,
{
    match de::DeserializeSeed::deserialize(serde_util::LuaValueSeed, deserializer)
        .map_err(de::Error::custom)?
    {
        serde_value::Value::Map(map) => map
            .into_iter()
            .map(|(k, v)| {
                let key = match k {
                    serde_value::Value::I64(i) => serde_util::LuaTableKey::IntKey(i as u64),
                    serde_value::Value::U64(u) => serde_util::LuaTableKey::IntKey(u),
                    serde_value::Value::String(s) => serde_util::LuaTableKey::StringKey(s),
                    other => {
                        return Err(de::Error::custom(format!("unexpected map key: {other:?}")))
                    }
                };
                let val = v.deserialize_into::<V>().map_err(de::Error::custom)?;
                Ok((key, val))
            })
            .try_collect()
            .map(Some),
        serde_value::Value::Seq(seq) => seq
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                let val = v.deserialize_into::<V>().map_err(de::Error::custom)?;
                Ok((serde_util::LuaTableKey::IntKey(i as u64 + 1), val))
            })
            .try_collect()
            .map(Some),
        serde_value::Value::Unit => Ok(None),
        other => Err(de::Error::custom(format!(
            "expected a table or nil, got {other:?}"
        ))),
    }
}

fn display_builtin_spec(
    spec: &HashMap<serde_util::LuaTableKey, backend::builtin::internal::ModuleSpec>,
) -> display::LuaValue {
    display::LuaValue::Table(
        spec.iter()
            .map(|(key, value)| display::LuaKV {
                key: match key {
                    serde_util::LuaTableKey::StringKey(s) => s.clone(),
                    serde_util::LuaTableKey::IntKey(_) => unreachable!("integer key in modules"),
                },
                value: value.display_lua_value(),
            })
            .collect(),
    )
}

fn display_path_string_map(map: &HashMap<PathBuf, String>) -> display::LuaValue {
    display::LuaValue::Table(
        map.iter()
            .map(|(k, v)| display::LuaKV {
                key: k.to_slash_lossy().into_owned(),
                value: display::LuaValue::String(v.clone()),
            })
            .collect(),
    )
}

fn display_include(include: &HashMap<serde_util::LuaTableKey, PathBuf>) -> display::LuaValue {
    display::LuaValue::Table(
        include
            .iter()
            .map(|(key, value)| display::LuaKV {
                key: match key {
                    serde_util::LuaTableKey::StringKey(s) => s.clone(),
                    serde_util::LuaTableKey::IntKey(_) => unreachable!("integer key in include"),
                },
                value: display::LuaValue::String(value.to_slash_lossy().into_owned()),
            })
            .collect(),
    )
}

impl display::AsLuaValue for Install {
    fn display_lua_value(&self) -> display::LuaValue {
        self.display_lua().value
    }
}

#[cfg(test)]
mod tests {

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

    #[tokio::test]
    pub async fn deserialize_build_type() {
        let build_type: internal::Type = serde_json::from_str("\"builtin\"").unwrap();
        assert_eq!(build_type, internal::Type::Builtin);
        let build_type: internal::Type = serde_json::from_str("\"module\"").unwrap();
        assert_eq!(build_type, internal::Type::Builtin);
        let build_type: internal::Type = serde_json::from_str("\"make\"").unwrap();
        assert_eq!(build_type, internal::Type::Make);
        let build_type: internal::Type = serde_json::from_str("\"custom_build_backend\"").unwrap();
        assert_eq!(
            build_type,
            internal::Type::LuaRock("custom_build_backend".into())
        );
        let build_type: internal::Type = serde_json::from_str("\"rust-mlua\"").unwrap();
        assert_eq!(build_type, internal::Type::RustMlua);
    }

    #[test]
    pub fn install_spec_roundtrip() {
        let spec = Install {
            lua: HashMap::from([(
                "mymod".parse::<backend::Module>().unwrap(),
                "src/mymod.lua".into(),
            )]),
            lib: HashMap::from([("mylib".into(), "lib/mylib.so".into())]),
            conf: HashMap::from([("myconf".into(), "conf/myconf.cfg".into())]),
            bin: HashMap::from([("mybinary".into(), "bin/mybinary".into())]),
        };
        let lua = spec.display_lua().to_string();
        let restored: Install = eval_lua_global(&lua, "install");
        assert_eq!(spec, restored);
    }

    #[test]
    pub fn install_spec_empty_roundtrip() {
        let spec = Install::default();
        let lua = spec.display_lua().to_string();
        let lua = if lua.trim().is_empty() {
            "install = {}".to_string()
        } else {
            lua
        };
        let restored: Install = eval_lua_global(&lua, "install");
        assert_eq!(spec, restored);
    }

    #[test]
    pub fn build_spec_internal_builtin_roundtrip() {
        let spec = internal::Spec {
            build_type: Some(internal::Type::Builtin),
            builtin_spec: Some(HashMap::from([(
                serde_util::LuaTableKey::StringKey("mymod".into()),
                backend::builtin::internal::ModuleSpec::SourcePath("src/mymod.lua".into()),
            )])),
            install: Some(Install {
                lua: HashMap::from([(
                    "extra".parse::<backend::Module>().unwrap(),
                    "src/extra.lua".into(),
                )]),
                bin: HashMap::from([("mytool".into(), "bin/mytool".into())]),
                ..Default::default()
            }),
            copy_directories: Some(vec!["docs".into()]),
            ..Default::default()
        };
        let lua = spec.display_lua().to_string();
        let restored: internal::Spec = eval_lua_global(&lua, "build");
        assert_eq!(spec, restored);
    }

    #[test]
    pub fn build_spec_internal_make_roundtrip() {
        let spec = internal::Spec {
            build_type: Some(internal::Type::Make),
            makefile: Some("GNUmakefile".into()),
            make_build_target: Some("all".into()),
            make_install_target: Some("install".into()),
            make_build_variables: Some(HashMap::from([("CFLAGS".into(), "-O2".into())])),
            make_install_variables: Some(HashMap::from([("PREFIX".into(), "/usr/local".into())])),
            variables: Some(HashMap::from([("LUA_LIBDIR".into(), "/usr/lib".into())])),
            ..Default::default()
        };
        let lua = spec.display_lua().to_string();
        let restored: internal::Spec = eval_lua_global(&lua, "build");
        assert_eq!(spec, restored);
    }
}
