use std::collections::HashMap;
use std::io;
use std::path::PathBuf;

use nonempty::NonEmpty;
use serde::Deserialize;
use ssri::Integrity;

use crate::config::Config;
use crate::fs;
use crate::hash::HasIntegrity;
use crate::lua;
use crate::operations::run;
use crate::package;
use crate::project::{TomlDeError, TOML_FILE_NAME};
use crate::rockspec::{self, lua_dependency, LuaVersionCompatibility};

use super::gen::{PackageVersionTemplate, RockSourceTemplate};
use super::parse_map_to_dependency_vec_opt;
use super::GenerateVersionError;

/// The `lux.toml` file for a project.
/// The only required fields are `package` and `build`, which are required to build a project using `lux build`.
/// The rest of the fields are optional, but are required to build a rockspec.
#[derive(Clone, Debug, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Partial {
    /// The name of the rock. Lux converts it to lower case when publishing.
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"package = "my-lua-project""#))
    )]
    pub(crate) package: package::Name,
    /// The package version. If unset, Lux derives it from the current SemVer
    /// git tag (optional `v` prefix), falling back to "dev".
    ///
    /// Versions like `1.0` are corrected to `1.0.0`.
    /// Non-SemVer versions (e.g. `1.0-rc1` instead of `1.0.0-rc1`) are treated
    /// as Dev versions and cannot be compared with SemVer versions.
    #[serde(default, rename = "version")]
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"version = "1.0.0""#))
    )]
    pub(crate) version_template: PackageVersionTemplate,
    /// How to build the project. See the build backend fields below.
    ///
    /// Rocks with C sources require a C compiler. Lux does not bundle one.
    #[serde(default)]
    pub(crate) build: rockspec::build::internal::Spec,
    /// The LuaRocks rockspec format version to write to the generated rockspec ("1.0", "2.0", or "3.0").
    /// Default: "3.0".
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"rockspec_format = "3.0""#))
    )]
    pub(crate) rockspec_format: Option<rockspec::Format>,
    /// How to run the project with `lx run`.
    #[serde(default)]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[run]
args = ["src/main.lua"]
"#))
    )]
    pub(crate) run: Option<RunSpec>,
    /// The Lua version requirement, e.g. ">= 5.1". Required to build a rockspec.
    /// Set to "luau" to build and run the project with Luau.
    #[serde(default)]
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"lua = ">=5.1""#))
    )]
    pub(crate) lua: Option<package::VersionReq>,
    /// Metadata published to LuaRocks.
    #[serde(default)]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[description]
summary = "A one-line description of the package"
detailed = """
A longer description of the package
that often spans multiple lines.
"""
license = "GPLv2"
homepage = "https://lumen-oss.github.io"
issues_url = "https://github.com/lumen-oss/lux/issues"
maintainer = "neorocks org"
labels = ["neovim", "tls", "https"]
"#))
    )]
    pub(crate) description: Option<rockspec::description::Spec>,
    /// Platforms this rock supports, mapping platform identifiers to `true` or
    /// `false` (e.g. `linux = true`).
    #[serde(default)]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Option<std::collections::BTreeMap<String, bool>>")
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[supported_platforms]
linux = true
windows = false
"#))
    )]
    pub(crate) supported_platforms: Option<HashMap<rockspec::platform::Identifier, bool>>,
    /// Runtime Lua dependencies, mapping package names to a version requirement
    /// (e.g. `foo = "1.0"`) or to a detailed table with `version`, `opt`, `pin`,
    /// `git`, `rev`, and `path` keys.
    ///
    /// Version requirements support the operators `==`, `~=`, `<`, `>`, `<=`,
    /// `>=`, and `~>`. `~> 2` means `>= 2, < 3`. An operator is optional;
    /// without one, the requirement is treated as `==`. Version requirements
    /// are mandatory.
    ///
    /// Dependencies are declared under `lua`, `[dependencies]`,
    /// `[build_dependencies]`, `[test_dependencies]`, and
    /// `[external_dependencies]`.
    ///
    /// Git dependencies support the `github:` (default), `gitlab:`,
    /// `codeberg:`, and `sourcehut:` shorthands, as well as full HTTPS and SSH
    /// URLs. If `rev` is unset, `version` is used as the commit SHA or tag.
    /// Lux searches the repository root for a `lux.toml` or `.rockspec` to
    /// determine the build specification, defaulting to `builtin` if none is
    /// found. Rocks with git dependencies cannot be published to LuaRocks. Git
    /// dependencies that require a custom LuaRocks build backend are not
    /// supported.
    #[serde(default, deserialize_with = "parse_map_to_dependency_vec_opt")]
    #[cfg_attr(
        feature = "schema",
        schemars(
            with = "Option<std::collections::BTreeMap<String, crate::schema::DependencyEntry>>"
        )
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[dependencies]
luafilesystem = "1.0.0"

[dependencies.argparse]
version = "0.7"
opt = true

[dependencies.foo]
version = "09511782201302ade916d4b250d01a6c61b56844"
git = "github:lunarmodules/foo"

[dependencies.bar]
path = "../bar"
version = "1.0.0"
"#))
    )]
    pub(crate) dependencies: Option<Vec<lua_dependency::Spec>>,
    /// Dependencies needed at build time. Same syntax as `dependencies`.
    #[serde(default, deserialize_with = "parse_map_to_dependency_vec_opt")]
    #[cfg_attr(
        feature = "schema",
        schemars(
            with = "Option<std::collections::BTreeMap<String, crate::schema::DependencyEntry>>"
        )
    )]
    pub(crate) build_dependencies: Option<Vec<lua_dependency::Spec>>,
    /// System (C/FFI) dependencies, discovered via pkg-config. Each entry may
    /// set `header` and/or `library`.
    #[serde(default)]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[external_dependencies.OPENSSL]
header = "openssl/ssl.h"
library = "ssl"
"#))
    )]
    pub(crate) external_dependencies: Option<HashMap<String, rockspec::external_dependency::Spec>>,
    /// Dependencies needed when running `lx test`. Same syntax as `dependencies`.
    #[serde(default, deserialize_with = "parse_map_to_dependency_vec_opt")]
    #[cfg_attr(
        feature = "schema",
        schemars(
            with = "Option<std::collections::BTreeMap<String, crate::schema::DependencyEntry>>"
        )
    )]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[test_dependencies]
busted = ">=2.0.0"
nlua = "~>0.3"
"#))
    )]
    pub(crate) test_dependencies: Option<Vec<lua_dependency::Spec>>,
    /// Templates for generating the source URL of a release.
    ///
    /// Variable substitutions:
    /// - `$(PACKAGE)`: package name
    /// - `$(VERSION)`: package version (from the git tag if not declared)
    /// - `$(REF)`: git tag or revision (priority: SemVer tag, tag, commit SHA)
    /// - `$(<VAR_NAME>)`: any environment variable
    #[serde(default, rename = "source")]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[source]
url = "https://host.com/owner/$(PACKAGE)/archive/refs/tags/$(REF).zip"
dev = "git+https://host.com/owner/$(PACKAGE).git"
"#))
    )]
    pub(crate) source_template: RockSourceTemplate,
    /// How to run the test suite with `lx test`.
    ///
    /// The `busted` backend is used when a `.busted` file exists and no backend
    /// is specified. `busted-nlua` is used when a `.busted` file exists or
    /// `busted` and `nlua` are `test_dependencies`. The `busted-nlua` backend is
    /// currently broken on macOS and Windows. Luau projects default to `tiniest`.
    #[serde(default)]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[test]
type = "busted"
flags = []
"#))
    )]
    pub(crate) test: Option<rockspec::test::internal::Spec>,
    /// Additional install and deployment options.
    #[serde(default)]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[deploy]
wrap_bin_scripts = false
"#))
    )]
    pub(crate) deploy: Option<rockspec::deploy::Spec>,
    /// Lux-specific project settings, ignored when generating a rockspec.
    #[serde(default)]
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-lux-toml-example" = r#"
[project]
root_dir = ".."
"#))
    )]
    pub(crate) project: ProjectSpec,

    /// Used to bind the TOML to the directory containing the `lux.toml`.
    #[serde(skip, default = "PathBuf::new")]
    pub(super) lux_toml_dir: PathBuf,
}

impl HasIntegrity for Partial {
    async fn hash(&self) -> io::Result<Integrity> {
        let toml_file = self.lux_toml_dir.join(TOML_FILE_NAME);
        let content = fs::sync::read_to_string(&toml_file).map_err(io::Error::other)?;
        Ok(Integrity::from(&content))
    }
}

impl Partial {
    pub(crate) fn new(name: &str, str: &str, lux_toml_dir: PathBuf) -> Result<Self, TomlDeError> {
        let mut this: Self = crate::project::parse_toml(name, str)?;
        this.lux_toml_dir = lux_toml_dir.to_path_buf();
        this.project.root = match this.project.root_dir {
            Some(ref root_dir) => {
                let relative_project_root = lux_toml_dir.join(root_dir);
                std::fs::canonicalize(&relative_project_root).unwrap_or(relative_project_root)
            }
            None => lux_toml_dir,
        };
        Ok(this)
    }

    /// This project's Lua version requirement, if any
    pub fn lua(&self) -> Option<&package::VersionReq> {
        self.lua.as_ref()
    }

    pub fn package(&self) -> &package::Name {
        &self.package
    }

    /// Returns the current package version, which may be generated from a template
    pub fn version(&self) -> Result<package::Version, GenerateVersionError> {
        self.version_template.try_generate(&self.lux_toml_dir, None)
    }

    /// Merge the `ProjectToml` struct with an unvalidated `LuaRockspec`.
    /// The final merged struct can then be validated.
    pub fn merge(self, other: rockspec::lua::Partial) -> Self {
        Partial {
            package: other.package.unwrap_or(self.package),
            version_template: self.version_template,
            lua: other
                .dependencies
                .as_ref()
                .and_then(|deps| {
                    deps.iter()
                        .find(|dep| dep.name() == &"lua".into())
                        .and_then(|dep| {
                            if dep.version_req().is_any() {
                                None
                            } else {
                                Some(dep.version_req().clone())
                            }
                        })
                })
                .or(self.lua),
            build: other.build.unwrap_or(self.build),
            run: self.run,
            description: other.description.or(self.description),
            supported_platforms: other
                .supported_platforms
                .map(|platform_support| platform_support.platforms().clone())
                .or(self.supported_platforms),
            dependencies: other
                .dependencies
                .map(|deps| {
                    deps.into_iter()
                        .filter(|dep| dep.name() != &"lua".into())
                        .collect()
                })
                .or(self.dependencies),
            build_dependencies: other.build_dependencies.or(self.build_dependencies),
            test_dependencies: other.test_dependencies.or(self.test_dependencies),
            external_dependencies: other.external_dependencies.or(self.external_dependencies),
            source_template: self.source_template,
            test: other.test.or(self.test),
            deploy: other.deploy.or(self.deploy),
            rockspec_format: other.format.or(self.rockspec_format),

            project: self.project,
            lux_toml_dir: self.lux_toml_dir.clone(),
        }
    }

    pub(super) fn resolve_lua_dependency_spec(
        &self,
        dep: lua_dependency::Spec,
    ) -> lua_dependency::Spec {
        match &dep.source {
            Some(rockspec::source::Spec::File(path)) if path.is_dir() => dep,
            Some(rockspec::source::Spec::File(path)) => lua_dependency::Spec {
                source: Some(rockspec::source::Spec::File(self.lux_toml_dir.join(path))),
                ..dep
            },
            _ => dep,
        }
    }
}

impl LuaVersionCompatibility for Partial {
    fn validate_lua_version(&self, version: &lua::Version) -> Result<(), rockspec::Error> {
        if self.supports_lua_version(version) {
            Ok(())
        } else {
            Err(rockspec::Error::UnsupportedLuaVersion(
                version.clone(),
                self.package().to_owned(),
                Box::new(
                    self.version_template
                        .try_generate(&self.lux_toml_dir, None)
                        .unwrap_or(package::Version::default_dev_version()),
                ),
            ))
        }
    }

    fn validate_lua_version_from_config(&self, config: &Config) -> Result<(), rockspec::Error> {
        let _ = self.lua_version_matches(config)?;
        Ok(())
    }

    fn lua_version_matches(&self, config: &Config) -> Result<lua::Version, rockspec::Error> {
        let version = lua::Version::from(config)?.clone();
        if self.supports_lua_version(&version) {
            Ok(version)
        } else {
            Err(rockspec::Error::UnsupportedLuaVersion(
                version,
                self.package.clone(),
                Box::new(
                    self.version_template
                        .try_generate(&self.lux_toml_dir, None)
                        .unwrap_or(package::Version::default_dev_version()),
                ),
            ))
        }
    }

    fn supports_lua_version(&self, lua_version: &lua::Version) -> bool {
        self.lua.as_ref().is_none_or(|lua| {
            if lua.is_luau() {
                return matches!(lua_version, lua::Version::Luau);
            }
            lua.matches(&lua_version.as_version())
        })
    }

    fn lua_version(&self) -> Option<lua::Version> {
        if self.lua.as_ref().is_some_and(|lua| lua.is_luau()) {
            return Some(lua::Version::Luau);
        }
        for (possibility, version) in [
            ("5.5.0", lua::Version::Lua55),
            ("5.4.0", lua::Version::Lua54),
            ("5.3.0", lua::Version::Lua53),
            ("5.2.0", lua::Version::Lua52),
            ("5.1.0", lua::Version::Lua51),
        ] {
            let possibility = unsafe { possibility.parse().unwrap_unchecked() };
            if self
                .lua
                .as_ref()
                .is_none_or(|lua| lua.matches(&possibility))
            {
                return Some(version);
            }
        }
        None
    }
}

/// Lux-specific project configuration.
///
/// This is not part of the rockspec format and is ignored when generating a rockspec.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub(crate) struct ProjectSpec {
    /// The project root directory, relative to the directory containing the `lux.toml`.
    /// Useful for multi-project workspaces, where the build files
    /// (e.g. the cargo workspace `Cargo.toml`) do not live next to the `lux.toml`.
    root_dir: Option<PathBuf>,
    /// The project root directory, resolved from `root_dir` when deserializing.
    #[serde(skip, default)]
    pub(crate) root: PathBuf,
}

impl Default for ProjectSpec {
    fn default() -> Self {
        Self {
            root_dir: None,
            root: PathBuf::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RunSpec {
    /// The command to execute when running the project
    pub(crate) command: Option<run::Command>,
    /// Arguments to pass to the command
    #[cfg_attr(feature = "schema", schemars(with = "Option<Vec<String>>"))]
    pub(crate) args: Option<NonEmpty<String>>,
}
