mod builtin;
mod cmake;
mod make;
mod rust_mlua;
mod tree_sitter;

pub use builtin::{BuiltinBuildSpec, LuaModule, ModulePaths, ModuleSpec};
pub use cmake::*;
pub use make::*;
use path_slash::PathBufExt;
pub use rust_mlua::*;
pub use tree_sitter::*;

use builtin::{
    ModulePathsMissingSources, ModuleSpecAmbiguousPlatformOverride, ModuleSpecInternal,
    ParseLuaModuleError,
};

use itertools::Itertools;

use std::{
    collections::HashMap, convert::Infallible, env::consts::DLL_EXTENSION, fmt::Display,
    path::PathBuf, str::FromStr,
};
use thiserror::Error;

use serde::{de, de::IntoDeserializer, Deserialize, Deserializer};

use crate::{
    lua_rockspec::per_platform_from_intermediate,
    package::{PackageName, PackageReq},
    rockspec::lua_dependency::LuaDependencySpec,
};

use super::{
    DisplayAsLuaKV, DisplayAsLuaValue, DisplayLuaKV, DisplayLuaValue, LuaTableKey, LuaValueSeed,
    PartialOverride, PerPlatform, PlatformOverridable,
};

/// The build specification for a given rock, serialized from `rockspec.build = { ... }`.
///
/// See [the rockspec format](https://github.com/luarocks/luarocks/wiki/Rockspec-format) for more
/// info.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildSpec {
    /// Determines the build backend to use.
    pub build_backend: Option<BuildBackendSpec>,
    /// A set of instructions on how/where to copy files from the project.
    // TODO(vhyrro): While we may want to support this, we also may want to supercede this in our
    // new Lua project rewrite.
    pub install: InstallSpec,
    /// A list of directories that should be copied as-is into the resulting rock.
    pub copy_directories: Vec<PathBuf>,
    /// A list of patches to apply to the project before packaging it.
    // NOTE: This cannot be a diffy::Patch<'a, str>
    // because Lua::from_value requires a DeserializeOwned
    pub patches: HashMap<PathBuf, String>,
}

impl Default for BuildSpec {
    fn default() -> Self {
        Self {
            build_backend: Some(BuildBackendSpec::default()),
            install: InstallSpec::default(),
            copy_directories: Vec::default(),
            patches: HashMap::default(),
        }
    }
}

#[derive(Error, Debug)]
pub enum BuildSpecInternalError {
    #[error("'builtin' modules should not have list elements")]
    ModulesHaveListElements,
    #[error("no 'modules' specified for the 'rust-mlua' build backend")]
    NoModulesSpecified,
    #[error("no 'lang' specified for 'treesitter-parser' build backend")]
    NoTreesitterParserLanguageSpecified,
    #[error("invalid 'rust-mlua' modules format")]
    InvalidRustMLuaFormat,
    #[error(transparent)]
    ModulePathsMissingSources(#[from] ModulePathsMissingSources),
    #[error(transparent)]
    ParseLuaModuleError(#[from] ParseLuaModuleError),
}

impl BuildSpec {
    pub(crate) fn from_internal_spec(
        internal: BuildSpecInternal,
    ) -> Result<Self, BuildSpecInternalError> {
        let build_backend = match internal.build_type.unwrap_or_default() {
            BuildType::Builtin => Some(BuildBackendSpec::Builtin(BuiltinBuildSpec {
                modules: internal
                    .builtin_spec
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(key, module_spec_internal)| {
                        let key_str = match key {
                            LuaTableKey::IntKey(_) => {
                                Err(BuildSpecInternalError::ModulesHaveListElements)
                            }
                            LuaTableKey::StringKey(str) => Ok(LuaModule::from_str(str.as_str())?),
                        }?;
                        match ModuleSpec::from_internal(module_spec_internal) {
                            Ok(module_spec) => Ok((key_str, module_spec)),
                            Err(err) => Err(err.into()),
                        }
                    })
                    .collect::<Result<HashMap<LuaModule, ModuleSpec>, BuildSpecInternalError>>()?,
            })),
            BuildType::Make => {
                let default = MakeBuildSpec::default();
                Some(BuildBackendSpec::Make(MakeBuildSpec {
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
            BuildType::CMake => {
                let default = CMakeBuildSpec::default();
                Some(BuildBackendSpec::CMake(CMakeBuildSpec {
                    cmake_lists_content: internal.cmake_lists_content,
                    build_pass: internal.build_pass.unwrap_or(default.build_pass),
                    install_pass: internal.install_pass.unwrap_or(default.install_pass),
                    variables: internal.variables.unwrap_or_default(),
                }))
            }
            BuildType::Command => Some(BuildBackendSpec::Command(CommandBuildSpec {
                build_command: internal.build_command,
                install_command: internal.install_command,
            })),
            BuildType::None => None,
            BuildType::LuaRock(s) => Some(BuildBackendSpec::LuaRock(s)),
            BuildType::RustMlua => Some(BuildBackendSpec::RustMlua(RustMluaBuildSpec {
                modules: internal
                    .builtin_spec
                    .ok_or(BuildSpecInternalError::NoModulesSpecified)?
                    .into_iter()
                    .map(|(key, value)| match (key, value) {
                        (LuaTableKey::IntKey(_), ModuleSpecInternal::SourcePath(module)) => {
                            let mut rust_lib: PathBuf = format!("lib{}", module.display()).into();
                            rust_lib.set_extension(DLL_EXTENSION);
                            Ok((module.to_string_lossy().to_string(), rust_lib))
                        }
                        (
                            LuaTableKey::StringKey(module_name),
                            ModuleSpecInternal::SourcePath(module),
                        ) => {
                            let mut rust_lib: PathBuf = format!("lib{}", module.display()).into();
                            rust_lib.set_extension(DLL_EXTENSION);
                            Ok((module_name, rust_lib))
                        }
                        _ => Err(BuildSpecInternalError::InvalidRustMLuaFormat),
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
                        LuaTableKey::IntKey(_) => (dest.clone(), dest),
                        LuaTableKey::StringKey(src) => (src.into(), dest),
                    })
                    .collect(),
            })),
            BuildType::TreesitterParser => Some(BuildBackendSpec::TreesitterParser(
                TreesitterParserBuildSpec {
                    lang: internal
                        .lang
                        .ok_or(BuildSpecInternalError::NoTreesitterParserLanguageSpecified)?,
                    parser: internal.parser.unwrap_or(false),
                    generate: internal.generate.unwrap_or(false),
                    location: internal.location,
                    queries: internal.queries.unwrap_or_default(),
                },
            )),
            BuildType::Source => Some(BuildBackendSpec::Source),
        };
        Ok(Self {
            build_backend,
            install: internal.install.unwrap_or_default(),
            copy_directories: internal.copy_directories.unwrap_or_default(),
            patches: internal.patches.unwrap_or_default(),
        })
    }
}

impl TryFrom<BuildSpecInternal> for BuildSpec {
    type Error = BuildSpecInternalError;

    fn try_from(internal: BuildSpecInternal) -> Result<Self, Self::Error> {
        BuildSpec::from_internal_spec(internal)
    }
}

impl<'de> Deserialize<'de> for BuildSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let internal = BuildSpecInternal::deserialize(deserializer)?;
        BuildSpec::from_internal_spec(internal).map_err(de::Error::custom)
    }
}

// TODO(vhyrro): Remove this when we migrate to deepmerge.
// This is a hacky implementation that would work normally with just the above deserialization
// strategy however since there is no PlatformOevrridable implemented for this struct this is
// necessary.
impl<'de> Deserialize<'de> for PerPlatform<BuildSpec> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        per_platform_from_intermediate::<_, BuildSpecInternal, _>(deserializer)
    }
}

impl Default for BuildBackendSpec {
    fn default() -> Self {
        Self::Builtin(BuiltinBuildSpec::default())
    }
}

/// Encodes extra information about each backend.
/// When selecting a backend, one may provide extra parameters
/// to `build = { ... }` in order to further customize the behaviour of the build step.
///
/// Luarocks provides several default build types, these are also reflected in `lux`
/// for compatibility.
#[derive(Debug, PartialEq, Clone)]
pub enum BuildBackendSpec {
    Builtin(BuiltinBuildSpec),
    Make(MakeBuildSpec),
    CMake(CMakeBuildSpec),
    Command(CommandBuildSpec),
    LuaRock(String),
    RustMlua(RustMluaBuildSpec),
    TreesitterParser(TreesitterParserBuildSpec),
    /// Build from the source rockspec, if present.
    /// Otherwise, fall back to the builtin build and copy all directories.
    /// This is currently unimplemented by luarocks, but we don't ever publish rockspecs
    /// that implement this.
    /// It could be implemented as a custom build backend.
    Source,
}

impl BuildBackendSpec {
    pub(crate) fn can_use_build_dependencies(&self) -> bool {
        match self {
            Self::Make(_) | Self::CMake(_) | Self::Command(_) | Self::LuaRock(_) => true,
            Self::Builtin(_) | Self::RustMlua(_) | Self::TreesitterParser(_) | Self::Source => {
                false
            }
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct CommandBuildSpec {
    pub build_command: Option<String>,
    pub install_command: Option<String>,
}

#[derive(Clone, Debug)]
struct LuaPathBufTable(HashMap<LuaTableKey, PathBuf>);

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
                    LuaTableKey::IntKey(_) => value
                        .with_extension("")
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                    LuaTableKey::StringKey(key) => key,
                };
                Ok((S::from_str(&key)?, value))
            })
            .try_collect()
    }
}

#[derive(Clone, Debug)]
struct LibPathBufTable(HashMap<LuaTableKey, PathBuf>);

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
                    LuaTableKey::IntKey(_) => value
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                    LuaTableKey::StringKey(key) => key,
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
/// For example, build.install.lua = {["foo.bar"] = {"src/bar.lua"}} will copy src/bar.lua
/// to the foo directory under the rock's Lua files directory.
#[derive(Debug, PartialEq, Default, Deserialize, Clone, lux_macros::DisplayAsLuaKV)]
#[display_lua(key = "install")]
pub struct InstallSpec {
    /// Lua modules written in Lua.
    #[serde(default, deserialize_with = "deserialize_module_path_map")]
    pub lua: HashMap<LuaModule, PathBuf>,
    /// Dynamic libraries implemented compiled Lua modules.
    #[serde(default, deserialize_with = "deserialize_file_name_path_map")]
    pub lib: HashMap<String, PathBuf>,
    /// Configuration files.
    #[serde(default)]
    pub conf: HashMap<String, PathBuf>,
    /// Lua command-line scripts.
    // TODO(vhyrro): The String component should be checked to ensure that it consists of a single
    // path component, such that targets like `my.binary` are not allowed.
    #[serde(default, deserialize_with = "deserialize_file_name_path_map")]
    pub bin: HashMap<String, PathBuf>,
}

fn deserialize_module_path_map<'de, D>(
    deserializer: D,
) -> Result<HashMap<LuaModule, PathBuf>, D::Error>
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
) -> Result<Option<HashMap<LuaTableKey, V>>, D::Error>
where
    D: Deserializer<'de>,
    V: de::DeserializeOwned,
{
    match de::DeserializeSeed::deserialize(LuaValueSeed, deserializer).map_err(de::Error::custom)? {
        serde_value::Value::Map(map) => map
            .into_iter()
            .map(|(k, v)| {
                let key = match k {
                    serde_value::Value::I64(i) => LuaTableKey::IntKey(i as u64),
                    serde_value::Value::U64(u) => LuaTableKey::IntKey(u),
                    serde_value::Value::String(s) => LuaTableKey::StringKey(s),
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
                Ok((LuaTableKey::IntKey(i as u64 + 1), val))
            })
            .try_collect()
            .map(Some),
        serde_value::Value::Unit => Ok(None),
        other => Err(de::Error::custom(format!(
            "expected a table or nil, got {other:?}"
        ))),
    }
}

fn display_builtin_spec(spec: &HashMap<LuaTableKey, ModuleSpecInternal>) -> DisplayLuaValue {
    DisplayLuaValue::Table(
        spec.iter()
            .map(|(key, value)| DisplayLuaKV {
                key: match key {
                    LuaTableKey::StringKey(s) => s.clone(),
                    LuaTableKey::IntKey(_) => unreachable!("integer key in modules"),
                },
                value: value.display_lua_value(),
            })
            .collect(),
    )
}

fn display_path_string_map(map: &HashMap<PathBuf, String>) -> DisplayLuaValue {
    DisplayLuaValue::Table(
        map.iter()
            .map(|(k, v)| DisplayLuaKV {
                key: k.to_slash_lossy().into_owned(),
                value: DisplayLuaValue::String(v.clone()),
            })
            .collect(),
    )
}

fn display_include(include: &HashMap<LuaTableKey, PathBuf>) -> DisplayLuaValue {
    DisplayLuaValue::Table(
        include
            .iter()
            .map(|(key, value)| DisplayLuaKV {
                key: match key {
                    LuaTableKey::StringKey(s) => s.clone(),
                    LuaTableKey::IntKey(_) => unreachable!("integer key in include"),
                },
                value: DisplayLuaValue::String(value.to_slash_lossy().into_owned()),
            })
            .collect(),
    )
}

#[derive(Debug, PartialEq, Deserialize, Default, Clone, lux_macros::DisplayAsLuaKV)]
#[display_lua(key = "build")]
pub(crate) struct BuildSpecInternal {
    #[serde(rename = "type", default)]
    #[display_lua(rename = "type")]
    pub(crate) build_type: Option<BuildType>,
    #[serde(
        rename = "modules",
        default,
        deserialize_with = "deserialize_map_or_seq"
    )]
    #[display_lua(rename = "modules", convert_with = "display_builtin_spec")]
    pub(crate) builtin_spec: Option<HashMap<LuaTableKey, ModuleSpecInternal>>,
    #[serde(default)]
    pub(crate) makefile: Option<PathBuf>,
    #[serde(rename = "build_target", default)]
    #[display_lua(rename = "build_target")]
    pub(crate) make_build_target: Option<String>,
    #[serde(default)]
    pub(crate) build_pass: Option<bool>,
    #[serde(rename = "install_target", default)]
    #[display_lua(rename = "install_target")]
    pub(crate) make_install_target: Option<String>,
    #[serde(default)]
    pub(crate) install_pass: Option<bool>,
    #[serde(rename = "build_variables", default)]
    #[display_lua(rename = "build_variables")]
    pub(crate) make_build_variables: Option<HashMap<String, String>>,
    #[serde(rename = "install_variables", default)]
    #[display_lua(rename = "install_variables")]
    pub(crate) make_install_variables: Option<HashMap<String, String>>,
    #[serde(default)]
    pub(crate) variables: Option<HashMap<String, String>>,
    #[serde(rename = "cmake", default)]
    #[display_lua(rename = "cmake")]
    pub(crate) cmake_lists_content: Option<String>,
    #[serde(default)]
    pub(crate) build_command: Option<String>,
    #[serde(default)]
    pub(crate) install_command: Option<String>,
    #[serde(default)]
    pub(crate) install: Option<InstallSpec>,
    #[serde(default, deserialize_with = "deserialize_copy_directories")]
    pub(crate) copy_directories: Option<Vec<PathBuf>>,
    #[serde(default)]
    #[display_lua(convert_with = "display_path_string_map")]
    pub(crate) patches: Option<HashMap<PathBuf, String>>,
    #[serde(default)]
    pub(crate) target_path: Option<PathBuf>,
    #[serde(default)]
    pub(crate) default_features: Option<bool>,
    #[serde(default)]
    pub(crate) features: Option<Vec<String>>,
    pub(crate) cargo_extra_args: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_map_or_seq")]
    #[display_lua(convert_with = "display_include")]
    pub(crate) include: Option<HashMap<LuaTableKey, PathBuf>>,
    #[serde(default)]
    pub(crate) lang: Option<String>,
    #[serde(default)]
    pub(crate) parser: Option<bool>,
    #[serde(default)]
    pub(crate) generate: Option<bool>,
    #[serde(default)]
    pub(crate) location: Option<PathBuf>,
    #[serde(default)]
    #[display_lua(convert_with = "display_path_string_map")]
    pub(crate) queries: Option<HashMap<PathBuf, String>>,
}

impl PartialOverride for BuildSpecInternal {
    type Err = ModuleSpecAmbiguousPlatformOverride;

    fn apply_overrides(&self, override_spec: &Self) -> Result<Self, Self::Err> {
        override_build_spec_internal(self, override_spec)
    }
}

impl PlatformOverridable for BuildSpecInternal {
    type Err = Infallible;

    fn on_nil<T>() -> Result<PerPlatform<T>, <Self as PlatformOverridable>::Err>
    where
        T: PlatformOverridable,
        T: Default,
    {
        Ok(PerPlatform::default())
    }
}

fn override_build_spec_internal(
    base: &BuildSpecInternal,
    override_spec: &BuildSpecInternal,
) -> Result<BuildSpecInternal, ModuleSpecAmbiguousPlatformOverride> {
    Ok(BuildSpecInternal {
        build_type: override_opt(&override_spec.build_type, &base.build_type),
        builtin_spec: match (
            override_spec.builtin_spec.clone(),
            base.builtin_spec.clone(),
        ) {
            (Some(override_val), Some(base_spec_map)) => {
                Some(base_spec_map.into_iter().chain(override_val).try_fold(
                    HashMap::default(),
                    |mut acc: HashMap<LuaTableKey, ModuleSpecInternal>,
                     (k, module_spec_override)|
                     -> Result<
                        HashMap<LuaTableKey, ModuleSpecInternal>,
                        ModuleSpecAmbiguousPlatformOverride,
                    > {
                        let overridden = match acc.get(&k) {
                            None => module_spec_override,
                            Some(base_module_spec) => {
                                base_module_spec.apply_overrides(&module_spec_override)?
                            }
                        };
                        acc.insert(k, overridden);
                        Ok(acc)
                    },
                )?)
            }
            (override_val @ Some(_), _) => override_val,
            (_, base_val @ Some(_)) => base_val,
            _ => None,
        },
        makefile: override_opt(&override_spec.makefile, &base.makefile),
        make_build_target: override_opt(&override_spec.make_build_target, &base.make_build_target),
        build_pass: override_opt(&override_spec.build_pass, &base.build_pass),
        make_install_target: override_opt(
            &override_spec.make_install_target,
            &base.make_install_target,
        ),
        install_pass: override_opt(&override_spec.install_pass, &base.install_pass),
        make_build_variables: merge_map_opts(
            &override_spec.make_build_variables,
            &base.make_build_variables,
        ),
        make_install_variables: merge_map_opts(
            &override_spec.make_install_variables,
            &base.make_build_variables,
        ),
        variables: merge_map_opts(&override_spec.variables, &base.variables),
        cmake_lists_content: override_opt(
            &override_spec.cmake_lists_content,
            &base.cmake_lists_content,
        ),
        build_command: override_opt(&override_spec.build_command, &base.build_command),
        install_command: override_opt(&override_spec.install_command, &base.install_command),
        install: override_opt(&override_spec.install, &base.install),
        copy_directories: match (
            override_spec.copy_directories.clone(),
            base.copy_directories.clone(),
        ) {
            (Some(override_vec), Some(base_vec)) => {
                let merged: Vec<PathBuf> =
                    base_vec.into_iter().chain(override_vec).unique().collect();
                Some(merged)
            }
            (None, base_vec @ Some(_)) => base_vec,
            (override_vec @ Some(_), None) => override_vec,
            _ => None,
        },
        patches: override_opt(&override_spec.patches, &base.patches),
        target_path: override_opt(&override_spec.target_path, &base.target_path),
        default_features: override_opt(&override_spec.default_features, &base.default_features),
        features: override_opt(&override_spec.features, &base.features),
        cargo_extra_args: override_opt(&override_spec.cargo_extra_args, &base.cargo_extra_args),
        include: merge_map_opts(&override_spec.include, &base.include),
        lang: override_opt(&override_spec.lang, &base.lang),
        parser: override_opt(&override_spec.parser, &base.parser),
        generate: override_opt(&override_spec.generate, &base.generate),
        location: override_opt(&override_spec.location, &base.location),
        queries: merge_map_opts(&override_spec.queries, &base.queries),
    })
}

fn override_opt<T: Clone>(override_opt: &Option<T>, base: &Option<T>) -> Option<T> {
    match override_opt.clone() {
        override_val @ Some(_) => override_val,
        None => base.clone(),
    }
}

fn merge_map_opts<K, V>(
    override_map: &Option<HashMap<K, V>>,
    base_map: &Option<HashMap<K, V>>,
) -> Option<HashMap<K, V>>
where
    K: Clone,
    K: Eq,
    K: std::hash::Hash,
    V: Clone,
{
    match (override_map.clone(), base_map.clone()) {
        (Some(override_map), Some(base_map)) => {
            Some(base_map.into_iter().chain(override_map).collect())
        }
        (_, base_map @ Some(_)) => base_map,
        (override_map @ Some(_), _) => override_map,
        _ => None,
    }
}

/// Maps `build.type` to an enum.
#[derive(Debug, PartialEq, Deserialize, Clone)]
#[serde(rename_all = "lowercase", remote = "BuildType")]
#[derive(Default)]
pub(crate) enum BuildType {
    /// "builtin" or "module"
    #[default]
    Builtin,
    /// "make"
    Make,
    /// "cmake"
    CMake,
    /// "command"
    Command,
    /// "none"
    None,
    /// external Lua rock
    LuaRock(String),
    #[serde(rename = "rust-mlua")]
    RustMlua,
    #[serde(rename = "treesitter-parser")]
    TreesitterParser,
    Source,
}

impl BuildType {
    pub(crate) fn luarocks_build_backend(&self) -> Option<LuaDependencySpec> {
        match self {
            &BuildType::Builtin
            | &BuildType::Make
            | &BuildType::CMake
            | &BuildType::Command
            | &BuildType::None
            | &BuildType::LuaRock(_)
            | &BuildType::Source => None,
            &BuildType::RustMlua => unsafe {
                Some(
                    PackageReq::parse("luarocks-build-rust-mlua >= 0.2.6")
                        .unwrap_unchecked()
                        .into(),
                )
            },
            &BuildType::TreesitterParser => {
                Some(PackageName::new("luarocks-build-treesitter-parser".into()).into())
            } // IMPORTANT: If adding another luarocks build backend,
              // make sure to also add it to the filters in `operations::resolve::do_get_all_dependencies`.
        }
    }
}

// Special Deserialize case for BuildType:
// Both "module" and "builtin" map to `Builtin`
impl<'de> Deserialize<'de> for BuildType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if s == "builtin" || s == "module" {
            Ok(Self::Builtin)
        } else {
            match Self::deserialize(s.clone().into_deserializer()) {
                Err(_) => Ok(Self::LuaRock(s)),
                ok => ok,
            }
        }
    }
}

impl Display for BuildType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildType::Builtin => write!(f, "builtin"),
            BuildType::Make => write!(f, "make"),
            BuildType::CMake => write!(f, "cmake"),
            BuildType::Command => write!(f, "command"),
            BuildType::None => write!(f, "none"),
            BuildType::LuaRock(s) => write!(f, "{s}"),
            BuildType::RustMlua => write!(f, "rust-mlua"),
            BuildType::TreesitterParser => write!(f, "treesitter-parser"),
            BuildType::Source => write!(f, "source"),
        }
    }
}

impl DisplayAsLuaValue for BuildType {
    fn display_lua_value(&self) -> DisplayLuaValue {
        DisplayLuaValue::String(self.to_string())
    }
}

impl DisplayAsLuaValue for InstallSpec {
    fn display_lua_value(&self) -> DisplayLuaValue {
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
        let build_type: BuildType = serde_json::from_str("\"builtin\"").unwrap();
        assert_eq!(build_type, BuildType::Builtin);
        let build_type: BuildType = serde_json::from_str("\"module\"").unwrap();
        assert_eq!(build_type, BuildType::Builtin);
        let build_type: BuildType = serde_json::from_str("\"make\"").unwrap();
        assert_eq!(build_type, BuildType::Make);
        let build_type: BuildType = serde_json::from_str("\"custom_build_backend\"").unwrap();
        assert_eq!(
            build_type,
            BuildType::LuaRock("custom_build_backend".into())
        );
        let build_type: BuildType = serde_json::from_str("\"rust-mlua\"").unwrap();
        assert_eq!(build_type, BuildType::RustMlua);
    }

    #[test]
    pub fn install_spec_roundtrip() {
        let spec = InstallSpec {
            lua: HashMap::from([(
                "mymod".parse::<LuaModule>().unwrap(),
                "src/mymod.lua".into(),
            )]),
            lib: HashMap::from([("mylib".into(), "lib/mylib.so".into())]),
            conf: HashMap::from([("myconf".into(), "conf/myconf.cfg".into())]),
            bin: HashMap::from([("mybinary".into(), "bin/mybinary".into())]),
        };
        let lua = spec.display_lua().to_string();
        let restored: InstallSpec = eval_lua_global(&lua, "install");
        assert_eq!(spec, restored);
    }

    #[test]
    pub fn install_spec_empty_roundtrip() {
        let spec = InstallSpec::default();
        let lua = spec.display_lua().to_string();
        let lua = if lua.trim().is_empty() {
            "install = {}".to_string()
        } else {
            lua
        };
        let restored: InstallSpec = eval_lua_global(&lua, "install");
        assert_eq!(spec, restored);
    }

    #[test]
    pub fn build_spec_internal_builtin_roundtrip() {
        let spec = BuildSpecInternal {
            build_type: Some(BuildType::Builtin),
            builtin_spec: Some(HashMap::from([(
                LuaTableKey::StringKey("mymod".into()),
                ModuleSpecInternal::SourcePath("src/mymod.lua".into()),
            )])),
            install: Some(InstallSpec {
                lua: HashMap::from([(
                    "extra".parse::<LuaModule>().unwrap(),
                    "src/extra.lua".into(),
                )]),
                bin: HashMap::from([("mytool".into(), "bin/mytool".into())]),
                ..Default::default()
            }),
            copy_directories: Some(vec!["docs".into()]),
            ..Default::default()
        };
        let lua = spec.display_lua().to_string();
        let restored: BuildSpecInternal = eval_lua_global(&lua, "build");
        assert_eq!(spec, restored);
    }

    #[test]
    pub fn build_spec_internal_make_roundtrip() {
        let spec = BuildSpecInternal {
            build_type: Some(BuildType::Make),
            makefile: Some("GNUmakefile".into()),
            make_build_target: Some("all".into()),
            make_install_target: Some("install".into()),
            make_build_variables: Some(HashMap::from([("CFLAGS".into(), "-O2".into())])),
            make_install_variables: Some(HashMap::from([("PREFIX".into(), "/usr/local".into())])),
            variables: Some(HashMap::from([("LUA_LIBDIR".into(), "/usr/lib".into())])),
            ..Default::default()
        };
        let lua = spec.display_lua().to_string();
        let restored: BuildSpecInternal = eval_lua_global(&lua, "build");
        assert_eq!(spec, restored);
    }
}
