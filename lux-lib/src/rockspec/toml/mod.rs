//! Structs and utilities for `lux.toml`

use crate::fs;
use crate::git::shorthand::RemoteGitUrlShorthand;
use crate::git::{GitRef, GitSource};
use crate::hash::HasIntegrity;
use crate::lockfile::OptState;
use crate::lockfile::PinnedState;
use crate::lua;
use crate::operations::RunCommand;
use crate::package;
use crate::project::TomlDeError;
use crate::rockspec;
use crate::rockspec::display::AsLuaKV;
use crate::rockspec::lua_dependency;
use crate::ROCKSPEC_FUEL_LIMIT;
use std::io;
use std::path::Path;
use std::{collections::HashMap, path::PathBuf};

use crate::{
    build::autodetect_modules,
    config::Config,
    package::{BuildDependencies, Dependencies},
    rockspec::{LuaVersionCompatibility, Rockspec},
};
use itertools::Itertools;
use miette::Diagnostic;
use nonempty::NonEmpty;

use crate::wally;
use serde::de;
use serde::{Deserialize, Deserializer};
use ssri::Integrity;
use thiserror::Error;

mod gen;
mod validate;

use self::gen::PackageVersionTemplate;
use self::gen::RockSourceTemplate;
pub use self::gen::{GenerateSourceError, GenerateVersionError};
use crate::project::TOML_FILE_NAME;

#[derive(Deserialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)] // This is ok because it's just a Deserialize helper
enum DependencyEntry {
    Simple(package::VersionReq),
    Detailed(DependencyTableEntry),
}

#[derive(Debug, Deserialize)]
struct DependencyTableEntry {
    #[serde(default)]
    version: Option<package::VersionReq>,
    #[serde(default)]
    opt: Option<bool>,
    #[serde(default)]
    pin: Option<bool>,
    #[serde(default)]
    git: Option<RemoteGitUrlShorthand>,
    #[serde(default)]
    path: Option<PathBuf>,
    #[serde(default)]
    rev: Option<String>,
    #[serde(default)]
    wally: Option<wally::package::Req>,
}

fn parse_map_to_dependency_vec_opt<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<lua_dependency::Spec>>, D::Error>
where
    D: Deserializer<'de>,
{
    let packages: Option<HashMap<package::Name, DependencyEntry>> =
        Option::deserialize(deserializer)?;

    match packages {
        None => Ok(None),
        Some(packages) => Ok(Some(
            packages
                .into_iter()
                .map(|(name, spec)| match spec {
                    DependencyEntry::Simple(version_req) => {
                        Ok(package::Req { name, version_req }.into())
                    }
                    DependencyEntry::Detailed(entry) => {
                        let DependencyTableEntry {
                            version,
                            opt,
                            pin,
                            git,
                            path,
                            rev,
                            wally,
                        } = entry;

                        let (version_req, source) = if let Some(wally) = wally {
                            if version.is_some() || git.is_some() || rev.is_some() || path.is_some() {
                                return Err(de::Error::custom(format!(
                                    "dependency '{}' specifies 'wally', which cannot be combined with 'version', 'git', 'rev', or 'path'",
                                    name
                                )));
                            }
                            (
                                package::VersionReq::SemVer(wally.version_req().clone()),
                                Some(rockspec::source::Spec::Wally(wally)),
                            )
                        } else {
                            let version = version.ok_or_else(|| {
                                de::Error::custom(format!(
                                    "dependency '{name}' is missing a 'version' or 'wally' field"
                                ))
                            })?;
                            let source = match (git, rev, path) {
                                (None, None, None) => Ok(None),
                                (None, Some(_), None) => Err(de::Error::custom(format!(
                                    "dependency {} specifies a 'rev', but missing a 'git' field",
                                    name
                                ))),
                                (Some(git), Some(rev), None) => {
                                    Ok(Some(rockspec::source::Spec::Git(GitSource {
                                        url: git.into(),
                                        git_ref: Some(GitRef::Tag(rev)),
                                    })))
                                }
                                (Some(git), None, None) => Ok(Some(rockspec::source::Spec::Git(GitSource {
                                    url: git.into(),
                                    git_ref: Some(GitRef::Tag(
                                        version
                                            .clone()
                                            .to_string()
                                            .trim_start_matches("=")
                                            .to_string(),
                                    )),
                                }))),
                                (None, None, Some(path)) => Ok(Some(rockspec::source::Spec::File(path))),
                                (_, _, Some(_)) => Err(de::Error::custom(format!(
                                    "dependency '{}' specifies a 'path', which cannot be combined with 'git' or 'rev'",
                                    name
                                ))),
                            }?;
                            (version, source)
                        };

                        Ok(lua_dependency::Spec {
                            package_req: package::Req { name, version_req },
                            opt: OptState::from(opt.unwrap_or(false)),
                            pin: PinnedState::from(pin.unwrap_or(false)),
                            source,
                        })
                    }
                })
                .try_collect()?,
        )),
    }
}

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("error generating rockspec source")]
    #[diagnostic(forward(0))]
    GenerateSource(#[from] GenerateSourceError),
    #[error("error generating rockspec version")]
    #[diagnostic(forward(0))]
    GenerateVersion(#[from] GenerateVersionError),
    #[error("generated rockspec exceeds computational limit of {ROCKSPEC_FUEL_LIMIT} steps")]
    #[diagnostic(help(
        r#"the rockspec may be too complex or contain too many dependencies.
        Try reducing the number of dependencies or simplifying build instructions."#
    ))]
    FuelLimitExceeded,
    #[error(
        r#"generated invalid Lua from TOML:
{0}
"#
    )]
    #[diagnostic(help("this is a bug in Lux. please report it."))]
    GeneratedInvalidLua(String),
    #[error("no lua version provided")]
    #[diagnostic(help(
        r#"add a `lua` field to your lux.toml, for example:

  lua = ">=5.1""#
    ))]
    NoLuaVersion,
    #[error("could not decode the test spec")]
    #[diagnostic(forward(0))]
    Decode(#[from] rockspec::test::DecodeError),
    #[error("could not decode the build spec")]
    #[diagnostic(forward(0))]
    Build(#[from] rockspec::build::internal::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    PlatformValidation(#[from] rockspec::platform::Error),
    #[error("{}copy_directories cannot contain a rockspec name", ._0.as_ref().map(|p| format!("{p}: ")).unwrap_or_default())]
    CopyDirectoriesContainRockspecName(Option<String>),
    #[error("could not decode the source spec")]
    #[diagnostic(forward(0))]
    RockSource(#[from] rockspec::source::Error),
    #[error("duplicate dependencies: {0}")]
    #[diagnostic(help("remove the duplicate entries from the [dependencies] table."))]
    DuplicateDependencies(package::List),
    #[error("duplicate test dependencies: {0}")]
    #[diagnostic(help("remove the duplicate entries from the [test_dependencies] table."))]
    DuplicateTestDependencies(package::List),
    #[error("duplicate build dependencies: {0}")]
    #[diagnostic(help("remove the duplicate entries from the [build_dependencies] table."))]
    DuplicateBuildDependencies(package::List),
    #[error(
        r#"dependencies field cannot contain 'lua'.
        Please provide the version in the top-level 'lua' field

        Example:

        ```toml
        package = "my-package"
        lua = ">=5.1"

        [dependencies]
        # do not add Lua here!
        ```
        "#
    )]
    DependenciesContainLua,
}

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
    lux_toml_dir: PathBuf,
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

    // In the not-yet-validated struct, we create getters only
    // for the non-optional fields.
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

            // Keep the project root the same, as it is not part of the lua rockspec
            project: self.project,
            lux_toml_dir: self.lux_toml_dir.clone(),
        }
    }

    fn resolve_lua_dependency_spec(&self, dep: lua_dependency::Spec) -> lua_dependency::Spec {
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

// This is automatically implemented for `Remote`,
// but we also add a special implementation for `ProjectToml` (as providing a lua version
// is required even by the non-validated struct).
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

// TODO(vhyrro): Move this struct into a different directory.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RunSpec {
    /// The command to execute when running the project
    pub(crate) command: Option<RunCommand>,
    /// Arguments to pass to the command
    #[cfg_attr(feature = "schema", schemars(with = "Option<Vec<String>>"))]
    pub(crate) args: Option<NonEmpty<String>>,
}

/// The `lux.toml` file, after being properly deserialized.
/// This struct may be used to build a local version of a project.
/// To build a rockspec, use `Remote`.
#[derive(Debug)]
pub struct Local {
    package: package::Name,
    version: package::Version,
    lua: package::VersionReq,
    rockspec_format: Option<rockspec::Format>,
    run: Option<rockspec::PerPlatform<RunSpec>>,
    description: rockspec::description::Spec,
    supported_platforms: rockspec::platform::Support,
    dependencies: rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>>,
    build_dependencies: rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>>,
    external_dependencies:
        rockspec::PerPlatform<HashMap<String, rockspec::external_dependency::Spec>>,
    test_dependencies: rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>>,
    test: rockspec::PerPlatform<rockspec::test::Spec>,
    build: rockspec::PerPlatform<rockspec::build::Spec>,
    deploy: rockspec::PerPlatform<rockspec::deploy::Spec>,

    // Used for simpler serialization
    internal: Partial,

    /// A source pointing to the current project's root.
    source: rockspec::PerPlatform<rockspec::source::Remote>,
}

impl Local {
    pub fn run(&self) -> Option<&rockspec::PerPlatform<RunSpec>> {
        self.run.as_ref()
    }

    /// Convert this project TOML to a Lua rockspec.
    /// Fails if there is no valid project root or if there are off-spec dependencies.
    pub fn to_lua_rockspec(&self) -> Result<rockspec::lua::Local, rockspec::Error> {
        if let Some(dep) = self
            .dependencies()
            .per_platform
            .values()
            .filter_map(|deps| deps.iter().find(|dep| dep.source().is_some()))
            .collect_vec()
            .first()
        {
            return Err(rockspec::lua::Error::OffSpecSpec(dep.name().clone()).into());
        }
        if let Some(dep) = self
            .build_dependencies()
            .per_platform
            .values()
            .filter_map(|deps| deps.iter().find(|dep| dep.source().is_some()))
            .collect_vec()
            .first()
        {
            return Err(rockspec::lua::Error::OffSpecBuildSpec(dep.name().clone()).into());
        }
        if let Some(dep) = self
            .test_dependencies()
            .per_platform
            .values()
            .filter_map(|deps| deps.iter().find(|dep| dep.source().is_some()))
            .collect_vec()
            .first()
        {
            return Err(rockspec::lua::Error::OffSpecTestSpec(dep.name().clone()).into());
        }
        Ok(rockspec::lua::Local::new(
            &self.to_lua_remote_rockspec_string()?,
            self.internal.lux_toml_dir.clone(),
        )?)
    }

    /// Renders the rockspec for local builds, using the rockspec's own version
    /// and its source pointing at the project root.
    pub fn to_local_rockspec(&self) -> Result<rockspec::lua::Remote, rockspec::Error> {
        let content = self.to_lua_local_rockspec_string()?;
        Ok(rockspec::lua::Local::from_project_toml(self, content)
            .into_remote_with_source(self.source.clone()))
    }

    /// Like [`Rockspec::to_lua_remote_rockspec_string`], but pointing at the project root,
    /// so projects without a release URL can still render a rockspec.
    fn to_lua_local_rockspec_string(&self) -> Result<String, Error> {
        self.render_rockspec(
            &self.version,
            Some(self.source.current_platform().source_spec.display_lua()),
        )
    }

    fn render_rockspec(
        &self,
        version: &package::Version,
        source: Option<rockspec::display::LuaKV>,
    ) -> Result<String, Error> {
        let starter = format!(
            r#"
rockspec_format = "{}"
package = "{}"
version = "{}""#,
            self.rockspec_format
                .as_ref()
                .unwrap_or(&rockspec::Format::default()),
            self.package,
            version
        );

        let mut template = Vec::new();

        if self.description != rockspec::description::Spec::default() {
            template.push(self.description.display_lua());
        }

        if self.supported_platforms != rockspec::platform::Support::default() {
            template.push(self.supported_platforms.display_lua());
        }

        {
            let mut dependencies = self.internal.dependencies.clone().unwrap_or_default();
            if !self.lua.is_luau() {
                dependencies.insert(
                    0,
                    package::Req {
                        name: "lua".into(),
                        version_req: self.lua.clone(),
                    }
                    .into(),
                );
            }
            template.push(Dependencies(&dependencies).display_lua());
        }

        let mut build_dependencies = self
            .internal
            .build_dependencies
            .as_ref()
            .cloned()
            .unwrap_or_default();

        let build_backend_dependency = self
            .internal
            .build
            .build_type
            .as_ref()
            .and_then(|build_type| build_type.luarocks_build_backend());

        if let Some(build_backend_dependency) = build_backend_dependency {
            build_dependencies.push(build_backend_dependency);
        }

        if !build_dependencies.is_empty() {
            template.push(BuildDependencies(&build_dependencies).display_lua());
        }

        match self.internal.external_dependencies {
            Some(ref external_dependencies) if !external_dependencies.is_empty() => {
                template.push(
                    rockspec::external_dependency::Dependencies(external_dependencies)
                        .display_lua(),
                );
            }
            _ => {}
        }

        if let Some(source) = source {
            template.push(source);
        }

        template.push(
            build_spec_with_autodetected_modules(&self.internal.build, &self.internal.lux_toml_dir)
                .display_lua(),
        );

        let unformatted_code = std::iter::once(starter)
            .chain(template.into_iter().map(|kv| kv.to_string()))
            .join("\n\n");
        let result = match stylua_lib::format_code(
            &unformatted_code,
            stylua_lib::Config::default(),
            None,
            stylua_lib::OutputVerification::Full,
        ) {
            Ok(formatted_code) => formatted_code,
            Err(_) => unformatted_code,
        };
        validate_generated_lua(&result)?;
        Ok(result)
    }
}

impl Rockspec for Local {
    fn package(&self) -> &package::Name {
        &self.package
    }

    fn version(&self) -> &package::Version {
        &self.version
    }

    fn description(&self) -> &rockspec::description::Spec {
        &self.description
    }

    fn supported_platforms(&self) -> &rockspec::platform::Support {
        &self.supported_platforms
    }

    fn lua(&self) -> &package::VersionReq {
        &self.lua
    }

    fn dependencies(&self) -> &rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>> {
        &self.dependencies
    }

    fn build_dependencies(&self) -> &rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>> {
        &self.build_dependencies
    }

    fn external_dependencies(
        &self,
    ) -> &rockspec::PerPlatform<HashMap<String, rockspec::external_dependency::Spec>> {
        &self.external_dependencies
    }

    fn test_dependencies(&self) -> &rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>> {
        &self.test_dependencies
    }

    fn build(&self) -> &rockspec::PerPlatform<rockspec::build::Spec> {
        &self.build
    }

    fn test(&self) -> &rockspec::PerPlatform<rockspec::test::Spec> {
        &self.test
    }

    fn build_mut(&mut self) -> &mut rockspec::PerPlatform<rockspec::build::Spec> {
        &mut self.build
    }

    fn test_mut(&mut self) -> &mut rockspec::PerPlatform<rockspec::test::Spec> {
        &mut self.test
    }

    fn format(&self) -> &Option<rockspec::Format> {
        &self.rockspec_format
    }

    fn source(&self) -> &rockspec::PerPlatform<rockspec::source::Remote> {
        &self.source
    }

    fn source_mut(&mut self) -> &mut rockspec::PerPlatform<rockspec::source::Remote> {
        &mut self.source
    }

    fn deploy(&self) -> &rockspec::PerPlatform<rockspec::deploy::Spec> {
        &self.deploy
    }

    fn deploy_mut(&mut self) -> &mut rockspec::PerPlatform<rockspec::deploy::Spec> {
        &mut self.deploy
    }

    fn to_lua_remote_rockspec_string(&self) -> Result<String, rockspec::Error> {
        let lux_toml_dir = &self.internal.lux_toml_dir;
        let version = self
            .internal
            .version_template
            .try_generate(lux_toml_dir, None)
            .map_err(Error::from)?;
        let source = self
            .internal
            .source_template
            .try_generate(lux_toml_dir, &self.package, &version)
            .map_err(Error::from)?;
        Ok(self.render_rockspec(&version, Some(source.display_lua()))?)
    }
}

/// Creates a [`build::internal::Spec`] with auto-detected modules (e.g. *.luau sources)
/// that LuaRocks does not auto-detect, so that LuaRocks can install them.
fn build_spec_with_autodetected_modules(
    spec: &rockspec::build::internal::Spec,
    project_root: &Path,
) -> rockspec::build::internal::Spec {
    let builtin_capable = spec.build_type.as_ref().is_none_or(|build_type| {
        matches!(
            build_type,
            rockspec::build::internal::Type::Builtin | rockspec::build::internal::Type::None
        )
    });
    if !builtin_capable {
        return spec.clone();
    }
    let detected = match autodetect_modules(project_root, Default::default()) {
        Ok(modules) => modules
            .into_iter()
            .filter_map(|(module, spec)| match spec {
                rockspec::build::backend::ModuleSpec::SourcePath(path)
                    if path.extension().is_some_and(|ext| ext == "luau") =>
                {
                    Some((module, path))
                }
                _ => None,
            })
            .collect_vec(),
        Err(_) => return spec.clone(),
    };
    if detected.is_empty() {
        spec.clone()
    } else {
        let mut new_spec = spec.clone();
        let modules = new_spec.builtin_spec.get_or_insert_with(Default::default);
        for (module, path) in detected {
            modules
                .entry(rockspec::serde_util::LuaTableKey::StringKey(
                    module.to_string(),
                ))
                .or_insert(
                    rockspec::build::backend::builtin::internal::ModuleSpec::SourcePath(path),
                );
        }
        new_spec
    }
}

fn validate_generated_lua(lua_str: &str) -> Result<(), Error> {
    let mut lua = ottavino::Lua::core();
    lua.try_enter(|ctx| {
        let closure = ottavino::Closure::load(ctx, None, lua_str.as_bytes())?;
        let executor = ottavino::Executor::start(ctx, closure.into(), ());
        if !executor.step(ctx, &mut ottavino::Fuel::with(ROCKSPEC_FUEL_LIMIT))? {
            return Ok(Err(Error::FuelLimitExceeded));
        }
        Ok(Ok(()))
    })
    .map_err(|err| Error::GeneratedInvalidLua(err.to_string()))?
}

impl HasIntegrity for Local {
    async fn hash(&self) -> io::Result<Integrity> {
        match self.to_lua_rockspec() {
            Ok(lua_rockspec) => lua_rockspec.hash().await,
            Err(_) => self.internal.hash().await,
        }
    }
}

/// The `lux.toml`, after being validated and prepared for upload
#[derive(Debug)]
pub struct Remote {
    local: Local,
    source: rockspec::PerPlatform<rockspec::source::Remote>,
}

impl Remote {
    pub fn to_lua_rockspec(&self) -> Result<rockspec::lua::Remote, rockspec::Error> {
        Ok(rockspec::lua::Remote::new(
            &self.to_lua_remote_rockspec_string()?,
        )?)
    }
}

impl Rockspec for Remote {
    fn package(&self) -> &package::Name {
        self.local.package()
    }

    fn version(&self) -> &package::Version {
        self.local.version()
    }

    fn description(&self) -> &rockspec::description::Spec {
        self.local.description()
    }

    fn supported_platforms(&self) -> &rockspec::platform::Support {
        self.local.supported_platforms()
    }

    fn lua(&self) -> &package::VersionReq {
        self.local.lua()
    }

    fn dependencies(&self) -> &rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>> {
        self.local.dependencies()
    }

    fn build_dependencies(&self) -> &rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>> {
        self.local.build_dependencies()
    }

    fn external_dependencies(
        &self,
    ) -> &rockspec::PerPlatform<HashMap<String, rockspec::external_dependency::Spec>> {
        self.local.external_dependencies()
    }

    fn test_dependencies(&self) -> &rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>> {
        self.local.test_dependencies()
    }

    fn build(&self) -> &rockspec::PerPlatform<rockspec::build::Spec> {
        self.local.build()
    }

    fn test(&self) -> &rockspec::PerPlatform<rockspec::test::Spec> {
        self.local.test()
    }

    fn build_mut(&mut self) -> &mut rockspec::PerPlatform<rockspec::build::Spec> {
        self.local.build_mut()
    }

    fn test_mut(&mut self) -> &mut rockspec::PerPlatform<rockspec::test::Spec> {
        self.local.test_mut()
    }

    fn format(&self) -> &Option<rockspec::Format> {
        self.local.format()
    }

    fn source(&self) -> &rockspec::PerPlatform<rockspec::source::Remote> {
        &self.source
    }

    fn source_mut(&mut self) -> &mut rockspec::PerPlatform<rockspec::source::Remote> {
        &mut self.source
    }

    fn deploy(&self) -> &rockspec::PerPlatform<rockspec::deploy::Spec> {
        self.local.deploy()
    }

    fn deploy_mut(&mut self) -> &mut rockspec::PerPlatform<rockspec::deploy::Spec> {
        self.local.deploy_mut()
    }

    fn to_lua_remote_rockspec_string(&self) -> Result<String, rockspec::Error> {
        let lux_toml_dir = &self.local.internal.lux_toml_dir;
        let starter = format!(
            r#"
rockspec_format = "{}"
package = "{}"
version = "{}""#,
            self.local
                .rockspec_format
                .as_ref()
                .unwrap_or(&rockspec::Format::default()),
            self.local.package,
            self.version()
        );

        let mut template = Vec::new();

        if self.local.description != rockspec::description::Spec::default() {
            template.push(self.local.description.display_lua());
        }

        if self.local.supported_platforms != rockspec::platform::Support::default() {
            template.push(self.local.supported_platforms.display_lua());
        }

        {
            let mut dependencies = self.local.internal.dependencies.clone().unwrap_or_default();
            if !self.local.lua.is_luau() {
                dependencies.insert(
                    0,
                    package::Req {
                        name: "lua".into(),
                        version_req: self.local.lua.clone(),
                    }
                    .into(),
                );
            }
            template.push(Dependencies(&dependencies).display_lua());
        }

        let mut build_dependencies = self
            .local
            .internal
            .build_dependencies
            .as_ref()
            .cloned()
            .unwrap_or_default();

        let build_backend_dependency = self
            .local
            .internal
            .build
            .build_type
            .as_ref()
            .and_then(|build_type| build_type.luarocks_build_backend());

        if let Some(build_backend_dependency) = build_backend_dependency {
            build_dependencies.push(build_backend_dependency);
        }

        if !build_dependencies.is_empty() {
            template.push(BuildDependencies(&build_dependencies).display_lua());
        }

        match self.local.internal.external_dependencies {
            Some(ref external_dependencies) if !external_dependencies.is_empty() => {
                template.push(
                    rockspec::external_dependency::Dependencies(external_dependencies)
                        .display_lua(),
                );
            }
            _ => {}
        }

        let source = self
            .local
            .internal
            .source_template
            .try_generate(lux_toml_dir, &self.local.internal.package, self.version())
            .map_err(Error::from)?;
        template.push(source.display_lua());

        if let Some(ref deploy) = self.local.internal.deploy {
            template.push(deploy.display_lua());
        }

        template.push(
            build_spec_with_autodetected_modules(&self.local.internal.build, lux_toml_dir)
                .display_lua(),
        );

        let unformatted_code = std::iter::once(starter)
            .chain(template.into_iter().map(|kv| kv.to_string()))
            .join("\n\n");
        let result = match stylua_lib::format_code(
            &unformatted_code,
            stylua_lib::Config::default(),
            None,
            stylua_lib::OutputVerification::Full,
        ) {
            Ok(formatted_code) => formatted_code,
            Err(_) => unformatted_code,
        };
        validate_generated_lua(&result)?;
        Ok(result)
    }
}

impl HasIntegrity for Remote {
    async fn hash(&self) -> io::Result<Integrity> {
        self.to_lua_rockspec()
            .map_err(|err| {
                io::Error::other(format!(
                    "unable to convert remote project to Lua rockspec:\n{}",
                    err
                ))
            })?
            .hash()
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::rockspec;

    use assert_fs::prelude::{PathChild, PathCopy, PathCreateDir};
    use git2::Repository;
    use url::Url;

    use crate::{
        git::{url::RemoteGitUrl, GitRef, GitSource},
        project::{Project, TOML_FILE_NAME},
        rockspec::Rockspec,
    };

    use super::Partial;

    #[test]
    fn project_toml_parsing() {
        let project_toml = r#"
        package = "my-package"
        version = "1.0.0"
        lua = "5.3"

        rockspec_format = "1.0"

        [source]
        url = "https://example.com"

        [dependencies]
        foo = "1.0"
        bar = ">=2.0"

        [run]
        args = ["--foo", "--bar"]

        [build]
        type = "builtin"
        "#;

        let project = Partial::new(TOML_FILE_NAME, project_toml, PathBuf::default()).unwrap();
        let _ = project.into_remote(None).unwrap();

        let project_toml = r#"
        package = "my-package"
        version = "1.0.0"
        lua = "5.1"

        [description]
        summary = "A summary"
        detailed = "A detailed description"
        license = "MIT"
        homepage = "https://example.com"
        issues_url = "https://example.com/issues"
        maintainer = "John Doe"
        labels = ["label1", "label2"]

        [supported_platforms]
        linux = true
        windows = false

        [dependencies]
        foo = "1.0"
        bar = ">=2.0"

        [build_dependencies]
        baz = "1.0"

        [external_dependencies.foo]
        header = "foo.h"

        [external_dependencies.bar]
        library = "libbar.so"

        [test_dependencies]
        busted = "69.420"

        [source]
        url = "https://example.com"
        hash = "sha256-di00mD8txN7rjaVpvxzNbnQsAh6H16zUtJZapH7U4HU="
        file = "my-package-1.0.0.tar.gz"
        dir = "my-package-1.0.0"

        [test]
        type = "command"
        script = "test.lua"
        flags = [ "foo", "bar" ]

        [run]
        command = "my-command"
        args = ["--foo", "--bar"]

        [build]
        type = "builtin"
        "#;

        let project = Partial::new(TOML_FILE_NAME, project_toml, PathBuf::default()).unwrap();
        let _ = project.into_remote(None).unwrap();
    }

    #[test]
    fn compare_project_toml_with_rockspec() {
        let project_toml = r#"
        package = "my-package"
        version = "1.0.0"
        lua = "5.1"

        # For testing, specify a custom rockspec format
        # (defaults to 3.0)
        rockspec_format = "1.0"

        [description]
        summary = "A summary"
        detailed = "A detailed description"
        license = "MIT"
        homepage = "https://example.com"
        issues_url = "https://example.com/issues"
        maintainer = "John Doe"
        labels = ["label1", "label2"]

        [supported_platforms]
        linux = true
        windows = false

        [dependencies]
        foo = "1.0"
        bar = ">=2.0"

        [build_dependencies]
        baz = "1.0"

        [external_dependencies.foo]
        header = "foo.h"

        [external_dependencies.bar]
        library = "libbar.so"

        [test_dependencies]
        busted = "1.0"

        [source]
        url = "https://example.com"
        file = "my-package-1.0.0.tar.gz"
        dir = "my-package-1.0.0"

        [test]
        type = "command"
        script = "test.lua"
        flags = [ "foo", "bar" ]

        [run]
        command = "my-command"
        args = ["--foo", "--bar"]

        [deploy]
        wrap_bin_scripts = false

        [build]
        type = "builtin"

        [build.install.lua]
        "foo.bar" = "src/bar.lua"

        [build.install.lib]
        "foo.baz" = "src/baz.c"

        [build.install.bin]
        "bla" = "src/bla"

        [build.install.conf]
        "cfg.conf" = "resources/config.conf"
        "#;

        let expected_rockspec = r#"
            rockspec_format = "1.0"
            package = "my-package"
            version = "1.0.0"

            source = {
                url = "https://example.com",
                file = "my-package-1.0.0.tar.gz",
                dir = "my-package-1.0.0",
            }

            description = {
                summary = "A summary",
                detailed = "A detailed description",
                license = "MIT",
                homepage = "https://example.com",
                issues_url = "https://example.com/issues",
                maintainer = "John Doe",
                labels = {"label1", "label2"},
            }

            supported_platforms = {"linux", "!windows"}

            dependencies = {
                "lua ==5.1",
                "foo ==1.0",
                "bar >=2.0",
            }

            build_dependencies = {
                "baz ==1.0",
            }

            external_dependencies = {
                foo = { header = "foo.h" },
                bar = { library = "libbar.so" },
            }

            source = {
                url = "https://example.com",
                hash = "sha256-di00mD8txN7rjaVpvxzNbnQsAh6H16zUtJZapH7U4HU=",
                file = "my-package-1.0.0.tar.gz",
                dir = "my-package-1.0.0",
            }

            test = {
                type = "command",
                script = "test.lua",
                flags = {"foo", "bar"},
            }

            deploy = {
                wrap_bin_scripts = false,
            }

            build = {
                type = "builtin",
                install = {
                    lua = {
                        ["foo.bar"] = "src/bar.lua",
                    },
                    lib = {
                        ["foo.baz"] = "src/baz.c",
                    },
                    bin = {
                        bla = "src/bla",
                    },
                    conf = {
                        ["cfg.conf"] = "resources/config.conf",
                    },
                },
            }
        "#;

        let expected_rockspec = rockspec::lua::Remote::new(expected_rockspec).unwrap();

        let project_toml = Partial::new(TOML_FILE_NAME, project_toml, PathBuf::default()).unwrap();
        let rockspec = project_toml
            .into_remote(None)
            .unwrap()
            .to_lua_rockspec()
            .unwrap();

        let sorted_package_reqs =
            |v: &rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>>| {
                let mut v = v.current_platform().clone();
                v.sort_by(|a, b| a.name().cmp(b.name()));
                v
            };

        assert_eq!(rockspec.package(), expected_rockspec.package());
        assert_eq!(rockspec.version(), expected_rockspec.version());
        assert_eq!(rockspec.description(), expected_rockspec.description());
        assert_eq!(
            rockspec.supported_platforms(),
            expected_rockspec.supported_platforms()
        );
        assert_eq!(
            sorted_package_reqs(rockspec.dependencies()),
            sorted_package_reqs(expected_rockspec.dependencies())
        );
        assert_eq!(
            sorted_package_reqs(rockspec.build_dependencies()),
            sorted_package_reqs(expected_rockspec.build_dependencies())
        );
        assert_eq!(
            rockspec.external_dependencies(),
            expected_rockspec.external_dependencies()
        );
        assert_eq!(rockspec.source(), expected_rockspec.source());
        assert_eq!(rockspec.build(), expected_rockspec.build());
        assert_eq!(rockspec.format(), expected_rockspec.format());
    }

    #[test]
    fn merge_project_toml_with_partial_rockspec() {
        let project_toml = r#"
        package = "my-package"
        version = "1.0.0"
        lua = "5.1"

        # For testing, specify a custom rockspec format
        # (defaults to 3.0)
        rockspec_format = "1.0"

        [description]
        summary = "A summary"
        detailed = "A detailed description"
        license = "MIT"
        homepage = "https://example.com"
        issues_url = "https://example.com/issues"
        maintainer = "John Doe"
        labels = ["label1", "label2"]

        [supported_platforms]
        linux = true
        windows = false

        [dependencies]
        foo = "1.0"
        bar = ">=2.0"

        [build_dependencies]
        baz = "1.0"

        [external_dependencies.foo]
        header = "foo.h"

        [external_dependencies.bar]
        library = "libbar.so"

        [test_dependencies]
        busted = "1.0"

        [source]
        url = "https://example.com"
        file = "my-package-1.0.0.tar.gz"
        dir = "my-package-1.0.0"

        [test]
        type = "command"
        script = "test.lua"
        flags = [ "foo", "bar" ]

        [run]
        command = "my-command"
        args = [ "--foo", "--bar" ]

        [build]
        type = "builtin"
        "#;

        let mergable_rockspec_content = r#"
            rockspec_format = "1.0"
            package = "my-package-overwritten"

            description = {
                summary = "A summary overwritten",
                detailed = "A detailed description overwritten",
                license = "GPL-2.0",
                homepage = "https://example.com/overwritten",
                issues_url = "https://example.com/issues/overwritten",
                maintainer = "John Doe Overwritten",
                labels = {"over", "written"},
            }

            -- Inverted supported platforms
            supported_platforms = {"!linux", "windows"}

            dependencies = {
                "lua 5.1",
                "foo >1.0",
                "bar <=2.0",
            }

            build_dependencies = {
                "baz >1.0",
            }

            external_dependencies = {
                foo = { header = "overwritten.h" },
                bar = { library = "overwritten.so" },
            }

            test = {
                type = "command",
                script = "overwritten.lua",
                flags = {"over", "written"},
            }

            build = {
                type = "builtin",
            }
        "#;

        let remote_rockspec_content = format!(
            r#"{}
            version = "1.0.0"
            source = {{
                url = "https://example.com",
                file = "my-package-1.0.0.tar.gz",
                dir = "my-package-1.0.0",
            }}
        "#,
            mergable_rockspec_content
        );

        let project_toml = Partial::new(TOML_FILE_NAME, project_toml, PathBuf::default()).unwrap();
        let partial_rockspec = rockspec::lua::Partial::new(mergable_rockspec_content).unwrap();
        let expected_rockspec = rockspec::lua::Remote::new(&remote_rockspec_content).unwrap();

        let merged = project_toml
            .merge(partial_rockspec)
            .into_remote(None)
            .unwrap();

        let sorted_package_reqs =
            |v: &rockspec::PerPlatform<Vec<rockspec::lua_dependency::Spec>>| {
                let mut v = v.current_platform().clone();
                v.sort_by(|a, b| a.name().cmp(b.name()));
                v
            };

        assert_eq!(merged.package(), expected_rockspec.package());
        assert_eq!(merged.version(), expected_rockspec.version());
        assert_eq!(merged.description(), expected_rockspec.description());
        assert_eq!(
            merged.supported_platforms(),
            expected_rockspec.supported_platforms()
        );
        assert_eq!(
            sorted_package_reqs(merged.dependencies()),
            sorted_package_reqs(expected_rockspec.dependencies())
        );
        assert_eq!(
            sorted_package_reqs(merged.build_dependencies()),
            sorted_package_reqs(expected_rockspec.build_dependencies())
        );
        assert_eq!(
            merged.external_dependencies(),
            expected_rockspec.external_dependencies()
        );
        assert_eq!(merged.source(), expected_rockspec.source());
        assert_eq!(merged.build(), expected_rockspec.build());
        assert_eq!(merged.format(), expected_rockspec.format());
        // Ensure that the run command is retained after merge.
        assert!(merged.local.run().is_some());
    }

    #[test]
    fn wally_dependency_parsing() {
        let project_toml = r#"
        package = "my-package"
        version = "1.0.0"
        lua = "5.1"

        [dependencies.roact]
        wally = "jsdotperf/roact@^1.4.0"

        [build]
        type = "builtin"
        "#;

        let project = Partial::new(TOML_FILE_NAME, project_toml, PathBuf::default()).unwrap();
        let deps = project.dependencies.unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name().to_string(), "roact");
        assert!(matches!(
            deps[0].source(),
            Some(rockspec::source::Spec::Wally(req)) if req.name().to_string() == "jsdotperf/roact"
        ));
    }

    #[test]
    fn wally_dependency_conflicts_with_version() {
        let project_toml = r#"
        package = "my-package"
        version = "1.0.0"
        lua = "5.1"

        [dependencies.roact]
        wally = "jsdotperf/roact@^1.4.0"
        version = "1.4.0"

        [build]
        type = "builtin"
        "#;

        Partial::new(TOML_FILE_NAME, project_toml, PathBuf::default()).unwrap_err();
    }

    #[test]
    fn project_toml_with_lua_in_dependencies() {
        let project_toml = r#"
        package = "my-package"
        version = "1.0.0"
        # lua = ">5.1"

        [dependencies]
        lua = "5.1" # disallowed

        [build]
        type = "builtin"
        "#;

        Partial::new(TOML_FILE_NAME, project_toml, PathBuf::default())
            .unwrap()
            .into_local()
            .unwrap_err();
    }

    #[test]
    fn project_toml_with_invalid_run_command() {
        for command in ["lua", "lua5.1", "lua5.2", "lua5.3", "lua5.4", "luajit"] {
            let project_toml = format!(
                r#"
                package = "my-package"
                version = "1.0.0"
                lua = "5.1"

                [build]
                type = "builtin"

                [run]
                command = "{command}"
                "#,
            );

            Partial::new(TOML_FILE_NAME, &project_toml, PathBuf::default()).unwrap_err();
        }
    }

    #[test]
    fn project_root_dir_override_and_rockspec_generation() {
        let project_toml = r#"
        package = "my-package"
        version = "1.0.0"
        lua = "5.1"

        [project]
        root_dir = ".."

        [source]
        url = "https://example.com"

        [build]
        type = "builtin"
        "#;

        let workspace_root = assert_fs::TempDir::new().unwrap();
        let lux_toml_dir = workspace_root.child("proj");
        lux_toml_dir.create_dir_all().unwrap();
        std::fs::write(lux_toml_dir.join(TOML_FILE_NAME), project_toml).unwrap();

        let project =
            Partial::new(TOML_FILE_NAME, project_toml, lux_toml_dir.to_path_buf()).unwrap();
        let local = project.into_local().unwrap();
        let rockspec::source::Spec::File(root_dir) = &local.source().current_platform().source_spec
        else {
            panic!("expected a file source");
        };
        assert_eq!(
            root_dir,
            &std::fs::canonicalize(workspace_root.path()).unwrap()
        );

        // The `[project]` table must be ignored when generating a rockspec.
        let rockspec = local.to_lua_rockspec().unwrap();
        assert!(!rockspec
            .to_lua_remote_rockspec_string()
            .unwrap()
            .contains("root_dir"));
    }

    #[test]
    fn generate_non_deterministic_git_source() {
        let rockspec_content = r#"
            package = "test-package"
            version = "1.0.0"
            lua = ">=5.1"

            [source]
            url = "git+https://exaple.com/repo.git"

            [build]
            type = "builtin"
        "#;

        Partial::new(TOML_FILE_NAME, rockspec_content, PathBuf::default())
            .unwrap()
            .into_remote(None)
            .unwrap_err();
    }

    #[test]
    fn generate_deterministic_git_source() {
        let rockspec_content = r#"
            package = "test-package"
            version = "1.0.0"
            lua = ">=5.1"

            [source]
            url = "git+https://exaple.com/owner/repo.git"
            tag = "v0.1.0"

            [build]
            type = "builtin"
        "#;

        Partial::new(TOML_FILE_NAME, rockspec_content, PathBuf::default())
            .unwrap()
            .into_remote(None)
            .unwrap();
    }

    #[test]
    fn generate_git_source_with_branch() {
        let rockspec_content = r#"
            package = "test-package"
            version = "1.0.0"
            lua = ">=5.1"

            [source]
            url = "git+https://example.com/owner/repo.git"
            branch = "main"

            [build]
            type = "builtin"
        "#;

        let toml = Partial::new(TOML_FILE_NAME, rockspec_content, PathBuf::default())
            .unwrap()
            .into_remote(None)
            .unwrap();

        let lua = toml.to_lua_remote_rockspec_string().unwrap();
        assert!(lua.contains("branch = \"main\""));
        assert!(!lua.contains("tag = "));
    }

    fn init_git_repo(path: &std::path::Path) -> Repository {
        let repo = Repository::init(path).unwrap();
        {
            let tree_id = repo.index().unwrap().write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            let sig = git2::Signature::now("name", "email").unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
                .unwrap();
        }
        repo
    }

    fn init_sample_project_repo(temp_dir: &assert_fs::TempDir) -> Repository {
        let sample_project = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample-projects/source-template/");
        temp_dir.copy_from(&sample_project, &["**"]).unwrap();
        init_git_repo(temp_dir.path())
    }

    fn create_tag(repo: &Repository, name: &str) {
        let sig = git2::Signature::now("name", "email").unwrap();
        let id = repo.head().unwrap().target().unwrap();
        let obj = repo.find_object(id, None).unwrap();
        repo.tag(name, &obj, &sig, "msg", true).unwrap();
    }

    #[test]
    fn test_git_project_generate_dev_source() {
        let project_root = assert_fs::TempDir::new().unwrap();
        init_sample_project_repo(&project_root);
        let project = Project::from_exact(&project_root).unwrap().unwrap();
        let remote_project_toml = project.toml().into_remote(None).unwrap();
        let source = remote_project_toml.source.current_platform();
        let source_spec = &source.source_spec;
        assert!(matches!(source_spec, &rockspec::source::Spec::Git { .. }));
        if let rockspec::source::Spec::Git(GitSource { url, git_ref }) = source_spec {
            let expected_url: RemoteGitUrl =
                "https://github.com/lumen-oss/lux.git".parse().unwrap();
            assert_eq!(url, &expected_url);
            assert!(git_ref.is_some());
        }
        assert_eq!(source.unpack_dir, Some("lux-dev".into()));
    }

    #[test]
    fn test_git_project_generate_non_semver_tag_source() {
        let project_root = assert_fs::TempDir::new().unwrap();
        let repo = init_sample_project_repo(&project_root);
        let tag_name = "bla";
        create_tag(&repo, tag_name);
        let project = Project::from_exact(&project_root).unwrap().unwrap();
        let remote_project_toml = project.toml().into_remote(None).unwrap();
        let source = remote_project_toml.source.current_platform();
        let source_spec = &source.source_spec;
        assert!(matches!(source_spec, &rockspec::source::Spec::Git { .. }));
        if let rockspec::source::Spec::Git(GitSource { url, git_ref }) = source_spec {
            let expected_url: RemoteGitUrl =
                "https://github.com/lumen-oss/lux.git".parse().unwrap();
            assert_eq!(url, &expected_url);
            assert_eq!(git_ref, &Some(GitRef::Tag(tag_name.to_string())));
        }
        assert_eq!(source.unpack_dir, Some("lux-dev".into()));
    }

    #[test]
    fn test_git_project_generate_release_source_tag_with_v_prefix() {
        let project_root = assert_fs::TempDir::new().unwrap();
        let repo = init_sample_project_repo(&project_root);
        let tag_name = "v1.0.0";
        create_tag(&repo, "bla");
        create_tag(&repo, tag_name);
        let project = Project::from_exact(&project_root).unwrap().unwrap();
        let remote_project_toml = project.toml().into_remote(None).unwrap();
        let source = remote_project_toml.source.current_platform();
        let source_spec = &source.source_spec;
        assert!(matches!(source_spec, &rockspec::source::Spec::Url { .. }));
        if let rockspec::source::Spec::Url(url) = source_spec {
            let expected_url: Url = "https://github.com/lumen-oss/lux/archive/refs/tags/v1.0.0.zip"
                .parse()
                .unwrap();
            assert_eq!(url, &expected_url);
        }
        assert_eq!(source.unpack_dir, Some("lux-1.0.0".into()));
    }

    #[test]
    fn test_git_project_generate_release_source_tag_without_v_prefix() {
        let project_root = assert_fs::TempDir::new().unwrap();
        let repo = init_sample_project_repo(&project_root);
        create_tag(&repo, "bla");
        let tag_name = "1.0.0";
        create_tag(&repo, tag_name);
        let project = Project::from_exact(&project_root).unwrap().unwrap();
        let remote_project_toml = project.toml().into_remote(None).unwrap();
        let source = remote_project_toml.source.current_platform();
        let source_spec = &source.source_spec;
        assert!(matches!(source_spec, &rockspec::source::Spec::Url { .. }));
        if let rockspec::source::Spec::Url(url) = source_spec {
            let expected_url: Url = "https://github.com/lumen-oss/lux/archive/refs/tags/1.0.0.zip"
                .parse()
                .unwrap();
            assert_eq!(url, &expected_url);
        }
        assert_eq!(source.unpack_dir, Some("lux-1.0.0".into()));
    }

    #[test]
    fn test_git_project_in_subdirectory() {
        let temp_dir = assert_fs::TempDir::new().unwrap();
        let sample_project = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample-projects/source-template/");
        let project_dir = temp_dir.child("lux");
        project_dir.create_dir_all().unwrap();
        project_dir.copy_from(&sample_project, &["**"]).unwrap();
        let repo = init_git_repo(temp_dir.path());
        create_tag(&repo, "bla");
        let tag_name = "1.0.0";
        create_tag(&repo, tag_name);
        let project = Project::from_exact(&project_dir).unwrap().unwrap();
        let remote_project_toml = project.toml().into_remote(None).unwrap();
        let source = remote_project_toml.source.current_platform();
        let source_spec = &source.source_spec;
        assert!(matches!(source_spec, &rockspec::source::Spec::Url { .. }));
        if let rockspec::source::Spec::Url(url) = source_spec {
            let expected_url: Url = "https://github.com/lumen-oss/lux/archive/refs/tags/1.0.0.zip"
                .parse()
                .unwrap();
            assert_eq!(url, &expected_url);
        }
        assert_eq!(source.unpack_dir, Some("lux-1.0.0".into()));
    }

    #[test]
    fn test_git_project_in_subdirectory_generate_dev_source() {
        let temp_dir = assert_fs::TempDir::new().unwrap();
        let sample_project = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample-projects/source-template/");
        let project_dir = temp_dir.child("lux");
        project_dir.create_dir_all().unwrap();
        project_dir.copy_from(&sample_project, &["**"]).unwrap();
        init_git_repo(temp_dir.path());

        let project = Project::from_exact(&project_dir).unwrap().unwrap();
        let remote_project_toml = project.toml().into_remote(None).unwrap();
        let rockspec::source::Spec::Git(GitSource { git_ref, .. }) =
            &remote_project_toml.source.current_platform().source_spec
        else {
            panic!("expected a git source");
        };
        assert!(git_ref.is_some());
    }
}
