//! Conversion from the Lua-facing build spec to the public one.

use super::super::platform;
use super::super::platform::PartialOverride;

use super::backend::builtin::internal::{
    ModulePathsMissingSources, ModuleSpec, ModuleSpecAmbiguousPlatformOverride,
};
use super::backend::ParseLuaModuleError;
use super::*;

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("'builtin' modules should not have list elements")]
    ModulesHaveListElements,
    #[error("no 'modules' specified for the 'rust-mlua' build backend")]
    NoModulesSpecified,
    #[error("no 'lang' specified for 'treesitter-parser' build backend")]
    NoTreesitterParserLanguageSpecified,
    #[error("invalid 'rust-mlua' modules format")]
    InvalidRustMLuaFormat,
    #[error(transparent)]
    #[diagnostic(transparent)]
    ModulePathsMissingSources(#[from] ModulePathsMissingSources),
    #[error(transparent)]
    #[diagnostic(transparent)]
    ParseLuaModuleError(#[from] ParseLuaModuleError),
}

#[derive(Debug, PartialEq, Deserialize, Default, Clone, lux_macros::AsLuaKV)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[display_lua(key = "build")]
pub(crate) struct Spec {
    /// The build backend to use: "builtin", "make", "cmake",
    /// "command", "none", "rust-mlua", "rust-binary", or "treesitter-parser".
    /// Any other value is treated as a custom LuaRocks build backend.
    /// Default: "builtin".
    ///
    /// `make` requires Make, `cmake` requires CMake, and `rust-mlua`/`rust-binary`
    /// require a Rust toolchain. Set `type = "none"` to copy source files
    /// without building.
    ///
    /// For Luau projects, `builtin` auto-detects `.luau` modules. Native
    /// dependencies are not supported. Luau modules are transpiled to Lua when
    /// the project targets a Lua runtime.
    #[serde(rename = "type", default)]
    #[display_lua(rename = "type")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[build]
type = "builtin"
"#))
    )]
    pub(crate) build_type: Option<Type>,
    /// Modules to build, mapping module names (as used by `require`) to a source
    /// path, a list of C sources, or a table with `sources`, `libraries`,
    /// `defines`, `incdirs`, and `libdirs`.
    #[serde(
        rename = "modules",
        default,
        deserialize_with = "deserialize_map_or_seq"
    )]
    #[display_lua(rename = "modules", convert_with = "display_builtin_spec")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Option<std::collections::BTreeMap<String, crate::schema::ModuleSpec>>")
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[build.modules]
"foo.main" = "lua/foo.lua"
"foo.bla" = ["src/bla.c"]

[build.modules."foo.core"]
sources = ["src/core.c", "src/extra.c"]
libraries = ["ssl"]
defines = ["FOO=bar", "USE_BLA"]
incdirs = []
libdirs = []
"#))
    )]
    pub(crate) builtin_spec: Option<HashMap<serde_util::LuaTableKey, ModuleSpec>>,
    /// Makefile to use. Default: "Makefile" ("Makefile.win" on Windows).
    #[serde(default)]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[build]
type = "make"
makefile = "Makefile"
build_target = ""
build_pass = true
install_target = "install"
install_pass = true
build_variables = {}
install_variables = {}
variables = {}
"#))
    )]
    pub(crate) makefile: Option<PathBuf>,
    /// Target passed to `make` during the build pass. Default: empty.
    #[serde(rename = "build_target", default)]
    #[display_lua(rename = "build_target")]
    pub(crate) make_build_target: Option<String>,
    /// Whether to perform a build pass. Default: `true`.
    #[serde(default)]
    pub(crate) build_pass: Option<bool>,
    /// Target passed to `make` during the install pass. Default: "install".
    #[serde(rename = "install_target", default)]
    #[display_lua(rename = "install_target")]
    pub(crate) make_install_target: Option<String>,
    /// Whether to perform an install pass. Default: `true`.
    #[serde(default)]
    pub(crate) install_pass: Option<bool>,
    /// Make variable assignments for the build pass.
    #[serde(rename = "build_variables", default)]
    #[display_lua(rename = "build_variables")]
    pub(crate) make_build_variables: Option<HashMap<String, String>>,
    /// Make variable assignments for the install pass.
    #[serde(rename = "install_variables", default)]
    #[display_lua(rename = "install_variables")]
    pub(crate) make_install_variables: Option<HashMap<String, String>>,
    /// Make or CMake variable assignments applied to both passes.
    #[serde(default)]
    pub(crate) variables: Option<HashMap<String, String>>,
    /// Contents of the generated "CMakeLists.txt". If unset, the project's own
    /// file is used.
    ///
    /// Environment variables: `CMAKE_MODULE_PATH`, `CMAKE_INCLUDE_PATH`,
    /// `CMAKE_LIBRARY_PATH`.
    #[serde(rename = "cmake", default)]
    #[display_lua(rename = "cmake")]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[build]
type = "cmake"
cmake = """
<content>
"""
"#))
    )]
    pub(crate) cmake_lists_content: Option<String>,
    /// Command to run to build the package.
    #[serde(default)]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[build]
type = "command"
build_command = "make"
install_command = "make install"
"#))
    )]
    pub(crate) build_command: Option<String>,
    /// Command to run to install the package.
    #[serde(default)]
    pub(crate) install_command: Option<String>,
    /// Files to install, by category ("lua", "lib", "conf", "bin").
    #[serde(default)]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[build.install.lua]
"foo.bar" = "src/bar.lua"

[build.install.lib]
"foo.bar" = "lib/bar.so"

[build.install.conf]

[build.install.bin]
"#))
    )]
    pub(crate) install: Option<super::Install>,
    /// Directories in the source tree to copy as-is into the rock's "etc" tree.
    /// Do not use "lua", "lib", "rock_manifest", or the rockspec file name.
    #[serde(default, deserialize_with = "deserialize_copy_directories")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<Vec<String>>"))]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[build]
copy_directories = ["plugin", "ftplugin"]
"#))
    )]
    pub(crate) copy_directories: Option<Vec<PathBuf>>,
    /// Patches to apply before building, mapping file names to patch contents.
    /// Lux accepts any format supported by the [`diffy`](https://docs.rs/diffy/latest/diffy/) crate.
    /// LuaRocks only supports unified diff.
    #[serde(default)]
    #[display_lua(convert_with = "display_path_string_map")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Option<std::collections::BTreeMap<String, String>>")
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[build.patches]
"lua51-support.diff" = """
--- old/mymodule.c....
"""
"#))
    )]
    pub(crate) patches: Option<HashMap<PathBuf, String>>,
    /// The cargo target directory, if it is not `<source>/target`.
    #[serde(default)]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[build]
type = "rust-mlua"
target_path = "target"
default_features = false
features = []
cargo_extra_args = []

[build.modules]
"foo.main" = "alt_name"

[build.include]
"relative/path/to/file.lua" = "target-file.lua"
"#))
    )]
    pub(crate) target_path: Option<PathBuf>,
    /// Whether to build a cargo package with default features. Default: `true`.
    #[serde(default)]
    pub(crate) default_features: Option<bool>,
    /// Cargo features to enable.
    #[serde(default)]
    pub(crate) features: Option<Vec<String>>,
    /// Additional arguments to pass to cargo.
    pub(crate) cargo_extra_args: Option<Vec<String>>,
    /// The cargo package to build.
    /// Required for "rust-binary" in multi-package workspaces.
    #[serde(default)]
    pub(crate) package: Option<String>,
    /// Additional files to copy into the rock's "lua" directory, mapping source
    /// paths to destinations.
    #[serde(default, deserialize_with = "deserialize_map_or_seq")]
    #[display_lua(convert_with = "display_include")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Option<std::collections::BTreeMap<String, String>>")
    )]
    pub(crate) include: Option<HashMap<serde_util::LuaTableKey, PathBuf>>,
    /// The parser language, e.g. "haskell". Required for "treesitter-parser".
    #[serde(default)]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[build]
type = "treesitter-parser"
lang = "haskell"
parser = true
generate = true
location = "libs/tree-sitter-haskell"

[build.queries."highlights.scm"] = """
(signature
  name: (variable) @function)
"""
"#))
    )]
    pub(crate) lang: Option<String>,
    /// Whether to build the tree-sitter parser. Set to `false` for query-only packages.
    /// Default: `false`.
    #[serde(default)]
    pub(crate) parser: Option<bool>,
    /// Whether to generate the tree-sitter parser sources before building.
    /// Default: `false`.
    #[serde(default)]
    pub(crate) generate: Option<bool>,
    /// Location of the tree-sitter grammar, relative to the source root.
    #[serde(default)]
    pub(crate) location: Option<PathBuf>,
    /// Embedded queries to install into "etc/queries", mapping file names to
    /// contents.
    #[serde(default)]
    #[display_lua(convert_with = "display_path_string_map")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Option<std::collections::BTreeMap<String, String>>")
    )]
    pub(crate) queries: Option<HashMap<PathBuf, String>>,
}

impl platform::PartialOverride for Spec {
    type Err = ModuleSpecAmbiguousPlatformOverride;

    fn apply_overrides(&self, override_spec: &Self) -> Result<Self, Self::Err> {
        override_build_spec_internal(self, override_spec)
    }
}

impl platform::Overridable for Spec {
    type Err = Infallible;

    fn on_nil<T>() -> Result<PerPlatform<T>, <Self as platform::Overridable>::Err>
    where
        T: platform::Overridable,
        T: Default,
    {
        Ok(PerPlatform::default())
    }
}

fn override_build_spec_internal(
    base: &Spec,
    override_spec: &Spec,
) -> Result<Spec, ModuleSpecAmbiguousPlatformOverride> {
    Ok(Spec {
        build_type: override_opt(&override_spec.build_type, &base.build_type),
        builtin_spec: match (
            override_spec.builtin_spec.clone(),
            base.builtin_spec.clone(),
        ) {
            (Some(override_val), Some(base_spec_map)) => {
                Some(base_spec_map.into_iter().chain(override_val).try_fold(
                    HashMap::default(),
                    |mut acc: HashMap<serde_util::LuaTableKey, ModuleSpec>,
                     (k, module_spec_override)|
                     -> Result<
                        HashMap<serde_util::LuaTableKey, ModuleSpec>,
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
        package: override_opt(&override_spec.package, &base.package),
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
#[serde(rename_all = "lowercase", remote = "Type")]
#[derive(Default)]
pub(crate) enum Type {
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
    #[serde(rename = "rust-binary")]
    RustBinary,
    #[serde(rename = "treesitter-parser")]
    TreesitterParser,
    Source,
}

impl Type {
    pub(crate) fn luarocks_build_backend(&self) -> Option<lua_dependency::Spec> {
        match self {
            &Type::Builtin
            | &Type::Make
            | &Type::CMake
            | &Type::Command
            | &Type::None
            | &Type::Source => None,
            Type::LuaRock(s) => Some(package::Name::new(format!("luarocks-build-{s}")).into()),
            &Type::RustMlua => unsafe {
                Some(
                    package::Req::parse("luarocks-build-rust-mlua >= 0.2.6")
                        .unwrap_unchecked()
                        .into(),
                )
            },
            &Type::RustBinary => unsafe {
                Some(
                    package::Req::parse("luarocks-build-rust-binary >= 4.0.0")
                        .unwrap_unchecked()
                        .into(),
                )
            },
            &Type::TreesitterParser => {
                Some(package::Name::new("luarocks-build-treesitter-parser >= 6.1.0".into()).into())
            } // IMPORTANT: If adding another luarocks build backend,
              // make sure to also add it to the filters in `pipeline::resolve`.
        }
    }
}

// Special Deserialize case for Type:
// Both "module" and "builtin" map to `Builtin`
impl<'de> Deserialize<'de> for Type {
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

impl Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Builtin => write!(f, "builtin"),
            Type::Make => write!(f, "make"),
            Type::CMake => write!(f, "cmake"),
            Type::Command => write!(f, "command"),
            Type::None => write!(f, "none"),
            Type::LuaRock(s) => write!(f, "{s}"),
            Type::RustMlua => write!(f, "rust-mlua"),
            Type::RustBinary => write!(f, "rust-binary"),
            Type::TreesitterParser => write!(f, "treesitter-parser"),
            Type::Source => write!(f, "source"),
        }
    }
}

impl display::AsLuaValue for Type {
    fn display_lua_value(&self) -> display::LuaValue {
        display::LuaValue::String(self.to_string())
    }
}
