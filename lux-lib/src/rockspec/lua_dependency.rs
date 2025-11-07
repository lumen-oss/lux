use std::{collections::HashMap, convert::Infallible, fmt::Display, str::FromStr};

use mlua::{FromLua, IntoLua, LuaSerdeExt};
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
    /// Optional LuaLS library overrides to include in .luarc.json
    pub(crate) luarc: Option<Vec<String>>, 
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
    pub fn luarc(&self) -> &Option<Vec<String>> {
        &self.luarc
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
            luarc: None,
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
            luarc: None,
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
            luarc: None,
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

impl FromLua for LuaDependencySpec {
    fn from_lua(value: mlua::Value, lua: &mlua::Lua) -> mlua::Result<Self> {
        let package_req = lua.from_value(value)?;
        Ok(Self {
            package_req,
            pin: PinnedState::default(),
            opt: OptState::default(),
            source: None,
            luarc: None,
        })
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
            luarc: None,
        })
    }
}

impl mlua::UserData for LuaDependencySpec {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("name", |_, this, ()| Ok(this.name().to_string()));
        methods.add_method("version_req", |_, this, ()| {
            Ok(this.version_req().to_string())
        });
        methods.add_method("matches", |_, this, package: PackageSpec| {
            Ok(this.matches(&package))
        });
        methods.add_method("package_req", |_, this, ()| Ok(this.package_req().clone()));
    }
}

pub enum DependencyType<T> {
    Regular(Vec<T>),
    Build(Vec<T>),
    Test(Vec<T>),
    External(HashMap<String, ExternalDependencySpec>),
}

impl<T> IntoLua for DependencyType<T>
where
    T: IntoLua,
{
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        let table = lua.create_table()?;

        match self {
            DependencyType::Regular(deps) => {
                table.set("regular", deps)?;
            }
            DependencyType::Build(deps) => {
                table.set("build", deps)?;
            }
            DependencyType::Test(deps) => {
                table.set("test", deps)?;
            }
            DependencyType::External(deps) => {
                table.set("external", deps)?;
            }
        }

        Ok(mlua::Value::Table(table))
    }
}

impl<T> FromLua for DependencyType<T>
where
    T: FromLua,
{
    fn from_lua(value: mlua::Value, _lua: &mlua::Lua) -> mlua::Result<Self> {
        let tbl = value
            .as_table()
            .ok_or(mlua::Error::FromLuaConversionError {
                from: "Value",
                to: "table".to_string(),
                message: Some("Expected a table".to_string()),
            })?;

        let deps = {
            if let Some(regular) = tbl.get("regular")? {
                DependencyType::Regular(regular)
            } else if let Some(build) = tbl.get("build")? {
                DependencyType::Build(build)
            } else if let Some(test) = tbl.get("test")? {
                DependencyType::Test(test)
            } else if let Some(external) = tbl.get("external")? {
                DependencyType::External(external)
            } else {
                return Err(mlua::Error::FromLuaConversionError {
                    from: "table",
                    to: "DependencyType".to_string(),
                    message: Some(
                        "expected a table with `regular`, `build`, `test` or `external`"
                            .to_string(),
                    ),
                });
            }
        };

        Ok(deps)
    }
}

pub enum LuaDependencyType<T> {
    Regular(Vec<T>),
    Build(Vec<T>),
    Test(Vec<T>),
}

impl<T> IntoLua for LuaDependencyType<T>
where
    T: IntoLua,
{
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        let table = lua.create_table()?;

        match self {
            LuaDependencyType::Regular(deps) => {
                table.set("regular", deps)?;
            }
            LuaDependencyType::Build(deps) => {
                table.set("build", deps)?;
            }
            LuaDependencyType::Test(deps) => {
                table.set("test", deps)?;
            }
        }

        Ok(mlua::Value::Table(table))
    }
}

impl<T> FromLua for LuaDependencyType<T>
where
    T: FromLua,
{
    fn from_lua(value: mlua::Value, _lua: &mlua::Lua) -> mlua::Result<Self> {
        let tbl = value
            .as_table()
            .ok_or(mlua::Error::FromLuaConversionError {
                from: "Value",
                to: "table".to_string(),
                message: Some("Expected a table".to_string()),
            })?;

        let deps = {
            if let Some(regular) = tbl.get("regular")? {
                LuaDependencyType::Regular(regular)
            } else if let Some(build) = tbl.get("build")? {
                LuaDependencyType::Build(build)
            } else if let Some(test) = tbl.get("test")? {
                LuaDependencyType::Test(test)
            } else {
                return Err(mlua::Error::FromLuaConversionError {
                    from: "table",
                    to: "LuaDependencyType".to_string(),
                    message: Some("expected a table with `regular`, `build` or `test`".to_string()),
                });
            }
        };

        Ok(deps)
    }
}

#[cfg(test)]
mod test {

    use super::*;

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
}
