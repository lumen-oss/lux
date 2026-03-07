use std::{collections::HashMap, convert::Infallible, fmt::Display, str::FromStr};

use serde::{Deserialize, Deserializer};
use thiserror::Error;

use crate::{
    lockfile::{OptState, PinnedState},
    lua_rockspec::{
        ExternalDependencySpec, PartialOverride, PerPlatform, PlatformOverridable, RockSourceSpec,
    },
    package::{PackageName, PackageReq, PackageReqParseError, PackageSpec, PackageVersionReq},
};

#[derive(Error, Debug)]
pub enum LuaDependencySpecParseError {
    #[error(transparent)]
    PackageReq(#[from] PackageReqParseError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LuaDependencySpec {
    pub(crate) package_req: PackageReq,
    pub(crate) pin: PinnedState,
    pub(crate) opt: OptState,
    pub(crate) source: Option<RockSourceSpec>,
}

impl LuaDependencySpec {
    pub fn package_req(&self) -> &PackageReq {
        &self.package_req
    }
    pub fn pin(&self) -> &PinnedState {
        &self.pin
    }
    pub fn opt(&self) -> &OptState {
        &self.opt
    }
    pub fn source(&self) -> &Option<RockSourceSpec> {
        &self.source
    }
    pub fn into_package_req(self) -> PackageReq {
        self.package_req
    }
    pub fn name(&self) -> &PackageName {
        self.package_req.name()
    }
    pub fn version_req(&self) -> &PackageVersionReq {
        self.package_req.version_req()
    }
    pub fn matches(&self, package: &PackageSpec) -> bool {
        self.package_req.matches(package)
    }
}

impl From<PackageName> for LuaDependencySpec {
    fn from(name: PackageName) -> Self {
        Self {
            package_req: PackageReq::from(name),
            pin: PinnedState::default(),
            opt: OptState::default(),
            source: None,
        }
    }
}

impl From<PackageReq> for LuaDependencySpec {
    fn from(package_req: PackageReq) -> Self {
        Self {
            package_req,
            pin: PinnedState::default(),
            opt: OptState::default(),
            source: None,
        }
    }
}

impl FromStr for LuaDependencySpec {
    type Err = LuaDependencySpecParseError;

    fn from_str(str: &str) -> Result<Self, LuaDependencySpecParseError> {
        let package_req = PackageReq::from_str(str)?;
        Ok(Self {
            package_req,
            pin: PinnedState::default(),
            opt: OptState::default(),
            source: None,
        })
    }
}

impl Display for LuaDependencySpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.version_req().is_any() {
            self.name().fmt(f)
        } else {
            f.write_str(format!("{}{}", self.name(), self.version_req()).as_str())
        }
    }
}

/// Override `base_deps` with `override_deps`
/// - Adds missing dependencies
/// - Replaces dependencies with the same name
impl PartialOverride for Vec<LuaDependencySpec> {
    type Err = Infallible;

    fn apply_overrides(&self, override_vec: &Self) -> Result<Self, Self::Err> {
        let mut result_map: HashMap<String, LuaDependencySpec> = self
            .iter()
            .map(|dep| (dep.name().clone().to_string(), dep.clone()))
            .collect();
        for override_dep in override_vec {
            result_map.insert(
                override_dep.name().clone().to_string(),
                override_dep.clone(),
            );
        }
        Ok(result_map.into_values().collect())
    }
}

impl PlatformOverridable for Vec<LuaDependencySpec> {
    type Err = Infallible;

    fn on_nil<T>() -> Result<super::PerPlatform<T>, <Self as PlatformOverridable>::Err>
    where
        T: PlatformOverridable,
        T: Default,
    {
        Ok(PerPlatform::default())
    }
}

impl<'de> Deserialize<'de> for LuaDependencySpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let package_req = PackageReq::deserialize(deserializer)?;
        Ok(Self {
            package_req,
            pin: PinnedState::default(),
            opt: OptState::default(),
            source: None,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DependencyType<T> {
    Regular(Vec<T>),
    Build(Vec<T>),
    Test(Vec<T>),
    External(HashMap<String, ExternalDependencySpec>),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LuaDependencyType<T> {
    Regular(Vec<T>),
    Build(Vec<T>),
    Test(Vec<T>),
}

#[cfg(test)]
mod test {

    use path_slash::PathBufExt;
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

    #[tokio::test]
    async fn test_override_lua_dependency_spec() {
        let neorg_a: LuaDependencySpec = "neorg 1.0.0".parse().unwrap();
        let neorg_b: LuaDependencySpec = "neorg 2.0.0".parse().unwrap();
        let foo: LuaDependencySpec = "foo 1.0.0".parse().unwrap();
        let bar: LuaDependencySpec = "bar 1.0.0".parse().unwrap();
        let base_vec = vec![neorg_a, foo.clone()];
        let override_vec = vec![neorg_b.clone(), bar.clone()];
        let result = base_vec.apply_overrides(&override_vec).unwrap();
        assert_eq!(result.clone().len(), 3);
        assert_eq!(
            result
                .into_iter()
                .filter(|dep| *dep == neorg_b || *dep == foo || *dep == bar)
                .count(),
            3
        );
    }

    #[test]
    fn test_dependency_type_from_lua() {
        let regular_deps: DependencyType<LuaDependencySpec> =
            eval_lua(r#"return { regular = {"neorg 1.0.0", "foo 1.0.0"} }"#).unwrap();
        let build_deps: DependencyType<LuaDependencySpec> =
            eval_lua(r#"return { build = {"neorg 1.0.0", "foo 1.0.0"} }"#).unwrap();
        let test_deps: DependencyType<LuaDependencySpec> =
            eval_lua(r#"return { test = {"neorg 1.0.0", "foo 1.0.0"} }"#).unwrap();
        let external_deps: DependencyType<ExternalDependencySpec> = eval_lua(
            r#"return { external = { foo = { header = "foo.h", library = "libfoo.so" }, bar = { header = "bar.h" } } }"#,
        )
        .unwrap();

        match regular_deps {
            DependencyType::Regular(deps) => {
                assert_eq!(deps.len(), 2);
                assert_eq!(deps[0].to_string(), "neorg==1.0.0");
                assert_eq!(deps[1].to_string(), "foo==1.0.0");
            }
            _ => panic!("Expected regular dependencies"),
        }

        match build_deps {
            DependencyType::Build(deps) => {
                assert_eq!(deps.len(), 2);
                assert_eq!(deps[0].to_string(), "neorg==1.0.0");
                assert_eq!(deps[1].to_string(), "foo==1.0.0");
            }
            _ => panic!("Expected build dependencies"),
        }

        match test_deps {
            DependencyType::Test(deps) => {
                assert_eq!(deps.len(), 2);
                assert_eq!(deps[0].to_string(), "neorg==1.0.0");
                assert_eq!(deps[1].to_string(), "foo==1.0.0");
            }
            _ => panic!("Expected test dependencies"),
        }

        match external_deps {
            DependencyType::External(deps) => {
                assert_eq!(deps.len(), 2);
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
            }
            _ => panic!("Expected external dependencies"),
        }

        let _err: piccolo::StaticError =
            eval_lua::<DependencyType<ExternalDependencySpec>>("return {}").unwrap_err();
    }

    #[test]
    fn test_lua_dependency_type_from_lua() {
        let regular_deps: LuaDependencyType<LuaDependencySpec> =
            eval_lua(r#"return { regular = {"neorg 1.0.0", "foo 1.0.0"} }"#).unwrap();
        let build_deps: LuaDependencyType<LuaDependencySpec> =
            eval_lua(r#"return { build = {"neorg 1.0.0", "foo 1.0.0"} }"#).unwrap();
        let test_deps: LuaDependencyType<LuaDependencySpec> =
            eval_lua(r#"return { test = {"neorg 1.0.0", "foo 1.0.0"} }"#).unwrap();

        match regular_deps {
            LuaDependencyType::Regular(deps) => {
                assert_eq!(deps.len(), 2);
                assert_eq!(deps[0].to_string(), "neorg==1.0.0");
                assert_eq!(deps[1].to_string(), "foo==1.0.0");
            }
            _ => panic!("Expected regular dependencies"),
        }

        match build_deps {
            LuaDependencyType::Build(deps) => {
                assert_eq!(deps.len(), 2);
                assert_eq!(deps[0].to_string(), "neorg==1.0.0");
                assert_eq!(deps[1].to_string(), "foo==1.0.0");
            }
            _ => panic!("Expected build dependencies"),
        }

        match test_deps {
            LuaDependencyType::Test(deps) => {
                assert_eq!(deps.len(), 2);
                assert_eq!(deps[0].to_string(), "neorg==1.0.0");
                assert_eq!(deps[1].to_string(), "foo==1.0.0");
            }
            _ => panic!("Expected test dependencies"),
        }

        eval_lua::<LuaDependencyType<LuaDependencySpec>>("return {}").unwrap_err();
    }
}
