use super::platform;
use crate::package;
use itertools::Itertools;

use miette::Diagnostic;
use path_slash::PathExt;
use serde::{Deserialize, Deserializer};
use serde_enum_str::Serialize_enum_str;
use std::{convert::Infallible, path::PathBuf};
use thiserror::Error;

use crate::lua;

use crate::{
    config::{self, Config},
    project::Project,
    rockspec::platform::per_platform_from_intermediate,
    rockspec::Rockspec,
};

use super::PerPlatform;

#[cfg(target_family = "unix")]
const NLUA_EXE: &str = "nlua";
#[cfg(target_family = "windows")]
const NLUA_EXE: &str = "nlua.bat";

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum DecodeError {
    #[error("the 'command' test type must specify either a 'command' or 'script' field")]
    NoCommandOrScript,
    #[error("the 'command' test type cannot have both 'command' and 'script' fields")]
    CommandAndScript,
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("could not auto-detect the test spec. Please add one to your lux.toml")]
    NoTestSpecDetected,
    #[error("project validation failed")]
    #[diagnostic(forward(0))]
    LocalProjectTomlValidation(#[from] super::toml::Error),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Spec {
    #[default]
    AutoDetect,
    Busted(Busted),
    BustedNlua(Busted),
    Tiniest,
    Command(Command),
    Script(LuaScript),
}

impl Spec {
    pub(crate) fn test_dependencies(&self, project: &Project) -> Vec<package::Req> {
        self.to_validated(project)
            .ok()
            .iter()
            .flat_map(|spec| spec.test_dependencies())
            .collect_vec()
    }

    pub(crate) fn to_validated(&self, project: &Project) -> Result<internal::Validated, Error> {
        let project_root = project.root();
        let toml = project.toml().into_local()?;
        let test_dependencies = toml.test_dependencies().current_platform();
        let busted_dependency = test_dependencies
            .iter()
            .find(|dep| dep.name().to_string() == "busted");
        let busted = busted_dependency
            .map(|dep| dep.package_req.clone())
            .unwrap_or_else(|| unsafe { package::Req::new_unchecked("busted".into(), None) });
        let nlua_dependency = test_dependencies
            .iter()
            .find(|dep| dep.name().to_string() == "nlua");
        let nlua = nlua_dependency
            .map(|dep| dep.package_req.clone())
            .unwrap_or_else(|| unsafe { package::Req::new_unchecked("nlua".into(), None) });
        let is_busted = project_root.join(".busted").is_file() || busted_dependency.is_some();
        match self {
            Self::AutoDetect if is_busted => {
                if nlua_dependency.is_some() {
                    Ok(internal::Validated::BustedNlua {
                        spec: Busted::default(),
                        dependencies: vec![busted, nlua],
                    })
                } else {
                    Ok(internal::Validated::Busted {
                        spec: Busted::default(),
                        dependencies: vec![busted],
                    })
                }
            }
            Self::Busted(spec) => Ok(internal::Validated::Busted {
                spec: spec.clone(),
                dependencies: vec![busted],
            }),
            Self::BustedNlua(spec) => Ok(internal::Validated::BustedNlua {
                spec: spec.clone(),
                dependencies: vec![busted, nlua],
            }),
            Self::Tiniest => Ok(internal::Validated::Tiniest),
            Self::Command(spec) => Ok(internal::Validated::Command(spec.clone())),
            Self::Script(spec) => Ok(internal::Validated::LuaScript(spec.clone())),
            Self::AutoDetect => Err(Error::NoTestSpecDetected),
        }
    }
}

/// Specification for running a test suite with busted
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Busted {
    /// Additional CLI flags to pass to Busted when running
    pub(crate) flags: Vec<String>,
}

impl Busted {
    pub fn flags(&self) -> &Vec<String> {
        &self.flags
    }
}

/// Specification for running a test suite with a command
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    /// The command to run
    pub(crate) command: String,
    /// Additional CLI flags to pass to Busted when running
    pub(crate) flags: Vec<String>,
}

impl Command {
    pub fn flags(&self) -> &Vec<String> {
        &self.flags
    }

    pub fn command(&self) -> &str {
        &self.command
    }
}

/// Specification for running a test suite with a Lua script
#[derive(Clone, Debug, PartialEq)]
pub struct LuaScript {
    /// The script to run
    pub(crate) script: PathBuf,
    /// Additional CLI flags to pass to the script when running
    pub(crate) flags: Vec<String>,
}

impl LuaScript {
    pub fn flags(&self) -> &Vec<String> {
        &self.flags
    }

    pub fn script(&self) -> &PathBuf {
        &self.script
    }
}

pub(crate) mod internal {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    pub(crate) enum Validated {
        Busted {
            spec: Busted,
            dependencies: Vec<package::Req>,
        },
        BustedNlua {
            spec: Busted,
            dependencies: Vec<package::Req>,
        },
        Tiniest,
        Command(Command),
        LuaScript(LuaScript),
    }
    impl Validated {
        pub fn args(&self) -> Vec<String> {
            match self {
                Self::Busted {
                    spec,
                    dependencies: _,
                } => spec.flags.clone(),
                Self::BustedNlua {
                    spec,
                    dependencies: _,
                } => {
                    let mut flags = spec.flags.clone();
                    // If there's a .busted config file which has lua set to "nlua",
                    // we tell busted to ignore it, because we set the correct
                    // platform-dependent "nlua" executable in the wrapper script.
                    flags.push("--ignore-lua".into());
                    flags
                }
                Self::Tiniest => Vec::new(),
                Self::Command(spec) => spec.flags.clone(),
                Self::LuaScript(spec) => std::iter::once(spec.script.to_slash_lossy().to_string())
                    .chain(spec.flags.clone())
                    .collect_vec(),
            }
        }

        pub(crate) fn test_config(&self, config: &Config) -> Result<Config, config::Error> {
            match self {
                Self::BustedNlua { .. } => {
                    let config_builder: config::Builder = config.clone().into();

                    // XXX: On macos and msvc, Neovim and LuaJIT segfault when
                    // requiring luafilesystem's `lfs` module
                    // if it is built with Lua 5.1 headers.
                    #[cfg(not(any(target_os = "macos", target_env = "msvc")))]
                    let lua_version = lua::Version::Lua51;

                    #[cfg(any(target_os = "macos", target_env = "msvc"))]
                    let lua_version = lua::Version::LuaJIT;

                    Ok(config_builder
                        .lua_version(Some(lua_version))
                        .variables(Some(
                            vec![("LUA".to_string(), NLUA_EXE.to_string())]
                                .into_iter()
                                .collect(),
                        ))
                        .build()?)
                }
                _ => Ok(config.clone()),
            }
        }

        pub(crate) fn test_dependencies(&self) -> Vec<package::Req> {
            match self {
                Self::Busted {
                    spec: _,
                    dependencies,
                } => dependencies.clone(),
                Self::BustedNlua {
                    spec: _,
                    dependencies,
                } => dependencies.clone(),
                Self::Tiniest => Vec::new(),
                Self::Command(_) => Vec::new(),
                Self::LuaScript(_) => Vec::new(),
            }
        }
    }
    #[derive(Debug, Deserialize, Serialize_enum_str, PartialEq, Clone)]
    #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
    #[serde(rename_all = "kebab-case")]
    pub(crate) enum Type {
        Busted,
        BustedNlua,
        Tiniest,
        Command,
    }

    #[derive(Debug, PartialEq, Deserialize, Default, Clone)]
    #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
    pub(crate) struct Spec {
        /// The test backend: "busted", "busted-nlua", "tiniest", or "command".
        /// If unset, Lux auto-detects a backend (e.g. "busted" when a ".busted"
        /// file or the "busted" dependency is present).
        #[serde(default, rename = "type")]
        pub(crate) test_type: Option<Type>,
        /// Extra CLI flags to pass to the test backend.
        #[serde(default)]
        pub(crate) flags: Option<Vec<String>>,
        /// Shell command to run as the test suite. Mutually exclusive with `script`.
        #[serde(default)]
        #[cfg_attr(
            feature = "schema",
            schemars(extend("x-lux-toml-example" = r#"
    [test]
    type = "command"
    command = "make"
    flags = ["test"]
    "#))
        )]
        pub(crate) command: Option<String>,
        /// Lua script to run as the test suite. Mutually exclusive with `command`.
        #[serde(default, rename = "script", alias = "lua_script")]
        #[cfg_attr(
            feature = "schema",
            schemars(extend("x-lux-toml-example" = r#"
    [test]
    type = "script"
    script = "tests.lua"
    flags = []
    "#))
        )]
        pub(crate) script: Option<PathBuf>,
    }

    impl platform::PartialOverride for Spec {
        type Err = Infallible;

        fn apply_overrides(&self, override_spec: &Self) -> Result<Self, Self::Err> {
            Ok(Spec {
                test_type: override_opt(&override_spec.test_type, &self.test_type),
                flags: match (override_spec.flags.clone(), self.flags.clone()) {
                    (Some(override_vec), Some(base_vec)) => {
                        let merged: Vec<String> =
                            base_vec.into_iter().chain(override_vec).unique().collect();
                        Some(merged)
                    }
                    (None, base_vec @ Some(_)) => base_vec,
                    (override_vec @ Some(_), None) => override_vec,
                    _ => None,
                },
                command: match override_spec.script.clone() {
                    Some(_) => None,
                    None => override_opt(&override_spec.command, &self.command),
                },
                script: match override_spec.command.clone() {
                    Some(_) => None,
                    None => override_opt(&override_spec.script, &self.script),
                },
            })
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

    fn override_opt<T: Clone>(override_opt: &Option<T>, base: &Option<T>) -> Option<T> {
        match override_opt.clone() {
            override_val @ Some(_) => override_val,
            None => base.clone(),
        }
    }
    impl TryFrom<Spec> for super::Spec {
        type Error = DecodeError;

        fn try_from(internal: Spec) -> Result<Self, Self::Error> {
            let test_spec = match internal.test_type {
                Some(Type::Busted) => Ok(Self::Busted(Busted {
                    flags: internal.flags.unwrap_or_default(),
                })),
                Some(Type::BustedNlua) => Ok(Self::BustedNlua(Busted {
                    flags: internal.flags.unwrap_or_default(),
                })),
                Some(Type::Tiniest) => Ok(Self::Tiniest),
                Some(Type::Command) => match (internal.command, internal.script) {
                    (None, None) => Err(DecodeError::NoCommandOrScript),
                    (None, Some(script)) => Ok(Self::Script(LuaScript {
                        script,
                        flags: internal.flags.unwrap_or_default(),
                    })),
                    (Some(command), None) => Ok(Self::Command(Command {
                        command,
                        flags: internal.flags.unwrap_or_default(),
                    })),
                    (Some(_), Some(_)) => Err(DecodeError::CommandAndScript),
                },
                None => Ok(Self::default()),
            }?;
            Ok(test_spec)
        }
    }
    impl<'de> Deserialize<'de> for PerPlatform<super::Spec> {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            per_platform_from_intermediate::<_, Spec, _>(deserializer)
        }
    }
    impl<'de> Deserialize<'de> for super::Spec {
        fn deserialize<D>(deserializer: D) -> Result<super::Spec, D::Error>
        where
            D: Deserializer<'de>,
        {
            let internal = Spec::deserialize(deserializer)?;
            super::Spec::try_from(internal).map_err(serde::de::Error::custom)
        }
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
    pub async fn test_spec_from_lua() {
        let lua_content = "
        test = {\n
        }\n
        ";
        let test_spec: PerPlatform<Spec> = exec_lua(lua_content, "test").unwrap();
        assert!(matches!(test_spec.default, Spec::AutoDetect));
        let lua_content = "
        test = {\n
            type = 'busted',\n
        }\n
        ";
        let test_spec: PerPlatform<Spec> = exec_lua(lua_content, "test").unwrap();
        assert_eq!(test_spec.default, Spec::Busted(Busted::default()));
        let lua_content = "
        test = {\n
            type = 'busted',\n
            flags = { 'foo', 'bar' },\n
        }\n
        ";
        let test_spec: PerPlatform<Spec> = exec_lua(lua_content, "test").unwrap();
        assert_eq!(
            test_spec.default,
            Spec::Busted(Busted {
                flags: vec!["foo".into(), "bar".into()],
            })
        );
        let lua_content = "
        test = {\n
            type = 'command',\n
        }\n
        ";
        let result: Result<PerPlatform<Spec>, _> = exec_lua(lua_content, "test");
        let _err = result.unwrap_err();
        let lua_content = "
        test = {\n
            type = 'command',\n
            command = 'foo',\n
            script = 'bar',\n
        }\n
        ";
        let result: Result<PerPlatform<Spec>, _> = exec_lua(lua_content, "test");
        let _err = result.unwrap_err();
        let lua_content = "
        test = {\n
            type = 'command',\n
            command = 'baz',\n
            flags = { 'foo', 'bar' },\n
        }\n
        ";
        let test_spec: PerPlatform<Spec> = exec_lua(lua_content, "test").unwrap();
        assert_eq!(
            test_spec.default,
            Spec::Command(Command {
                command: "baz".into(),
                flags: vec!["foo".into(), "bar".into()],
            })
        );
        let lua_content = "
        test = {\n
            type = 'command',\n
            script = 'test.lua',\n
            flags = { 'foo', 'bar' },\n
        }\n
        ";
        let test_spec: PerPlatform<Spec> = exec_lua(lua_content, "test").unwrap();
        assert_eq!(
            test_spec.default,
            Spec::Script(LuaScript {
                script: PathBuf::from("test.lua"),
                flags: vec!["foo".into(), "bar".into()],
            })
        );
        let lua_content = "
        test = {\n
            type = 'command',\n
            command = 'baz',\n
            flags = { 'foo', 'bar' },\n
            platforms = {\n
                unix = { flags = { 'baz' }, },\n
                macosx = {\n
                    script = 'bat.lua',\n
                    flags = { 'bat' },\n
                },\n
                linux = { type = 'busted' },\n
            },\n
        }\n
        ";
        let test_spec: PerPlatform<Spec> = exec_lua(lua_content, "test").unwrap();
        assert_eq!(
            test_spec.default,
            Spec::Command(Command {
                command: "baz".into(),
                flags: vec!["foo".into(), "bar".into()],
            })
        );
        let unix = test_spec
            .per_platform
            .get(&platform::Identifier::Unix)
            .unwrap();
        assert_eq!(
            *unix,
            Spec::Command(Command {
                command: "baz".into(),
                flags: vec!["foo".into(), "bar".into(), "baz".into()],
            })
        );
        let macosx = test_spec
            .per_platform
            .get(&platform::Identifier::MacOSX)
            .unwrap();
        assert_eq!(
            *macosx,
            Spec::Script(LuaScript {
                script: "bat.lua".into(),
                flags: vec!["foo".into(), "bar".into(), "bat".into(), "baz".into()],
            })
        );
        let linux = test_spec
            .per_platform
            .get(&platform::Identifier::Linux)
            .unwrap();
        assert_eq!(
            *linux,
            Spec::Busted(Busted {
                flags: vec!["foo".into(), "bar".into(), "baz".into()],
            })
        );
        let lua_content = "
        test = {\n
            type = 'busted-nlua',\n
        }";
        let test_spec: PerPlatform<Spec> = exec_lua(lua_content, "test").unwrap();
        assert_eq!(
            test_spec.default,
            Spec::BustedNlua(Busted { flags: Vec::new() })
        );
    }
}
