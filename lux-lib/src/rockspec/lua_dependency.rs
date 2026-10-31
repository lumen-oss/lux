use crate::package;
use std::{collections::HashMap, convert::Infallible, fmt::Display, str::FromStr};

use crate::{
    lockfile::{OptState, PinnedState},
    rockspec,
};
use miette::Diagnostic;
use serde::{Deserialize, Deserializer};
use thiserror::Error;

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Req(#[from] package::req::Error),
}

/// Specification for a Lua dependency in a Lux project
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub(crate) package_req: package::Req,
    pub(crate) pin: PinnedState,
    pub(crate) opt: OptState,
    pub(crate) source: Option<rockspec::source::Spec>,
}

impl Spec {
    pub fn new(
        package_req: package::Req,
        pin: PinnedState,
        opt: OptState,
        source: Option<rockspec::source::Spec>,
    ) -> Self {
        Self {
            package_req,
            pin,
            opt,
            source,
        }
    }

    pub fn package_req(&self) -> &package::Req {
        &self.package_req
    }
    pub fn pin(&self) -> &PinnedState {
        &self.pin
    }
    pub fn opt(&self) -> &OptState {
        &self.opt
    }
    pub fn source(&self) -> &Option<rockspec::source::Spec> {
        &self.source
    }
    pub fn into_package_req(self) -> package::Req {
        self.package_req
    }
    pub fn name(&self) -> &package::Name {
        self.package_req.name()
    }
    pub fn version_req(&self) -> &package::VersionReq {
        self.package_req.version_req()
    }
    pub fn matches(&self, package: &package::Spec) -> bool {
        self.package_req.matches(package)
    }
}

impl From<package::Name> for Spec {
    fn from(name: package::Name) -> Self {
        Self {
            package_req: package::Req::from(name),
            pin: PinnedState::default(),
            opt: OptState::default(),
            source: None,
        }
    }
}

impl From<package::Req> for Spec {
    fn from(package_req: package::Req) -> Self {
        Self {
            package_req,
            pin: PinnedState::default(),
            opt: OptState::default(),
            source: None,
        }
    }
}

impl FromStr for Spec {
    type Err = Error;

    fn from_str(str: &str) -> Result<Self, Error> {
        let package_req = package::Req::from_str(str)?;
        Ok(Self {
            package_req,
            pin: PinnedState::default(),
            opt: OptState::default(),
            source: None,
        })
    }
}

impl Display for Spec {
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
impl rockspec::platform::PartialOverride for Vec<Spec> {
    type Err = Infallible;

    fn apply_overrides(&self, override_vec: &Self) -> Result<Self, Self::Err> {
        let mut result_map: HashMap<String, Spec> = self
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

impl rockspec::platform::Overridable for Vec<Spec> {
    type Err = Infallible;

    fn on_nil<T>() -> Result<super::PerPlatform<T>, <Self as rockspec::platform::Overridable>::Err>
    where
        T: rockspec::platform::Overridable,
        T: Default,
    {
        Ok(rockspec::PerPlatform::default())
    }
}

impl<'de> Deserialize<'de> for Spec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let package_req = package::Req::deserialize(deserializer)?;
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
pub enum Kind<T> {
    Regular(Vec<T>),
    Build(Vec<T>),
    Test(Vec<T>),
}

#[cfg(test)]
mod tests {
    use crate::rockspec::platform::PartialOverride;

    use ottavino::{Closure, Executor, Fuel, Lua, Value};
    use ottavino_util::serde::from_value;

    use super::*;

    fn eval_lua<T: serde::de::DeserializeOwned>(code: &str) -> Result<T, ottavino::ExternError> {
        Lua::core().try_enter(|ctx| {
            let closure = Closure::load(ctx, None, code.as_bytes())?;
            let executor = Executor::start(ctx, closure.into(), ());
            executor.step(ctx, &mut Fuel::with(i32::MAX))?;
            from_value(executor.take_result::<Value<'_>>(ctx)??).map_err(ottavino::Error::from)
        })
    }

    #[tokio::test]
    async fn test_override_lua_dependency_spec() {
        let neorg_a: Spec = "neorg 1.0.0".parse().unwrap();
        let neorg_b: Spec = "neorg 2.0.0".parse().unwrap();
        let foo: Spec = "foo 1.0.0".parse().unwrap();
        let bar: Spec = "bar 1.0.0".parse().unwrap();
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
    fn lua_dependency_kind_from_lua() {
        let regular_deps: Kind<Spec> =
            eval_lua(r#"return { regular = {"neorg 1.0.0", "foo 1.0.0"} }"#).unwrap();
        let build_deps: Kind<Spec> =
            eval_lua(r#"return { build = {"neorg 1.0.0", "foo 1.0.0"} }"#).unwrap();
        let test_deps: Kind<Spec> =
            eval_lua(r#"return { test = {"neorg 1.0.0", "foo 1.0.0"} }"#).unwrap();

        match regular_deps {
            Kind::Regular(deps) => {
                assert_eq!(deps.len(), 2);
                assert_eq!(deps[0].to_string(), "neorg==1.0.0");
                assert_eq!(deps[1].to_string(), "foo==1.0.0");
            }
            _ => panic!("Expected regular dependencies"),
        }

        match build_deps {
            Kind::Build(deps) => {
                assert_eq!(deps.len(), 2);
                assert_eq!(deps[0].to_string(), "neorg==1.0.0");
                assert_eq!(deps[1].to_string(), "foo==1.0.0");
            }
            _ => panic!("Expected build dependencies"),
        }

        match test_deps {
            Kind::Test(deps) => {
                assert_eq!(deps.len(), 2);
                assert_eq!(deps[0].to_string(), "neorg==1.0.0");
                assert_eq!(deps[1].to_string(), "foo==1.0.0");
            }
            _ => panic!("Expected test dependencies"),
        }

        eval_lua::<Kind<Spec>>("return {}").unwrap_err();
    }
}
