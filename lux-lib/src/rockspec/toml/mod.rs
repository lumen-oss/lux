//! Structs and utilities for `lux.toml`

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use itertools::Itertools;
use miette::Diagnostic;

use crate::build::autodetect_modules;
use crate::git;
use crate::lockfile::OptState;
use crate::lockfile::PinnedState;
use crate::package;
use crate::rockspec::{self, lua_dependency};
use crate::wally;
use crate::ROCKSPEC_FUEL_LIMIT;
use serde::de;
use serde::{Deserialize, Deserializer};
use thiserror::Error;

mod gen;
mod local;
mod partial;
mod remote;
mod validate;

pub use self::gen::{GenerateSourceError, GenerateVersionError};
pub use local::Local;
pub use partial::{Partial, RunSpec};
pub use remote::Remote;

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
    git: Option<git::remote::Shorthand>,
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
                                    Ok(Some(rockspec::source::Spec::Git(git::Source {
                                        url: git.into(),
                                        git_ref: Some(git::Ref::Tag(rev)),
                                    })))
                                }
                                (Some(git), None, None) => Ok(Some(rockspec::source::Spec::Git(git::Source {
                                    url: git.into(),
                                    git_ref: Some(git::Ref::Tag(
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
        Ok((modules, _)) => modules
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::rockspec;

    use assert_fs::prelude::{PathChild, PathCopy, PathCreateDir};
    use git2::Repository;
    use url::Url;

    use crate::{
        git::{self},
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
    fn generated_rockspec_format_follows_teal_build_dependency() {
        let rockspec = |build_dependencies: &str| {
            let content = format!(
                r#"
package = "test-package"
version = "1.0.0"
lua = ">=5.1"

[source]
url = "git+https://example.com/owner/repo.git"
tag = "v0.1.0"

[build]
type = "builtin"
{build_dependencies}
"#
            );
            Partial::new(TOML_FILE_NAME, &content, PathBuf::default())
                .unwrap()
                .into_remote(None)
                .unwrap()
                .to_lua_remote_rockspec_string()
                .unwrap()
        };
        assert!(rockspec("").contains("rockspec_format = \"3.0\""));
        assert!(
            rockspec("[build_dependencies]\ntl = \">=0.24\"").contains("rockspec_format = \"3.2\"")
        );
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
        if let rockspec::source::Spec::Git(git::Source { url, git_ref }) = source_spec {
            let expected_url: git::remote::Url =
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
        if let rockspec::source::Spec::Git(git::Source { url, git_ref }) = source_spec {
            let expected_url: git::remote::Url =
                "https://github.com/lumen-oss/lux.git".parse().unwrap();
            assert_eq!(url, &expected_url);
            assert_eq!(git_ref, &Some(git::Ref::Tag(tag_name.to_string())));
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
        let rockspec::source::Spec::Git(git::Source { git_ref, .. }) =
            &remote_project_toml.source.current_platform().source_spec
        else {
            panic!("expected a git source");
        };
        assert!(git_ref.is_some());
    }
}
