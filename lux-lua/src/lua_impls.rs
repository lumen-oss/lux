#![allow(unused)]

//! Lua implementations for various types in lux-lib. These are used to convert between Lua and
//! Rust types when calling Rust functions from Lua.

use std::{collections::HashMap, path::PathBuf, str::FromStr, time::Duration};

use itertools::Itertools;
use mlua::prelude::*;
use mlua_extras::typed::{
    IntoLuaTypeLiteral, Type, Typed, TypedDataFields, TypedDataMethods, TypedUserData,
};
use path_slash::PathBufExt;
use serde::{Deserialize, Serialize};
use serde_enum_str::{Deserialize_enum_str, Serialize_enum_str};
use strum::IntoEnumIterator;
use url::Url;

use lux_lib::{
    build::BuildBehaviour,
    config::{tree::RockLayoutConfig, Config, ConfigBuilder},
    git::GitSource,
    lockfile::{
        LocalPackage, LocalPackageHashes, LocalPackageId, LockConstraint, Lockfile, LockfileGuard,
        OptState, PinnedState, ReadOnly, ReadWrite,
    },
    lua_rockspec::{
        BuildBackendSpec, BuildSpec, BuiltinBuildSpec, BustedTestSpec, CMakeBuildSpec,
        CommandBuildSpec, CommandTestSpec, ExternalDependencySpec, InstallSpec, LocalLuaRockspec,
        LuaModule, LuaScriptTestSpec, MakeBuildSpec, ModulePaths, ModuleSpec, PartialLuaRockspec,
        PartialOverride, PerPlatform, PlatformIdentifier, PlatformOverridable, PlatformSupport,
        RemoteLuaRockspec, RemoteRockSource, RockDescription, RockSourceSpec, RockspecFormat,
        RustMluaBuildSpec, TestSpec, TreesitterParserBuildSpec,
    },
    lua_version::LuaVersion,
    operations::{DownloadedRockspec, PackageInstallSpec, SyncReport},
    package::{PackageName, PackageReq, PackageSpec, PackageVersion, PackageVersionReq, SpecRev},
    project::{
        project_toml::{LocalProjectToml, PartialProjectToml, RemoteProjectToml},
        Project,
    },
    remote_package_db::RemotePackageDB,
    rockspec::{
        lua_dependency::{DependencyType, LuaDependencySpec, LuaDependencyType},
        Rockspec,
    },
    tree::{EntryType, InstallTree, RockLayout, RockMatches, Tree},
    workspace::Workspace,
};

macro_rules! impl_from_lua_userdata {
    ($wrapper:ty) => {
        impl FromLua for $wrapper {
            fn from_lua(value: LuaValue, _lua: &Lua) -> LuaResult<Self> {
                match value {
                    LuaValue::UserData(ud) => Ok(ud.borrow::<$wrapper>()?.clone()),
                    v => Err(LuaError::FromLuaConversionError {
                        from: v.type_name(),
                        to: stringify!($wrapper).to_string(),
                        message: None,
                    }),
                }
            }
        }
    };
}

#[derive(Debug, Clone)]
pub struct LuaUrl(pub Url);

impl Typed for LuaUrl {
    fn ty() -> Type {
        Type::string()
    }
}

impl FromLua for LuaUrl {
    fn from_lua(value: LuaValue, lua: &Lua) -> LuaResult<Self> {
        let url_str: String = FromLua::from_lua(value, lua)?;
        Url::parse(&url_str).map(LuaUrl).into_lua_err()
    }
}

impl IntoLua for LuaUrl {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        self.0.to_string().into_lua(lua)
    }
}

#[derive(Debug, Clone)]
pub struct LuaVersionLua(pub LuaVersion);

impl IntoLuaTypeLiteral for LuaVersionLua {
    fn into_lua_type_literal(self) -> String {
        format!("'{}'", self.0)
    }
}

impl Typed for LuaVersionLua {
    fn ty() -> Type {
        Type::union(LuaVersion::iter().map(|v| Type::literal(LuaVersionLua(v))))
    }
}

impl FromLua for LuaVersionLua {
    fn from_lua(value: LuaValue, lua: &Lua) -> LuaResult<Self> {
        let version_str: String = FromLua::from_lua(value, lua)?;
        LuaVersion::from_str(&version_str)
            .map(LuaVersionLua)
            .into_lua_err()
    }
}

impl IntoLua for LuaVersionLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        self.0.to_string().into_lua(lua)
    }
}

#[derive(Debug, Clone)]
pub struct PackageVersionLua(pub PackageVersion);

impl Typed for PackageVersionLua {
    fn ty() -> Type {
        Type::string()
    }
}

impl IntoLua for PackageVersionLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        self.0.to_string().into_lua(lua)
    }
}

impl FromLua for PackageVersionLua {
    fn from_lua(value: LuaValue, lua: &Lua) -> LuaResult<Self> {
        let s = String::from_lua(value, lua)?;
        PackageVersion::from_str(&s)
            .map(PackageVersionLua)
            .map_err(|err| LuaError::DeserializeError(err.to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct SpecRevLua(pub SpecRev);

impl Typed for SpecRevLua {
    fn ty() -> Type {
        Type::integer()
    }
}

impl FromLua for SpecRevLua {
    fn from_lua(value: LuaValue, _lua: &Lua) -> LuaResult<Self> {
        match value {
            LuaValue::Integer(v) => {
                let u = u16::try_from(v).map_err(|err| {
                    LuaError::DeserializeError(format!("Error deserializing specrev {v}:\n{err}"))
                })?;
                Ok(SpecRevLua(SpecRev::from(u)))
            }
            v => Err(LuaError::DeserializeError(format!(
                "Expected specrev to be an integer, but got {}",
                v.type_name()
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PackageVersionReqLua(pub PackageVersionReq);

impl Typed for PackageVersionReqLua {
    fn ty() -> Type {
        Type::string()
    }
}

impl FromLua for PackageVersionReqLua {
    fn from_lua(value: LuaValue, lua: &Lua) -> LuaResult<Self> {
        PackageVersionReq::parse(&String::from_lua(value, lua)?)
            .map(PackageVersionReqLua)
            .into_lua_err()
    }
}

impl IntoLua for PackageVersionReqLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        self.0.to_string().into_lua(lua)
    }
}

#[derive(Debug, Clone)]
pub struct PackageNameLua(pub PackageName);

impl Typed for PackageNameLua {
    fn ty() -> Type {
        Type::string()
    }
}

impl IntoLua for PackageNameLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        self.0.to_string().into_lua(lua)
    }
}

impl FromLua for PackageNameLua {
    fn from_lua(value: LuaValue, lua: &Lua) -> LuaResult<Self> {
        Ok(PackageNameLua(PackageName::new(String::from_lua(
            value, lua,
        )?)))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PinnedStateLua(pub PinnedState);

impl Typed for PinnedStateLua {
    fn ty() -> Type {
        Type::boolean()
    }
}

impl FromLua for PinnedStateLua {
    fn from_lua(value: LuaValue, lua: &Lua) -> LuaResult<Self> {
        Ok(PinnedStateLua(PinnedState::from(bool::from_lua(
            value, lua,
        )?)))
    }
}

impl IntoLua for PinnedStateLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        match self.0 {
            PinnedState::Pinned => true.into_lua(lua),
            PinnedState::Unpinned => false.into_lua(lua),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OptStateLua(pub OptState);

impl Typed for OptStateLua {
    fn ty() -> Type {
        Type::boolean()
    }
}

impl FromLua for OptStateLua {
    fn from_lua(value: LuaValue, lua: &Lua) -> LuaResult<Self> {
        Ok(OptStateLua(OptState::from(bool::from_lua(value, lua)?)))
    }
}

impl IntoLua for OptStateLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        match self.0 {
            OptState::Optional => true.into_lua(lua),
            OptState::Required => false.into_lua(lua),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LocalPackageIdLua(pub LocalPackageId);

impl Typed for LocalPackageIdLua {
    fn ty() -> Type {
        Type::string()
    }
}

impl FromLua for LocalPackageIdLua {
    fn from_lua(value: LuaValue, lua: &Lua) -> LuaResult<Self> {
        Ok(LocalPackageIdLua(unsafe {
            LocalPackageId::from_unchecked(String::from_lua(value, lua)?)
        }))
    }
}

impl IntoLua for LocalPackageIdLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        self.0.into_string().into_lua(lua)
    }
}

#[derive(Debug, Clone)]
pub struct LockConstraintLua(pub LockConstraint);

impl Typed for LockConstraintLua {
    fn ty() -> Type {
        Type::string()
    }
}

impl IntoLua for LockConstraintLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        match self.0 {
            LockConstraint::Unconstrained => "*".into_lua(lua),
            LockConstraint::Constrained(req) => req.to_string().into_lua(lua),
        }
    }
}

impl FromLua for LockConstraintLua {
    fn from_lua(value: LuaValue, lua: &Lua) -> LuaResult<Self> {
        let s = String::from_lua(value, lua)?;
        match s.as_str() {
            "*" => Ok(LockConstraintLua(LockConstraint::Unconstrained)),
            _ => Ok(LockConstraintLua(LockConstraint::Constrained(
                s.parse().into_lua_err()?,
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExternalDependencySpecLua(pub ExternalDependencySpec);

impl Typed for ExternalDependencySpecLua {
    fn ty() -> Type {
        Type::named("table")
    }
}

impl IntoLua for ExternalDependencySpecLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let table = lua.create_table()?;
        if let Some(path) = self.0.header {
            table.set("header", path.to_slash_lossy().to_string())?;
        }
        if let Some(path) = self.0.library {
            table.set("library", path.to_slash_lossy().to_string())?;
        }
        Ok(LuaValue::Table(table))
    }
}

impl FromLua for ExternalDependencySpecLua {
    fn from_lua(value: LuaValue, _lua: &Lua) -> LuaResult<Self> {
        if let LuaValue::Table(table) = value {
            let header: Option<PathBuf> = table.get("header")?;
            let library: Option<PathBuf> = table.get("library")?;
            Ok(ExternalDependencySpecLua(ExternalDependencySpec {
                header,
                library,
            }))
        } else {
            Err(LuaError::FromLuaConversionError {
                from: "non-table",
                to: "ExternalDependencySpecLua".to_string(),
                message: Some("Expected a table".to_string()),
            })
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlatformIdentifierLua(pub PlatformIdentifier);

impl IntoLuaTypeLiteral for PlatformIdentifierLua {
    fn into_lua_type_literal(self) -> String {
        self.0.to_string()
    }
}

impl Typed for PlatformIdentifierLua {
    fn ty() -> Type {
        Type::union(PlatformIdentifier::iter().map(|p| Type::literal(PlatformIdentifierLua(p))))
    }
}

impl FromLua for PlatformIdentifierLua {
    fn from_lua(value: LuaValue, lua: &Lua) -> LuaResult<Self> {
        let s = String::from_lua(value, lua)?;
        Ok(PlatformIdentifierLua(
            s.parse().unwrap_or(PlatformIdentifier::Unknown(s)),
        ))
    }
}

impl IntoLua for PlatformIdentifierLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        self.0.to_string().into_lua(lua)
    }
}

#[derive(Debug, Clone)]
pub struct RockspecFormatLua(pub RockspecFormat);

impl Typed for RockspecFormatLua {
    fn ty() -> Type {
        Type::string()
    }
}

impl FromLua for RockspecFormatLua {
    fn from_lua(value: LuaValue, lua: &Lua) -> LuaResult<Self> {
        let s = String::from_lua(value, lua)?;
        RockspecFormat::from_str(&s)
            .map(RockspecFormatLua)
            .map_err(|err| LuaError::DeserializeError(err.to_string()))
    }
}

impl IntoLua for RockspecFormatLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        self.0.to_string().into_lua(lua)
    }
}

#[derive(Debug, Clone)]
pub struct LuaModuleLua(pub LuaModule);

impl Typed for LuaModuleLua {
    fn ty() -> Type {
        Type::string()
    }
}

impl IntoLua for LuaModuleLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        self.0.as_str().to_string().into_lua(lua)
    }
}

impl FromLua for LuaModuleLua {
    fn from_lua(value: LuaValue, lua: &Lua) -> LuaResult<Self> {
        let s = String::from_lua(value, lua)?;
        Ok(LuaModuleLua(LuaModule::from_str(&s).into_lua_err()?))
    }
}

pub struct DependencyTypeLua<T>(pub DependencyType<T>);

impl<T: Typed> Typed for DependencyTypeLua<T> {
    fn ty() -> Type {
        Type::named("table")
    }
}

impl<T> IntoLua for DependencyTypeLua<T>
where
    T: IntoLua,
{
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let table = lua.create_table()?;
        match self.0 {
            DependencyType::Regular(deps) => table.set("regular", deps)?,
            DependencyType::Build(deps) => table.set("build", deps)?,
            DependencyType::Test(deps) => table.set("test", deps)?,
            DependencyType::External(deps) => {
                let wrapped: HashMap<String, ExternalDependencySpecLua> = deps
                    .into_iter()
                    .map(|(k, v)| (k, ExternalDependencySpecLua(v)))
                    .collect();
                table.set("external", wrapped)?;
            }
        }
        Ok(LuaValue::Table(table))
    }
}

impl<T> FromLua for DependencyTypeLua<T>
where
    T: FromLua,
{
    fn from_lua(value: LuaValue, _lua: &Lua) -> LuaResult<Self> {
        let tbl = value
            .as_table()
            .ok_or_else(|| LuaError::FromLuaConversionError {
                from: "Value",
                to: "DependencyTypeLua".to_string(),
                message: Some("Expected a table".to_string()),
            })?;
        let deps = if let Some(regular) = tbl.get("regular")? {
            DependencyType::Regular(regular)
        } else if let Some(build) = tbl.get("build")? {
            DependencyType::Build(build)
        } else if let Some(test) = tbl.get("test")? {
            DependencyType::Test(test)
        } else if let Some(external) =
            tbl.get::<Option<HashMap<String, ExternalDependencySpecLua>>>("external")?
        {
            DependencyType::External(external.into_iter().map(|(k, v)| (k, v.0)).collect())
        } else {
            return Err(LuaError::FromLuaConversionError {
                from: "table",
                to: "DependencyTypeLua".to_string(),
                message: Some(
                    "expected a table with `regular`, `build`, `test` or `external`".to_string(),
                ),
            });
        };
        Ok(DependencyTypeLua(deps))
    }
}

pub struct LuaDependencyTypeLua<T>(pub LuaDependencyType<T>);

impl<T: Typed> Typed for LuaDependencyTypeLua<T> {
    fn ty() -> Type {
        Type::named("table")
    }
}

impl<T> IntoLua for LuaDependencyTypeLua<T>
where
    T: IntoLua,
{
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let table = lua.create_table()?;
        match self.0 {
            LuaDependencyType::Regular(deps) => table.set("regular", deps)?,
            LuaDependencyType::Build(deps) => table.set("build", deps)?,
            LuaDependencyType::Test(deps) => table.set("test", deps)?,
        }
        Ok(LuaValue::Table(table))
    }
}

impl<T> FromLua for LuaDependencyTypeLua<T>
where
    T: FromLua,
{
    fn from_lua(value: LuaValue, _lua: &Lua) -> LuaResult<Self> {
        let tbl = value
            .as_table()
            .ok_or_else(|| LuaError::FromLuaConversionError {
                from: "Value",
                to: "LuaDependencyTypeLua".to_string(),
                message: Some("Expected a table".to_string()),
            })?;
        let deps = if let Some(regular) = tbl.get("regular")? {
            LuaDependencyType::Regular(regular)
        } else if let Some(build) = tbl.get("build")? {
            LuaDependencyType::Build(build)
        } else if let Some(test) = tbl.get("test")? {
            LuaDependencyType::Test(test)
        } else {
            return Err(LuaError::FromLuaConversionError {
                from: "table",
                to: "LuaDependencyTypeLua".to_string(),
                message: Some("expected a table with `regular`, `build`, or `test`".to_string()),
            });
        };
        Ok(LuaDependencyTypeLua(deps))
    }
}

pub struct RockMatchesLua(pub RockMatches);

impl Typed for RockMatchesLua {
    fn ty() -> Type {
        Type::named("table")
    }
}

impl IntoLua for RockMatchesLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let table = lua.create_table()?;
        let is_found = self.0.is_found();
        table.set("is_found", lua.create_function(move |_, ()| Ok(is_found))?)?;
        match self.0 {
            RockMatches::NotFound(req) => table.set("not_found", PackageReqLua(req))?,
            RockMatches::Single(id) => table.set("single", LocalPackageIdLua(id))?,
            RockMatches::Many(ids) => table.set(
                "many",
                ids.into_iter().map(LocalPackageIdLua).collect::<Vec<_>>(),
            )?,
        }
        Ok(LuaValue::Table(table))
    }
}

pub struct RockSourceSpecLua(pub RockSourceSpec);

impl Typed for RockSourceSpecLua {
    fn ty() -> Type {
        Type::named("table")
    }
}

impl IntoLua for RockSourceSpecLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let table = lua.create_table()?;
        match self.0 {
            RockSourceSpec::Git(git) => {
                table.set("git", GitSourceLua(git))?;
            }
            RockSourceSpec::File(path) => {
                table.set("file", path.to_slash_lossy().to_string())?;
            }
            RockSourceSpec::Url(url) => {
                table.set("url", url.to_string())?;
            }
        };
        Ok(LuaValue::Table(table))
    }
}

pub struct TestSpecLua(pub TestSpec);

impl Typed for TestSpecLua {
    fn ty() -> Type {
        Type::named("table")
    }
}

impl IntoLua for TestSpecLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let table = lua.create_table()?;
        match self.0 {
            TestSpec::AutoDetect => table.set("auto_detect", true)?,
            TestSpec::Busted(spec) => table.set("busted", BustedTestSpecLua(spec))?,
            TestSpec::BustedNlua(spec) => table.set("busted_nlua", BustedTestSpecLua(spec))?,
            TestSpec::Command(spec) => table.set("command", CommandTestSpecLua(spec))?,
            TestSpec::Script(spec) => table.set("script", LuaScriptTestSpecLua(spec))?,
        }
        Ok(LuaValue::Table(table))
    }
}

pub struct ModuleSpecLua(pub ModuleSpec);

impl Typed for ModuleSpecLua {
    fn ty() -> Type {
        Type::named("table")
    }
}

impl IntoLua for ModuleSpecLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let table = lua.create_table()?;
        match self.0 {
            ModuleSpec::SourcePath(path) => {
                table.set("source", path.to_slash_lossy().to_string())?
            }
            ModuleSpec::SourcePaths(paths) => table.set(
                "sources",
                paths
                    .into_iter()
                    .map(|p| p.to_slash_lossy().into_owned())
                    .collect::<Vec<_>>(),
            )?,
            ModuleSpec::ModulePaths(mp) => table.set("modules", ModulePathsLua(mp))?,
        }
        Ok(LuaValue::Table(table))
    }
}

pub struct BuiltinBuildSpecLua(pub BuiltinBuildSpec);

impl Typed for BuiltinBuildSpecLua {
    fn ty() -> Type {
        Type::named("table")
    }
}

impl IntoLua for BuiltinBuildSpecLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let table = lua.create_table()?;
        for (module, spec) in self.0.modules {
            table.set(module.as_str().to_string(), ModuleSpecLua(spec))?;
        }
        Ok(LuaValue::Table(table))
    }
}

pub struct BuildBackendSpecLua(pub BuildBackendSpec);

impl Typed for BuildBackendSpecLua {
    fn ty() -> Type {
        Type::named("table")
    }
}

impl IntoLua for BuildBackendSpecLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        match self.0 {
            BuildBackendSpec::Builtin(spec) => BuiltinBuildSpecLua(spec).into_lua(lua),
            BuildBackendSpec::Make(spec) => MakeBuildSpecLua(spec).into_lua(lua),
            BuildBackendSpec::CMake(spec) => CMakeBuildSpecLua(spec).into_lua(lua),
            BuildBackendSpec::Command(spec) => CommandBuildSpecLua(spec).into_lua(lua),
            BuildBackendSpec::LuaRock(s) => s.into_lua(lua),
            BuildBackendSpec::RustMlua(spec) => RustMluaBuildSpecLua(spec).into_lua(lua),
            BuildBackendSpec::TreesitterParser(spec) => {
                TreesitterParserBuildSpecLua(spec).into_lua(lua)
            }
            BuildBackendSpec::Source => "source".into_lua(lua),
        }
    }
}

pub struct SyncReportLua(pub SyncReport);

impl Typed for SyncReportLua {
    fn ty() -> Type {
        Type::named("table")
    }
}

impl IntoLua for SyncReportLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let table = lua.create_table()?;
        table.set(
            "added",
            self.0
                .added()
                .iter()
                .cloned()
                .map(LocalPackageLua)
                .collect::<Vec<_>>(),
        )?;
        table.set(
            "removed",
            self.0
                .removed()
                .iter()
                .cloned()
                .map(LocalPackageLua)
                .collect::<Vec<_>>(),
        )?;
        Ok(LuaValue::Table(table))
    }
}

pub struct PackageInstallSpecLua(pub PackageInstallSpec);

impl Typed for PackageInstallSpecLua {
    fn ty() -> Type {
        Type::named("table")
    }
}

impl FromLua for PackageInstallSpecLua {
    fn from_lua(value: LuaValue, lua: &Lua) -> LuaResult<Self> {
        let input: PackageInstallSpecInput = lua.from_value(value)?;
        let (package_str, entry_type, pin, opt, build_behaviour) = match input {
            PackageInstallSpecInput::Simple(s) => (
                s,
                EntryType::Entrypoint,
                false,
                false,
                BuildBehaviourLua::default(),
            ),
            PackageInstallSpecInput::Full {
                package,
                entry_type,
                pin,
                opt,
                build_behaviour,
            } => (package, entry_type.into(), pin, opt, build_behaviour),
        };
        let req = PackageReq::parse(&package_str).into_lua_err()?;
        let spec = PackageInstallSpec::new(req, entry_type)
            .pin(PinnedState::from(pin))
            .opt(OptState::from(opt))
            .build_behaviour(build_behaviour.into())
            .build();
        Ok(PackageInstallSpecLua(spec))
    }
}

#[derive(Debug, Clone)]
pub struct PackageSpecLua(pub PackageSpec);

impl Typed for PackageSpecLua {
    fn ty() -> Type {
        Type::named("PackageSpec")
    }
}

impl FromLua for PackageSpecLua {
    fn from_lua(value: LuaValue, _lua: &Lua) -> LuaResult<Self> {
        match value {
            LuaValue::UserData(ud) => Ok(ud.borrow::<PackageSpecLua>()?.clone()),
            v => Err(LuaError::FromLuaConversionError {
                from: v.type_name(),
                to: "PackageSpecLua".to_string(),
                message: None,
            }),
        }
    }
}

impl TypedUserData for PackageSpecLua {
    fn add_fields<F: TypedDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |_, this| Ok(this.0.name().to_string()));
        fields.add_field_method_get("version", |_, this| Ok(this.0.version().to_string()));
    }
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document(
            "Convert this spec to a package requirement (with an exact version requirement)",
        );
        methods.add_method("to_package_req", |_, this, ()| {
            Ok(PackageReqLua(this.0.clone().into_package_req()))
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Specification for a package with an exact name and version");
    }
}

impl mlua::UserData for PackageSpecLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct PackageReqLua(pub PackageReq);

impl Typed for PackageReqLua {
    fn ty() -> Type {
        Type::named("PackageReq")
    }
}

impl FromLua for PackageReqLua {
    fn from_lua(value: LuaValue, lua: &Lua) -> LuaResult<Self> {
        let str: String = lua.from_value(value)?;
        PackageReq::parse(&str).map(PackageReqLua).into_lua_err()
    }
}

impl TypedUserData for PackageReqLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_method("name", |_, this, ()| Ok(this.0.name().to_string()));
        methods.add_method("version_req", |_, this, ()| {
            Ok(this.0.version_req().to_string())
        });
        methods.document("Evaluate whether the given package satisfies this package requirement.");
        methods.param("package", "package spec to check");
        methods.add_method("matches", |_, this, package: PackageSpecLua| {
            Ok(this.0.matches(&package.0))
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("A lua package requirement with a name and an optional version requirement");
    }
}

impl mlua::UserData for PackageReqLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct LocalPackageHashesLua(pub LocalPackageHashes);

impl Typed for LocalPackageHashesLua {
    fn ty() -> Type {
        Type::named("LocalPackageHashes")
    }
}

impl_from_lua_userdata!(LocalPackageHashesLua);

impl TypedUserData for LocalPackageHashesLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.param("rockspec", "sha256sum of the rockspec");
        methods.add_method("rockspec", |_, this, ()| Ok(this.0.rockspec.to_hex().1));
        methods.param("source", "sha256sum of the package source");
        methods.add_method("source", |_, this, ()| Ok(this.0.source.to_hex().1));
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Rockspec and source integrities of an installed rock");
    }
}

impl mlua::UserData for LocalPackageHashesLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct LocalPackageLua(pub LocalPackage);

impl Typed for LocalPackageLua {
    fn ty() -> Type {
        Type::named("LocalPackage")
    }
}

impl FromLua for LocalPackageLua {
    fn from_lua(value: LuaValue, _lua: &Lua) -> LuaResult<Self> {
        match value {
            LuaValue::UserData(ud) => Ok(ud.borrow::<LocalPackageLua>()?.clone()),
            v => Err(LuaError::FromLuaConversionError {
                from: v.type_name(),
                to: "LocalPackageLua".to_string(),
                message: None,
            }),
        }
    }
}

impl TypedUserData for LocalPackageLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_method("id", |_, this, ()| Ok(LocalPackageIdLua(this.0.id())));
        methods.add_method("name", |_, this, ()| {
            Ok(PackageNameLua(this.0.name().clone()))
        });
        methods.add_method("version", |_, this, ()| {
            Ok(PackageVersionLua(this.0.version().clone()))
        });
        methods.add_method("pinned", |_, this, ()| Ok(PinnedStateLua(this.0.pinned())));
        methods.add_method("dependencies", |_, this, ()| {
            Ok(this
                .0
                .dependencies()
                .into_iter()
                .map(|id| LocalPackageIdLua(id.clone()))
                .collect::<Vec<_>>())
        });
        methods.add_method("constraint", |_, this, ()| {
            Ok(LockConstraintLua(this.0.constraint()))
        });
        methods.add_method("hashes", |_, this, ()| {
            Ok(LocalPackageHashesLua(this.0.hashes().clone()))
        });
        methods.add_method("to_package", |_, this, ()| {
            Ok(PackageSpecLua(this.0.to_package()))
        });
        methods.add_method("to_package_req", |_, this, ()| {
            Ok(PackageReqLua(this.0.clone().into_package_req()))
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("A locally installed rock");
    }
}

impl mlua::UserData for LocalPackageLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

pub struct RockLayoutLua(pub RockLayout);

impl Typed for RockLayoutLua {
    fn ty() -> Type {
        Type::named("RockLayout")
    }
}

impl TypedUserData for RockLayoutLua {
    fn add_fields<F: TypedDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("rock_path", |_, this| {
            Ok(this.0.rock_path.to_slash_lossy().into_owned())
        });
        fields.add_field_method_get("etc", |_, this| {
            Ok(this.0.etc.to_slash_lossy().into_owned())
        });
        fields.add_field_method_get("lib", |_, this| {
            Ok(this.0.lib.to_slash_lossy().into_owned())
        });
        fields.add_field_method_get("src", |_, this| {
            Ok(this.0.src.to_slash_lossy().into_owned())
        });
        fields.add_field_method_get("bin", |_, this| {
            Ok(this.0.bin.to_slash_lossy().into_owned())
        });
        fields.add_field_method_get("conf", |_, this| {
            Ok(this.0.conf.to_slash_lossy().into_owned())
        });
        fields.add_field_method_get("doc", |_, this| {
            Ok(this.0.doc.to_slash_lossy().into_owned())
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Change-agnostic way of referencing various paths for a rock");
    }
}

impl mlua::UserData for RockLayoutLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct TreeLua(pub Tree);

impl Typed for TreeLua {
    fn ty() -> Type {
        Type::named("Tree")
    }
}

impl_from_lua_userdata!(TreeLua);

impl TypedUserData for TreeLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("The root directory of the tree");
        methods.add_method("root", |_, this, ()| {
            Ok(this.0.root().to_slash_lossy().into_owned())
        });
        methods.document("The root directory of a package in this tree");
        methods.param("package", "");
        methods.add_method("root_for", |_, this, package: LocalPackageLua| {
            Ok(this.0.root_for(&package.0).to_slash_lossy().into_owned())
        });
        methods.document("Where wrapped package binaries are installed");
        methods.add_method("bin", |_, this, ()| {
            Ok(this.0.bin().to_slash_lossy().into_owned())
        });
        methods.document("Find installed rocks that match the given `PackageReq`");
        methods.param("req", "");
        methods.add_method("match_rocks", |lua, this, req: PackageReqLua| {
            this.0
                .match_rocks(&req.0)
                .map(|m| RockMatchesLua(m).into_lua(lua))
                .map_err(|err| LuaError::RuntimeError(err.to_string()))?
        });
        methods.document("Get the `RockLayout` for an installed package.");
        methods.param("package", "");
        methods.add_method("rock_layout", |_, this, package: LocalPackageLua| {
            this.0
                .installed_rock_layout(&package.0)
                .map(RockLayoutLua)
                .map_err(|err| LuaError::RuntimeError(err.to_string()))
        });
        methods.document("Create a `LockfileReadOnly` for this tree.");
        methods.add_method("lockfile", |_, this, ()| {
            this.0.lockfile().map(LockfileReadOnlyLua).into_lua_err()
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("A collection of files where installed rocks are located");
    }
}

impl mlua::UserData for TreeLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Deserialize_enum_str, Serialize_enum_str, Default)]
#[serde(rename_all = "snake_case")]
enum RockLayoutVariant {
    #[default]
    Default,
    Nvim,
}

#[derive(Deserialize)]
struct RockLayoutConfigInput {
    layout: RockLayoutVariant,
}

#[derive(Debug, Clone)]
pub struct RockLayoutConfigLua(pub RockLayoutConfig);

impl Typed for RockLayoutConfigLua {
    fn ty() -> Type {
        Type::named("RockLayoutConfig")
    }
}

impl FromLua for RockLayoutConfigLua {
    fn from_lua(value: LuaValue, lua: &Lua) -> LuaResult<Self> {
        let input: RockLayoutConfigInput = lua.from_value(value)?;
        Ok(match input.layout {
            RockLayoutVariant::Default => RockLayoutConfigLua(RockLayoutConfig::default()),
            RockLayoutVariant::Nvim => RockLayoutConfigLua(RockLayoutConfig::new_nvim_layout()),
        })
    }
}

impl TypedUserData for RockLayoutConfigLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("Instantiate the default rock layout");
        methods.add_function("new", |_, ()| {
            Ok(RockLayoutConfigLua(RockLayoutConfig::default()))
        });
        methods.document("Instantiate the a rock layout for Neovim plugins");
        methods.add_function("new_nvim_layout", |_, ()| {
            Ok(RockLayoutConfigLua(RockLayoutConfig::new_nvim_layout()))
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Template configuration for a rock's tree layout");
    }
}

impl mlua::UserData for RockLayoutConfigLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct ConfigLua(pub Config);

impl Typed for ConfigLua {
    fn ty() -> Type {
        Type::named("Config")
    }
}

impl FromLua for ConfigLua {
    fn from_lua(value: LuaValue, _lua: &Lua) -> LuaResult<Self> {
        match value {
            LuaValue::UserData(ud) => Ok(ud.borrow::<ConfigLua>()?.clone()),
            v => Err(LuaError::FromLuaConversionError {
                from: v.type_name(),
                to: "ConfigLua".to_string(),
                message: None,
            }),
        }
    }
}

impl TypedUserData for ConfigLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_function("builder", |_, ()| {
            ConfigBuilder::new().map(ConfigBuilderLua).into_lua_err()
        });
        methods.document("The luarocks repository server");
        methods.add_method("server", |_, this, ()| Ok(this.0.server().to_string()));
        methods.document("Additional luarocks repository servers");
        methods.add_method("extra_servers", |_, this, ()| {
            Ok(this
                .0
                .extra_servers()
                .iter()
                .map(|url| url.to_string())
                .collect_vec())
        });

        methods.document("The luarocks server namespace to use");
        methods.add_method("namespace", |_, this, ()| Ok(this.0.namespace().cloned()));

        methods.document("The directory in which to install Lua{n} if not found");
        methods.add_method("lua_dir", |_, this, ()| {
            Ok(this.0.lua_dir().map(|p| p.to_slash_lossy().into_owned()))
        });

        methods.document(
            r#"The tree in which to install rocks.
If installing packages for a project, use `project:tree(config)` instead"#,
        );
        methods.param("lua_version", "");
        methods.add_method("user_tree", |_, this, lua_version: LuaVersionLua| {
            this.0.user_tree(lua_version.0).map(TreeLua).into_lua_err()
        });

        methods.document("Whether to display verbose output of commands executed");
        methods.add_method("verbose", |_, this, ()| Ok(this.0.verbose()));

        methods.document("Whether to disable printing progress bars and spinners");
        methods.add_method("no_progress", |_, this, ()| Ok(this.0.no_progress()));

        methods.document("Whether to skip prompts, selecting the default option");
        methods.add_method("no_prompt", |_, this, ()| Ok(this.0.no_prompt()));

        methods.document(
            r#"Timeout on network operations, in seconds.
0 means no timeout (wait forever)."#,
        );
        methods.add_method("timeout", |_, this, ()| Ok(this.0.timeout().as_secs()));

        methods.document("The Lux cache directory");
        methods.add_method("cache_dir", |_, this, ()| {
            Ok(this.0.cache_dir().to_slash_lossy().into_owned())
        });

        methods.document("The Lux data directory");
        methods.add_method("data_dir", |_, this, ()| {
            Ok(this.0.data_dir().to_slash_lossy().into_owned())
        });

        methods.document(
            r#"The rock layout for entrypoints of new install trees.
Does not affect existing install trees or dependency rock layouts."#,
        );
        methods.add_method("entrypoint_layout", |_, this, ()| {
            Ok(RockLayoutConfigLua(this.0.entrypoint_layout().clone()))
        });

        methods.document(
            r#"Variable names, mapped to their values.
Lux populates variables in the `lux.toml` and in RockSpecs
with these before building."#,
        );
        methods.add_method("variables", |_, this, ()| Ok(this.0.variables().clone()));

        methods.document("Command to use for running `make` builds");
        methods.add_method("make_cmd", |_, this, ()| Ok(this.0.make_cmd()));

        methods.document("Command to use for running `cmake` builds");
        methods.add_method("cmake_cmd", |_, this, ()| Ok(this.0.cmake_cmd()));

        methods.document("Enabled luarocks repository servers that provide dev/scm rocks");
        methods.add_method("enabled_dev_servers", |_, this, ()| {
            Ok(this
                .0
                .enabled_dev_servers()
                .into_lua_err()?
                .into_iter()
                .map(|url| url.to_string())
                .collect_vec())
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add(
            r#"The resolved configuration for a Lux session.
Can be constructed via `ConfigBuilder`, which supports layering multiple
configuration sources (config file, CLI flags, environment variables)
        "#,
        );
    }
}

impl mlua::UserData for ConfigLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Clone)]
pub struct ConfigBuilderLua(pub ConfigBuilder);

impl Typed for ConfigBuilderLua {
    fn ty() -> Type {
        Type::named("ConfigBuilder")
    }
}

impl TypedUserData for ConfigBuilderLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("Whether to development packages");
        methods.param("dev", "Default: false");
        methods.add_method("dev", |_, this, dev: Option<bool>| {
            Ok(ConfigBuilderLua(this.0.clone().dev(dev)))
        });

        methods.document("Fetch rocks/rockspecs from this luarocks server");
        methods.param("server", "Default: 'https://luarocks.org/'");

        methods.add_method("server", |_, this, server: Option<LuaUrl>| {
            Ok(ConfigBuilderLua(
                this.0.clone().server(server.map(|url| url.0)),
            ))
        });

        methods.document("Fetch rocks/rockspecs from these servers in addition to the main server");
        methods.param("servers", "List of server URLs");
        methods.add_method("extra_servers", |_, this, servers: Option<Vec<LuaUrl>>| {
            Ok(ConfigBuilderLua(this.0.clone().extra_servers(
                servers.map(|urls| urls.into_iter().map(|url| url.0).collect()),
            )))
        });

        methods.document("Specify the luarocks server namespace to use");
        methods.param("namespace", "");
        methods.add_method("namespace", |_, this, namespace: Option<String>| {
            Ok(ConfigBuilderLua(this.0.clone().namespace(namespace)))
        });

        methods.document("Specify the directory in which to install Lua if not found");
        methods.param("lua_dir", "<path>");
        methods.add_method("lua_dir", |_, this, lua_dir: Option<String>| {
            Ok(ConfigBuilderLua(
                this.0.clone().lua_dir(lua_dir.map(PathBuf::from)),
            ))
        });

        methods.document("Which Lua version to use");
        methods.param(
            "lua_version",
            "Default: The installed Lua version, if detected",
        );
        methods.add_method(
            "lua_version",
            |_, this, lua_version: Option<LuaVersionLua>| {
                Ok(ConfigBuilderLua(
                    this.0.clone().lua_version(lua_version.map(|v| v.0)),
                ))
            },
        );

        methods.document("Which tree to operate on");
        methods.param("tree", "Tree root directory");
        methods.add_method("user_tree", |_, this, tree: Option<String>| {
            Ok(ConfigBuilderLua(
                this.0.clone().user_tree(tree.map(PathBuf::from)),
            ))
        });

        methods.document("Whether to display verbose output of commands executed");
        methods.param("verbose", "Default: false");
        methods.add_method("verbose", |_, this, verbose: Option<bool>| {
            Ok(ConfigBuilderLua(this.0.clone().verbose(verbose)))
        });

        methods.document("Whether to disable printing progress bars and spinners");
        methods.param("no_progress", "Default: false");
        methods.add_method("no_progress", |_, this, no_progress: Option<bool>| {
            Ok(ConfigBuilderLua(this.0.clone().no_progress(no_progress)))
        });

        methods.document("Whether to disable user prompts");
        methods.param("no_progress", "Default: false");
        methods.add_method("no_prompt", |_, this, no_prompt: Option<bool>| {
            Ok(ConfigBuilderLua(this.0.clone().no_prompt(no_prompt)))
        });

        methods.document(
            r#"Timeout on network operations, in seconds.
0 means no timeout (wait forever)."#,
        );
        methods.param("timeout", "Default: 30 s");
        methods.add_method("timeout", |_, this, timeout: Option<u64>| {
            Ok(ConfigBuilderLua(
                this.0.clone().timeout(timeout.map(Duration::from_secs)),
            ))
        });

        methods.document("The cache directory, e.g. for luarocks manifests.");
        methods.param("cache_dir", "");
        methods.add_method("cache_dir", |_, this, cache_dir: Option<String>| {
            Ok(ConfigBuilderLua(
                this.0.clone().cache_dir(cache_dir.map(PathBuf::from)),
            ))
        });

        methods.document("The data directory, in which the default user install tree resides.");
        methods.param("data_dir", "");
        methods.add_method("data_dir", |_, this, data_dir: Option<String>| {
            Ok(ConfigBuilderLua(
                this.0.clone().data_dir(data_dir.map(PathBuf::from)),
            ))
        });

        methods.document(
            r#"The rock layout for entrypoints of new install trees.
Does not affect existing install trees or dependency rock layouts."#,
        );
        methods.param("layout", "");
        methods.add_method(
            "entrypoint_layout",
            |_, this, layout: Option<RockLayoutConfigLua>| {
                Ok(ConfigBuilderLua(this.0.clone().entrypoint_layout(
                    layout.map(|l| l.0).unwrap_or_default(),
                )))
            },
        );

        methods.document("The user agent to set when making web requests.");
        methods.param("user_agent", "Default: 'lux-lua/<version>'");
        methods.add_method("user_agent", |_, this, user_agent: Option<String>| {
            Ok(ConfigBuilderLua(this.0.clone().user_agent(user_agent)))
        });

        methods.document("Whether to generate a `.luarc.json` on build.");
        methods.param("generate", "Default: true");
        methods.add_method("generate_luarc", |_, this, generate: Option<bool>| {
            Ok(ConfigBuilderLua(this.0.clone().generate_luarc(generate)))
        });

        methods.document(
            r#"Whether to wrap installed Lua bin scripts to be executed with
the detected or configured Lua installation.
Setting this to `false` disables wrapping globally.
If set to `true`, individual rocks can still disable wrapping of their own bin scripts.
        "#,
        );
        methods.param("wrap", "Default: true");
        methods.add_method("wrap_bin_scripts", |_, this, wrap: Option<bool>| {
            Ok(ConfigBuilderLua(this.0.clone().wrap_bin_scripts(wrap)))
        });
        methods.add_method("build", |_, this, ()| {
            this.0.clone().build().map(ConfigLua).into_lua_err()
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Incrementally builds a `Config` by layering configuration sources.");
    }
}

impl mlua::UserData for ConfigBuilderLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct LuaDependencySpecLua(pub LuaDependencySpec);

impl Typed for LuaDependencySpecLua {
    fn ty() -> Type {
        Type::named("LuaDependencySpec")
    }
}

impl FromLua for LuaDependencySpecLua {
    fn from_lua(value: LuaValue, lua: &Lua) -> LuaResult<Self> {
        match value {
            LuaValue::UserData(ref ud) => {
                if let Ok(borrowed) = ud.borrow::<LuaDependencySpecLua>() {
                    return Ok(borrowed.clone());
                }

                let s: String = lua.from_value(value)?;
                s.parse::<LuaDependencySpec>()
                    .map(LuaDependencySpecLua)
                    .into_lua_err()
            }
            _ => {
                let package_req: PackageReq = lua.from_value(value)?;
                Ok(LuaDependencySpecLua(LuaDependencySpec::from(package_req)))
            }
        }
    }
}

impl TypedUserData for LuaDependencySpecLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_method("name", |_, this, ()| Ok(this.0.name().to_string()));
        methods.add_method("version_req", |_, this, ()| {
            Ok(this.0.version_req().to_string())
        });
        methods.document(
            "Evaluate whether the given package satisfies this dependency's requirement.",
        );
        methods.param("package", "package spec to check");
        methods.add_method("matches", |_, this, package: PackageSpecLua| {
            Ok(this.0.matches(&package.0))
        });
        methods.add_method("package_req", |_, this, ()| {
            Ok(PackageReqLua(this.0.package_req().clone()))
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Specification for a Lua dependency in a Lux project");
    }
}

impl mlua::UserData for LuaDependencySpecLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct PlatformSupportLua(pub PlatformSupport);

impl Typed for PlatformSupportLua {
    fn ty() -> Type {
        Type::named("PlatformSupport")
    }
}

impl TypedUserData for PlatformSupportLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("Is the given platform supported?");
        methods.param("platform", "");
        methods.add_method(
            "is_supported",
            |_, this, platform: PlatformIdentifierLua| Ok(this.0.is_supported(&platform.0)),
        );
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Used to specify which platforms a rock can be built for");
    }
}

impl mlua::UserData for PlatformSupportLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct RockDescriptionLua(pub RockDescription);

impl Typed for RockDescriptionLua {
    fn ty() -> Type {
        Type::named("RockDescription")
    }
}

impl TypedUserData for RockDescriptionLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("A one-line description of the package");
        methods.add_method("summary", |_, this, ()| Ok(this.0.summary.clone()));

        methods.document("A longer description of the package");
        methods.add_method("detailed", |_, this, ()| Ok(this.0.detailed.clone()));

        methods.document("The license used by the package");
        methods.add_method("license", |_, this, ()| Ok(this.0.license.clone()));

        methods.document("An URL for the project. This is not the URL for the tarball, but the address of a website");
        methods.add_method("homepage", |_, this, ()| {
            Ok(this.0.homepage.as_ref().map(|url| url.to_string()))
        });

        methods.document("An URL for the project's issue tracker");
        methods.add_method("issues_url", |_, this, ()| {
            Ok(this.0.issues_url.as_ref().map(|url| url.to_string()))
        });

        methods.document("Contact information for the rockspec maintainer");
        methods.add_method("maintainer", |_, this, ()| Ok(this.0.maintainer.clone()));

        methods.document(
            "A list of short strings that specify labels for categorization of this rock",
        );
        methods.add_method("labels", |_, this, ()| Ok(this.0.labels.clone()));
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("A rock's metadata, to be displayed on the remote package server");
    }
}

impl mlua::UserData for RockDescriptionLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct GitSourceLua(pub GitSource);

impl Typed for GitSourceLua {
    fn ty() -> Type {
        Type::named("GitSource")
    }
}

impl TypedUserData for GitSourceLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_method("url", |_, this, ()| Ok(this.0.url.to_string()));
        methods.add_method("checkout_ref", |_, this, ()| {
            Ok(this.0.checkout_ref.clone())
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Specifies a source to be fetched from a git forge");
    }
}

impl mlua::UserData for GitSourceLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct RemoteRockSourceLua(pub RemoteRockSource);

impl Typed for RemoteRockSourceLua {
    fn ty() -> Type {
        Type::named("RemoteRockSource")
    }
}

impl TypedUserData for RemoteRockSourceLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_method("source_spec", |_, this, ()| {
            Ok(RockSourceSpecLua(this.0.source_spec.clone()))
        });
        methods.add_method("archive_name", |_, this, ()| {
            Ok(this
                .0
                .archive_name
                .as_ref()
                .map(|p| p.to_slash_lossy().into_owned()))
        });
        methods.add_method("unpack_dir", |_, this, ()| {
            Ok(this
                .0
                .unpack_dir
                .as_ref()
                .map(|p| p.to_slash_lossy().into_owned()))
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Specifies the source of a remote rock to be fetched");
    }
}

impl mlua::UserData for RemoteRockSourceLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct BustedTestSpecLua(pub BustedTestSpec);

impl Typed for BustedTestSpecLua {
    fn ty() -> Type {
        Type::named("BustedTestSpec")
    }
}

impl TypedUserData for BustedTestSpecLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("Additional CLI flags to pass to busted when running");
        methods.add_method("flags", |_, this, ()| Ok(this.0.flags().clone()));
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Specification for running a test suite with busted");
    }
}

impl mlua::UserData for BustedTestSpecLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct CommandTestSpecLua(pub CommandTestSpec);

impl Typed for CommandTestSpecLua {
    fn ty() -> Type {
        Type::named("CommandTestSpec")
    }
}

impl TypedUserData for CommandTestSpecLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("The command to run");
        methods.add_method("command", |_, this, ()| Ok(this.0.command().to_string()));

        methods.document("Additional CLI flags to pass to the command when running");
        methods.add_method("flags", |_, this, ()| Ok(this.0.flags().clone()));
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Specification for running a test suite with a command");
    }
}

impl mlua::UserData for CommandTestSpecLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct LuaScriptTestSpecLua(pub LuaScriptTestSpec);

impl Typed for LuaScriptTestSpecLua {
    fn ty() -> Type {
        Type::named("LuaScriptTestSpec")
    }
}

impl TypedUserData for LuaScriptTestSpecLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("The script to run");
        methods.add_method("script", |_, this, ()| {
            Ok(this.0.script().to_slash_lossy().into_owned())
        });

        methods.document("Additional CLI flags to pass to the script when running");
        methods.add_method("flags", |_, this, ()| Ok(this.0.flags().clone()));
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Specification for running a test suite with a Lua script");
    }
}

impl mlua::UserData for LuaScriptTestSpecLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct ModulePathsLua(pub ModulePaths);

impl Typed for ModulePathsLua {
    fn ty() -> Type {
        Type::named("ModulePaths")
    }
}

impl TypedUserData for ModulePathsLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("Path names of C sources, mandatory field");
        methods.add_method("sources", |_, this, ()| {
            Ok(this
                .0
                .sources
                .iter()
                .map(|p| p.to_slash_lossy().into_owned())
                .collect::<Vec<_>>())
        });

        methods.document("External libraries to be linked");
        methods.add_method("libraries", |_, this, ()| {
            Ok(this
                .0
                .libraries
                .iter()
                .map(|p| p.to_slash_lossy().into_owned())
                .collect::<Vec<_>>())
        });

        methods.document("C defines, e.g. { 'FOO=bar', 'USE_BLA' }");
        methods.add_method("defines", |_, this, ()| {
            Ok(this
                .0
                .defines
                .iter()
                .cloned()
                .collect::<HashMap<_, Option<_>>>())
        });

        methods
            .document("Directories to be added to the compiler's headers lookup directory list.");
        methods.add_method("incdirs", |_, this, ()| {
            Ok(this
                .0
                .incdirs
                .iter()
                .map(|p| p.to_slash_lossy().into_owned())
                .collect::<Vec<_>>())
        });

        methods.document("Directories to be added to the linker's library lookup directory list.");
        methods.add_method("libdirs", |_, this, ()| {
            Ok(this
                .0
                .libdirs
                .iter()
                .map(|p| p.to_slash_lossy().into_owned())
                .collect::<Vec<_>>())
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Specification for building a Lua module from various sources");
    }
}

impl mlua::UserData for ModulePathsLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct CMakeBuildSpecLua(pub CMakeBuildSpec);

impl Typed for CMakeBuildSpecLua {
    fn ty() -> Type {
        Type::named("CMakeBuildSpec")
    }
}

impl TypedUserData for CMakeBuildSpecLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_method("cmake_lists_content", |_, this, ()| {
            Ok(this.0.cmake_lists_content.clone())
        });

        methods.document("Whether to perform a build pass");
        methods.add_method("build_pass", |_, this, ()| Ok(this.0.build_pass));

        methods.document("Whether to perform an install pass");
        methods.add_method("install_pass", |_, this, ()| Ok(this.0.install_pass));

        methods.add_method("variables", |_, this, ()| Ok(this.0.variables.clone()));
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Specification for building a rock with the `cmake` build backend");
    }
}

impl mlua::UserData for CMakeBuildSpecLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct MakeBuildSpecLua(pub MakeBuildSpec);

impl Typed for MakeBuildSpecLua {
    fn ty() -> Type {
        Type::named("MakeBuildSpec")
    }
}

impl TypedUserData for MakeBuildSpecLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("Makefile to be used");
        methods.add_method("makefile", |_, this, ()| {
            Ok(this.0.makefile.to_slash_lossy().into_owned())
        });

        methods.add_method("build_target", |_, this, ()| {
            Ok(this.0.build_target.clone())
        });

        methods
            .document("Whether to perform a make pass on the target indicated by `build_target`");
        methods.add_method("build_pass", |_, this, ()| Ok(this.0.build_pass));

        methods.add_method("install_target", |_, this, ()| {
            Ok(this.0.install_target.clone())
        });

        methods
            .document("Whether to perform a make pass on the target indicated by `install_target`");
        methods.add_method("install_pass", |_, this, ()| Ok(this.0.install_pass));

        methods.document("Assignments to be passed to make during the build pass");
        methods.add_method("build_variables", |_, this, ()| {
            Ok(this.0.build_variables.clone())
        });

        methods.document("Assignments to be passed to make during the install pass");
        methods.add_method("install_variables", |_, this, ()| {
            Ok(this.0.install_variables.clone())
        });

        methods.document("Assignments to be passed to make during both passes");
        methods.add_method("variables", |_, this, ()| Ok(this.0.variables.clone()));
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Specification for building a rock with the `make` build backend");
    }
}

impl mlua::UserData for MakeBuildSpecLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct TreesitterParserBuildSpecLua(pub TreesitterParserBuildSpec);

impl Typed for TreesitterParserBuildSpecLua {
    fn ty() -> Type {
        Type::named("TreesitterParserBuildSpec")
    }
}

impl TypedUserData for TreesitterParserBuildSpecLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("Name of the parser language, e.g. 'haskell'");
        methods.add_method("lang", |_, this, ()| Ok(this.0.lang.clone()));

        methods.document("Won't build the parser if `false`");
        methods.add_method("parser", |_, this, ()| Ok(this.0.parser));

        methods.document("Must the sources be generated?");
        methods.add_method("generate", |_, this, ()| Ok(this.0.generate));

        methods.document("tree-sitter grammar's location (relative to the source root)");
        methods.add_method("location", |_, this, ()| {
            Ok(this
                .0
                .location
                .as_ref()
                .map(|p| p.to_slash_lossy().into_owned()))
        });

        methods.document("Embedded queries to be installed in the `etc/queries` directory");
        methods.add_method("queries", |_, this, ()| {
            Ok(this
                .0
                .queries
                .iter()
                .map(|(k, v)| (k.to_slash_lossy().into_owned(), v.clone()))
                .collect::<HashMap<_, _>>())
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Specification for building a rock with the `treesitter-parser` build backend");
    }
}

impl mlua::UserData for TreesitterParserBuildSpecLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct RustMluaBuildSpecLua(pub RustMluaBuildSpec);

impl Typed for RustMluaBuildSpecLua {
    fn ty() -> Type {
        Type::named("RustMluaBuildSpec")
    }
}

impl TypedUserData for RustMluaBuildSpecLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document(
            r#"Keys are module names in the format normally used by the `require()` function.
values are the library names in the target directory (without the `lib` prefix).
        "#,
        );
        methods.add_method("modules", |_, this, ()| {
            Ok(this
                .0
                .modules
                .iter()
                .map(|(k, v)| (k.clone(), v.to_slash_lossy().into_owned()))
                .collect::<HashMap<_, _>>())
        });

        methods.document("Set if the cargo `target` directory is not in the source root");
        methods.add_method("target_path", |_, this, ()| {
            Ok(this.0.target_path.to_slash_lossy().into_owned())
        });

        methods.document("If set to `false` pass `--no-default-features` to cargo.");
        methods.add_method("default_features", |_, this, ()| {
            Ok(this.0.default_features)
        });

        methods.document("Pass additional features");
        methods.add_method("features", |_, this, ()| Ok(this.0.features.clone()));

        methods.document("Additional flags to be passed in the cargo invocation");
        methods.add_method("cargo_extra_args", |_, this, ()| {
            Ok(this.0.cargo_extra_args.clone())
        });

        methods.document(
            r#"Copy additional files to the `lua` directory.
Keys are the sources, values the destinations (relative to the `lua` directory).
        "#,
        );
        methods.add_method("include", |_, this, ()| {
            Ok(this
                .0
                .include
                .iter()
                .map(|(k, v)| {
                    (
                        k.to_slash_lossy().into_owned(),
                        v.to_slash_lossy().into_owned(),
                    )
                })
                .collect::<HashMap<_, _>>())
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Specification for building a rock with the `rust-mlua` build backend");
    }
}

impl mlua::UserData for RustMluaBuildSpecLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct CommandBuildSpecLua(pub CommandBuildSpec);

impl Typed for CommandBuildSpecLua {
    fn ty() -> Type {
        Type::named("CommandBuildSpec")
    }
}

impl TypedUserData for CommandBuildSpecLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_method("build_command", |_, this, ()| {
            Ok(this.0.build_command.clone())
        });
        methods.add_method("install_command", |_, this, ()| {
            Ok(this.0.install_command.clone())
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Specification for building a rock with the `command` build backend");
    }
}

impl mlua::UserData for CommandBuildSpecLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct InstallSpecLua(pub InstallSpec);

impl Typed for InstallSpecLua {
    fn ty() -> Type {
        Type::named("InstallSpec")
    }
}

impl TypedUserData for InstallSpecLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("Lua modules written in Lua");
        methods.add_method("lua", |_, this, ()| {
            Ok(this
                .0
                .lua
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), v.to_slash_lossy().into_owned()))
                .collect::<HashMap<String, String>>())
        });

        methods.document("Dynamic libraries implemented compiled Lua modules");
        methods.add_method("lib", |_, this, ()| {
            Ok(this
                .0
                .lib
                .iter()
                .map(|(k, v)| (k.clone(), v.to_slash_lossy().into_owned()))
                .collect::<HashMap<String, String>>())
        });

        methods.document("Configuration files");
        methods.add_method("conf", |_, this, ()| {
            Ok(this
                .0
                .conf
                .iter()
                .map(|(k, v)| (k.clone(), v.to_slash_lossy().into_owned()))
                .collect::<HashMap<String, String>>())
        });

        methods.document("Lua command-line scripts");
        methods.add_method("bin", |_, this, ()| {
            Ok(this
                .0
                .bin
                .iter()
                .map(|(k, v)| (k.clone(), v.to_slash_lossy().into_owned()))
                .collect::<HashMap<String, String>>())
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add(
            r#"For packages which don't provide means to install modules
and expect the user to copy the .lua or library files by hand to the proper locations.
This struct contains categories of files. Each category is itself a table,
where the array part is a list of filenames to be copied.
For module directories only, in the hash part, other keys are identifiers in Lua module format,
to indicate which subdirectory the file should be copied to.
For example, lua = {["foo.bar"] = "src/bar.lua"} will copy src/bar.lua
to the foo directory under the rock's Lua files directory.
        "#,
        );
    }
}

impl mlua::UserData for InstallSpecLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct BuildSpecLua(pub BuildSpec);

impl Typed for BuildSpecLua {
    fn ty() -> Type {
        Type::named("BuildSpec")
    }
}

impl TypedUserData for BuildSpecLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("Determines the build backend to use");
        methods.add_method("build_backend", |_, this, ()| {
            Ok(this.0.build_backend.clone().map(BuildBackendSpecLua))
        });

        methods.document("A set of instructions on how/where to copy files from the project");
        methods.add_method("install", |_, this, ()| {
            Ok(InstallSpecLua(this.0.install.clone()))
        });

        methods
            .document("A list of directories that should be copied as-is into the resulting rock");
        methods.add_method("copy_directories", |_, this, ()| {
            Ok(this
                .0
                .copy_directories
                .iter()
                .map(|p| p.to_slash_lossy().into_owned())
                .collect::<Vec<_>>())
        });

        methods.document("A list of patches to apply to the project before packaging it");
        methods.add_method("patches", |_, this, ()| {
            Ok(this
                .0
                .patches
                .iter()
                .map(|(k, v)| (k.to_slash_lossy().into_owned(), v.clone()))
                .collect::<HashMap<String, String>>())
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("The build specification for a given rock, serialized from `build = { ... }`.");
    }
}

impl mlua::UserData for BuildSpecLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct LocalLuaRockspecLua(pub LocalLuaRockspec);

impl Typed for LocalLuaRockspecLua {
    fn ty() -> Type {
        Type::named("LocalLuaRockspec")
    }
}

impl TypedUserData for LocalLuaRockspecLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_method("package", |_, this, ()| {
            Ok(PackageNameLua(this.0.package().clone()))
        });
        methods.add_method("version", |_, this, ()| {
            Ok(PackageVersionLua(this.0.version().clone()))
        });
        methods.add_method("description", |_, this, ()| {
            Ok(RockDescriptionLua(this.0.description().clone()))
        });
        methods.add_method("supported_platforms", |_, this, ()| {
            Ok(PlatformSupportLua(this.0.supported_platforms().clone()))
        });
        methods.add_method("lua", |_, this, ()| {
            Ok(PackageVersionReqLua(this.0.lua().clone()))
        });
        methods.add_method("dependencies", |_, this, ()| {
            Ok(this
                .0
                .dependencies()
                .current_platform()
                .iter()
                .map(|d| LuaDependencySpecLua(d.clone()))
                .collect::<Vec<_>>())
        });
        methods.add_method("build_dependencies", |_, this, ()| {
            Ok(this
                .0
                .build_dependencies()
                .current_platform()
                .iter()
                .map(|d| LuaDependencySpecLua(d.clone()))
                .collect::<Vec<_>>())
        });
        methods.add_method("test_dependencies", |_, this, ()| {
            Ok(this
                .0
                .test_dependencies()
                .current_platform()
                .iter()
                .map(|d| LuaDependencySpecLua(d.clone()))
                .collect::<Vec<_>>())
        });
        methods.add_method("external_dependencies", |_, this, ()| {
            Ok(this
                .0
                .external_dependencies()
                .current_platform()
                .iter()
                .map(|(k, v)| (k.clone(), ExternalDependencySpecLua(v.clone())))
                .collect::<HashMap<_, _>>())
        });
        methods.add_method("build", |_, this, ()| {
            Ok(BuildSpecLua(this.0.build().current_platform().clone()))
        });
        methods.add_method("source", |_, this, ()| {
            Ok(RemoteRockSourceLua(
                this.0.source().current_platform().clone(),
            ))
        });
        methods.add_method("test", |_, this, ()| {
            Ok(TestSpecLua(this.0.test().current_platform().clone()))
        });
        methods.add_method("format", |_, this, ()| {
            Ok(this.0.format().clone().map(RockspecFormatLua))
        });
        methods.add_method("to_lua_rockspec_string", |_, this, ()| {
            Ok(this
                .0
                .to_lua_remote_rockspec_string()
                .unwrap_or_else(|e| match e {}))
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("RockSpec for a local rock installation, deserialized from a `.rockspec` file");
    }
}

impl mlua::UserData for LocalLuaRockspecLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct RemoteLuaRockspecLua(pub RemoteLuaRockspec);

impl Typed for RemoteLuaRockspecLua {
    fn ty() -> Type {
        Type::named("RemoteLuaRockspec")
    }
}

impl FromLua for RemoteLuaRockspecLua {
    fn from_lua(value: LuaValue, lua: &Lua) -> LuaResult<Self> {
        let content = String::from_lua(value, lua)?;
        RemoteLuaRockspec::new(&content)
            .map(RemoteLuaRockspecLua)
            .into_lua_err()
    }
}

impl TypedUserData for RemoteLuaRockspecLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_method("package", |_, this, ()| {
            Ok(PackageNameLua(this.0.package().clone()))
        });
        methods.add_method("version", |_, this, ()| {
            Ok(PackageVersionLua(this.0.version().clone()))
        });
        methods.add_method("description", |_, this, ()| {
            Ok(RockDescriptionLua(this.0.description().clone()))
        });
        methods.add_method("supported_platforms", |_, this, ()| {
            Ok(PlatformSupportLua(this.0.supported_platforms().clone()))
        });
        methods.add_method("lua", |_, this, ()| {
            Ok(PackageVersionReqLua(this.0.lua().clone()))
        });
        methods.add_method("dependencies", |_, this, ()| {
            Ok(this
                .0
                .dependencies()
                .current_platform()
                .iter()
                .map(|d| LuaDependencySpecLua(d.clone()))
                .collect::<Vec<_>>())
        });
        methods.add_method("build_dependencies", |_, this, ()| {
            Ok(this
                .0
                .build_dependencies()
                .current_platform()
                .iter()
                .map(|d| LuaDependencySpecLua(d.clone()))
                .collect::<Vec<_>>())
        });
        methods.add_method("test_dependencies", |_, this, ()| {
            Ok(this
                .0
                .test_dependencies()
                .current_platform()
                .iter()
                .map(|d| LuaDependencySpecLua(d.clone()))
                .collect::<Vec<_>>())
        });
        methods.add_method("external_dependencies", |_, this, ()| {
            Ok(this
                .0
                .external_dependencies()
                .current_platform()
                .iter()
                .map(|(k, v)| (k.clone(), ExternalDependencySpecLua(v.clone())))
                .collect::<HashMap<_, _>>())
        });
        methods.add_method("build", |_, this, ()| {
            Ok(BuildSpecLua(this.0.build().current_platform().clone()))
        });
        methods.add_method("source", |_, this, ()| {
            Ok(RemoteRockSourceLua(
                this.0.source().current_platform().clone(),
            ))
        });
        methods.add_method("test", |_, this, ()| {
            Ok(TestSpecLua(this.0.test().current_platform().clone()))
        });
        methods.add_method("format", |_, this, ()| {
            Ok(this.0.format().clone().map(RockspecFormatLua))
        });
        methods.add_method("to_lua_rockspec_string", |_, this, ()| {
            Ok(this
                .0
                .to_lua_remote_rockspec_string()
                .unwrap_or_else(|e| match e {}))
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("RockSpec for a remote rock, deserialized from a `.rockspec` file");
    }
}

impl mlua::UserData for RemoteLuaRockspecLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

pub struct PartialLuaRockspecLua(pub PartialLuaRockspec);

impl Typed for PartialLuaRockspecLua {
    fn ty() -> Type {
        Type::named("PartialLuaRockspec")
    }
}

impl FromLua for PartialLuaRockspecLua {
    fn from_lua(value: LuaValue, lua: &Lua) -> LuaResult<Self> {
        let content = String::from_lua(value, lua)?;
        PartialLuaRockspec::new(&content)
            .map(PartialLuaRockspecLua)
            .into_lua_err()
    }
}

impl TypedUserData for PartialLuaRockspecLua {
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Deserialized from a Lua `.rockspec`, not yet validated");
    }
}

impl mlua::UserData for PartialLuaRockspecLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct PartialProjectTomlLua(pub PartialProjectToml);

impl Typed for PartialProjectTomlLua {
    fn ty() -> Type {
        Type::named("PartialProjectToml")
    }
}

impl_from_lua_userdata!(PartialProjectTomlLua);

impl TypedUserData for PartialProjectTomlLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_method("package", |_, this, ()| {
            Ok(PackageNameLua(this.0.package().clone()))
        });
        methods.add_method("to_local", |_, this, ()| {
            this.0.into_local().map(LocalProjectTomlLua).into_lua_err()
        });

        methods.param("specrev", "The revision of the RockSpec");
        methods.add_method("to_remote", |_, this, specrev: Option<SpecRevLua>| {
            this.0
                .into_remote(specrev.map(|s| s.0))
                .map(RemoteProjectTomlLua)
                .into_lua_err()
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add(r#"The `lux.toml` file for a project.
The only required fields are `package` and `build`, which are required to build a project using `lux build`.
The rest of the fields are optional, but are required to build a rockspec.
"#);
    }
}

impl mlua::UserData for PartialProjectTomlLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

pub struct LocalProjectTomlLua(pub LocalProjectToml);

impl Typed for LocalProjectTomlLua {
    fn ty() -> Type {
        Type::named("LocalProjectToml")
    }
}

impl TypedUserData for LocalProjectTomlLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_method("package", |_, this, ()| {
            Ok(PackageNameLua(this.0.package().clone()))
        });
        methods.add_method("version", |_, this, ()| {
            Ok(PackageVersionLua(this.0.version().clone()))
        });
        methods.add_method("description", |_, this, ()| {
            Ok(RockDescriptionLua(this.0.description().clone()))
        });
        methods.add_method("dependencies", |_, this, ()| {
            Ok(this
                .0
                .dependencies()
                .current_platform()
                .iter()
                .map(|d| LuaDependencySpecLua(d.clone()))
                .collect::<Vec<_>>())
        });
        methods.add_method("build_dependencies", |_, this, ()| {
            Ok(this
                .0
                .build_dependencies()
                .current_platform()
                .iter()
                .map(|d| LuaDependencySpecLua(d.clone()))
                .collect::<Vec<_>>())
        });
        methods.add_method("test_dependencies", |_, this, ()| {
            Ok(this
                .0
                .test_dependencies()
                .current_platform()
                .iter()
                .map(|d| LuaDependencySpecLua(d.clone()))
                .collect::<Vec<_>>())
        });
        methods.add_method("build", |_, this, ()| {
            Ok(BuildSpecLua(this.0.build().current_platform().clone()))
        });
        methods.add_method("test", |_, this, ()| {
            Ok(TestSpecLua(this.0.test().current_platform().clone()))
        });
        methods.add_method("to_lua_rockspec", |_, this, ()| {
            this.0
                .to_lua_rockspec()
                .map(LocalLuaRockspecLua)
                .into_lua_err()
        });
        methods.add_method("to_lua_rockspec_string", |_, this, ()| {
            let rockspec = this.0.to_lua_rockspec().into_lua_err()?;
            Ok(rockspec
                .to_lua_remote_rockspec_string()
                .unwrap_or_else(|e| match e {}))
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add(
            r#"The `lux.toml` file, after being properly deserialized.
This struct may be used to build a local version of a project.
To build a rockspec, use `RemoteProjectToml`.
"#,
        );
    }
}

impl mlua::UserData for LocalProjectTomlLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

pub struct RemoteProjectTomlLua(pub RemoteProjectToml);

impl Typed for RemoteProjectTomlLua {
    fn ty() -> Type {
        Type::named("RemoteProjectToml")
    }
}

impl TypedUserData for RemoteProjectTomlLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_method("package", |_, this, ()| {
            Ok(PackageNameLua(this.0.package().clone()))
        });
        methods.add_method("version", |_, this, ()| {
            Ok(PackageVersionLua(this.0.version().clone()))
        });
        methods.add_method("description", |_, this, ()| {
            Ok(RockDescriptionLua(this.0.description().clone()))
        });
        methods.add_method("dependencies", |_, this, ()| {
            Ok(this
                .0
                .dependencies()
                .current_platform()
                .iter()
                .map(|d| LuaDependencySpecLua(d.clone()))
                .collect::<Vec<_>>())
        });
        methods.add_method("build", |_, this, ()| {
            Ok(BuildSpecLua(this.0.build().current_platform().clone()))
        });
        methods.add_method("source", |_, this, ()| {
            Ok(RemoteRockSourceLua(
                this.0.source().current_platform().clone(),
            ))
        });
        methods.add_method("to_lua_rockspec", |_, this, ()| {
            this.0
                .to_lua_rockspec()
                .map(RemoteLuaRockspecLua)
                .into_lua_err()
        });
        methods.add_method("to_lua_rockspec_string", |_, this, ()| {
            let rockspec = this.0.to_lua_rockspec().into_lua_err()?;
            Ok(rockspec
                .to_lua_remote_rockspec_string()
                .unwrap_or_else(|e| match e {}))
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("The `lux.toml`, after being validated and prepared for upload");
    }
}

impl mlua::UserData for RemoteProjectTomlLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct RemotePackageDBLua(pub RemotePackageDB);

impl Typed for RemotePackageDBLua {
    fn ty() -> Type {
        Type::named("RemotePackageDB")
    }
}

impl FromLua for RemotePackageDBLua {
    fn from_lua(value: LuaValue, _lua: &Lua) -> LuaResult<Self> {
        match value {
            LuaValue::UserData(ud) => Ok(ud.borrow::<RemotePackageDBLua>()?.clone()),
            v => Err(LuaError::FromLuaConversionError {
                from: v.type_name(),
                to: "RemotePackageDBLua".to_string(),
                message: None,
            }),
        }
    }
}

impl mlua::UserData for RemotePackageDBLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

impl TypedUserData for RemotePackageDBLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("Search for all packages that match the requirement");
        methods.param(
            "package_req",
            "Package to search for, e.g. 'foo' or 'foo >= 1.0.0'",
        );
        methods.add_method("search", |_, this, package_req: PackageReqLua| {
            let results: HashMap<String, Vec<String>> = this
                .0
                .search(&package_req.0)
                .into_iter()
                .map(|(name, versions)| {
                    (
                        name.to_string(),
                        versions.into_iter().map(|v| v.to_string()).collect(),
                    )
                })
                .collect();
            Ok(results)
        });

        methods.document("Find the latest package that matches the requirement.");
        methods.param(
            "package_req",
            "Package to search for, e.g. 'foo' or 'foo >= 1.0.0'",
        );
        methods.add_method("latest_match", |_, this, package_req: PackageReqLua| {
            Ok(this
                .0
                .latest_match(&package_req.0, None)
                .map(PackageSpecLua))
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Package database, used to look up remote rocks");
    }
}

#[derive(Clone)]
pub struct LockfileReadOnlyLua(pub Lockfile<ReadOnly>);

impl Typed for LockfileReadOnlyLua {
    fn ty() -> Type {
        Type::named("LockfileReadOnly")
    }
}

impl TypedUserData for LockfileReadOnlyLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_method("version", |_, this, ()| Ok(this.0.version().clone()));
        methods.add_method("rocks", |_, this, ()| {
            Ok(this
                .0
                .rocks()
                .iter()
                .map(|(id, rock)| {
                    (
                        id.clone().into_string().clone(),
                        LocalPackageLua(rock.clone()),
                    )
                })
                .collect::<HashMap<_, _>>())
        });

        methods.param("id", "");
        methods.add_method("get", |_, this, id: LocalPackageIdLua| {
            Ok(this.0.get(&id.0).cloned().map(LocalPackageLua))
        });

        methods.document(
            "Converts the current lockfile into a writeable one, executes `f` and flushes",
        );
        methods.param("f", "Takes the writable lockfile");
        methods.add_method("map_then_flush", |_, this, f: LuaFunction| {
            let lockfile = this.0.clone().write_guard();
            f.call::<()>(LockfileGuardLua(lockfile))?;
            Ok(())
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Read-only lockfile for an install tree");
    }
}

impl mlua::UserData for LockfileReadOnlyLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

pub struct LockfileGuardLua(pub LockfileGuard);

impl Typed for LockfileGuardLua {
    fn ty() -> Type {
        Type::named("LockfileGuard")
    }
}

impl TypedUserData for LockfileGuardLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_method("version", |_, this, ()| Ok(this.0.version().clone()));
        methods.add_method("rocks", |_, this, ()| {
            Ok(this
                .0
                .rocks()
                .iter()
                .map(|(id, rock)| {
                    (
                        id.clone().into_string().clone(),
                        LocalPackageLua(rock.clone()),
                    )
                })
                .collect::<HashMap<_, _>>())
        });

        methods.param("id", "");
        methods.add_method("get", |_, this, id: LocalPackageIdLua| {
            Ok(this.0.get(&id.0).cloned().map(LocalPackageLua))
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Flushes a lockfile automatically when it goes out of scope");
    }
}

impl mlua::UserData for LockfileGuardLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Clone)]
pub struct LockfileReadWriteLua(pub Lockfile<ReadWrite>);

impl Typed for LockfileReadWriteLua {
    fn ty() -> Type {
        Type::named("LockfileReadWrite")
    }
}

impl FromLua for WorkspaceLua {
    fn from_lua(value: LuaValue, _lua: &Lua) -> LuaResult<Self> {
        match value {
            LuaValue::UserData(ud) => Ok(ud.borrow::<WorkspaceLua>()?.clone()),
            v => Err(LuaError::FromLuaConversionError {
                from: v.type_name(),
                to: "WorkspaceLua".to_string(),
                message: None,
            }),
        }
    }
}

impl TypedUserData for LockfileReadWriteLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_method("version", |_, this, ()| Ok(this.0.version().to_owned()));
        methods.add_method("rocks", |_, this, ()| {
            Ok(this
                .0
                .rocks()
                .iter()
                .map(|(id, rock)| {
                    (
                        id.clone().into_string().clone(),
                        LocalPackageLua(rock.clone()),
                    )
                })
                .collect::<HashMap<_, _>>())
        });

        methods.param("id", "");
        methods.add_method("get", |_, this, id: String| {
            Ok(this
                .0
                .get(unsafe { &LocalPackageId::from_unchecked(id) })
                .cloned()
                .map(LocalPackageLua))
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Writable lockfile for an install tree");
    }
}

impl mlua::UserData for LockfileReadWriteLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct WorkspaceLua(pub Workspace);

impl Typed for WorkspaceLua {
    fn ty() -> Type {
        Type::named("Workspace")
    }
}

impl TypedUserData for WorkspaceLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_method("root", |_, this, ()| {
            Ok(this.0.root().to_slash_lossy().into_owned())
        });
        methods.add_method("members", |_, this, ()| {
            Ok(this
                .0
                .members()
                .iter()
                .map(|project| ProjectLua(project.to_owned()))
                .collect_vec()
                .to_owned())
        });

        methods.param("name", "Package name of the member to select");
        methods.add_method(
            "single_member_or_select",
            |_, mut this, name: Option<PackageNameLua>| {
                this.0
                    .single_member_or_select(&name.map(|name| name.0))
                    .map(|project| ProjectLua(project.to_owned()))
                    .map_err(|err| LuaError::RuntimeError(err.to_string()))
            },
        );
        methods.add_method("lockfile_path", |_, this, ()| {
            Ok(this.0.lockfile_path().to_slash_lossy().into_owned())
        });

        methods.param("config", "Lux config");
        methods.add_method("tree", |_, this, config: ConfigLua| {
            this.0.tree(&config.0).map(TreeLua).into_lua_err()
        });

        methods.param("config", "Lux config");
        methods.add_method("test_tree", |_, this, config: ConfigLua| {
            this.0.test_tree(&config.0).map(TreeLua).into_lua_err()
        });

        methods.add_method("luarc_path", |_, this, config: ConfigLua| {
            Ok(this.0.luarc_path(&config.0).to_slash_lossy().into_owned())
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("A workspace, which can contain one or many Lux projects");
    }
}

impl mlua::UserData for WorkspaceLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Debug, Clone)]
pub struct ProjectLua(pub Project);

impl Typed for ProjectLua {
    fn ty() -> Type {
        Type::named("Project")
    }
}

impl FromLua for ProjectLua {
    fn from_lua(value: LuaValue, _lua: &Lua) -> LuaResult<Self> {
        match value {
            LuaValue::UserData(ud) => Ok(ud.borrow::<ProjectLua>()?.clone()),
            v => Err(LuaError::FromLuaConversionError {
                from: v.type_name(),
                to: "ProjectLua".to_string(),
                message: None,
            }),
        }
    }
}

impl TypedUserData for ProjectLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_method("toml_path", |_, this, ()| {
            Ok(this.0.toml_path().to_slash_lossy().into_owned())
        });
        methods.add_method("extra_rockspec_path", |_, this, ()| {
            Ok(this.0.extra_rockspec_path().to_slash_lossy().into_owned())
        });
        methods.add_method("root", |_, this, ()| {
            Ok(this
                .0
                .root()
                .as_ref()
                .to_owned()
                .to_slash_lossy()
                .into_owned())
        });
        methods.add_method("toml", |_, this, ()| {
            Ok(PartialProjectTomlLua(this.0.toml().clone()))
        });
        methods.add_method("local_rockspec", |_, this, ()| {
            this.0
                .local_rockspec()
                .map(LocalLuaRockspecLua)
                .into_lua_err()
        });

        methods.param("specrev", "The revision of the RockSpec");
        methods.add_method("remote_rockspec", |_, this, specrev: Option<SpecRevLua>| {
            this.0
                .remote_rockspec(specrev.map(|s| s.0))
                .map(RemoteLuaRockspecLua)
                .into_lua_err()
        });

        methods.param("config", "Lux config");
        methods.add_method("lua_version", |_, this, config: ConfigLua| {
            this.0
                .lua_version(&config.0)
                .map(|v| v.to_string())
                .into_lua_err()
        });
        methods.add_method("extra_rockspec", |_, this, ()| {
            this.0
                .extra_rockspec()
                .map(|opt| opt.map(PartialLuaRockspecLua))
                .into_lua_err()
        });

        methods.param("deps", "Dependencies to add");
        methods.param("config", "Lux config");
        methods.add_async_method_mut(
            "add",
            |_, mut this, (deps, config): (DependencyTypeLua<PackageReqLua>, ConfigLua)| async move {
                let _guard = lux_lib::lua::lua_runtime().enter();
                let deps = map_dependency_type(deps.0);
                let package_db =
                    RemotePackageDB::from_config(&config.0)
                        .await
                        .into_lua_err()?;
                this.0.add(deps.as_ref(), &package_db).await.into_lua_err()
            },
        );

        methods.param("deps", "Dependencies to remove");
        methods.add_async_method_mut(
            "remove",
            |_, mut this, deps: DependencyTypeLua<PackageNameLua>| async move {
                let _guard = lux_lib::lua::lua_runtime().enter();
                let deps = map_dependency_type_names(deps.0);
                this.0.remove(deps.as_ref()).await.into_lua_err()
            },
        );

        methods.add_method("project_files", |_, this, ()| {
            Ok(this
                .0
                .project_files()
                .into_iter()
                .map(|p| p.to_slash_lossy().into_owned())
                .collect::<Vec<_>>())
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Lux project, with methods for managing dependencies, etc.");
    }
}

impl mlua::UserData for ProjectLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[derive(Clone)]
pub struct DownloadedRockspecLua(pub DownloadedRockspec);

impl Typed for DownloadedRockspecLua {
    fn ty() -> Type {
        Type::named("DownloadedRockspec")
    }
}

impl_from_lua_userdata!(DownloadedRockspecLua);

impl TypedUserData for DownloadedRockspecLua {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.add_method("rockspec", |_, this, ()| {
            Ok(RemoteLuaRockspecLua(this.0.rockspec.clone()))
        });
    }
    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Remote Lua RockSpec that has been downloaded from a remote server, along with its source metadata");
    }
}

impl mlua::UserData for DownloadedRockspecLua {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

// Definition registrations

#[cfg(feature = "definitions")]
mod definitions_registry {
    use mlua_extras::typed::{Type, TypedClassBuilder};

    use super::{
        BuildSpecLua, BustedTestSpecLua, CMakeBuildSpecLua, CommandBuildSpecLua,
        CommandTestSpecLua, ConfigBuilderLua, ConfigLua, DownloadedRockspecLua, GitSourceLua,
        InstallSpecLua, LocalLuaRockspecLua, LocalPackageHashesLua, LocalPackageLua,
        LocalProjectTomlLua, LockfileGuardLua, LockfileReadOnlyLua, LockfileReadWriteLua,
        LuaDependencySpecLua, LuaScriptTestSpecLua, MakeBuildSpecLua, ModulePathsLua,
        PackageReqLua, PackageSpecLua, PartialLuaRockspecLua, PartialProjectTomlLua,
        PlatformSupportLua, ProjectLua, RemoteLuaRockspecLua, RemotePackageDBLua,
        RemoteProjectTomlLua, RemoteRockSourceLua, RockDescriptionLua, RockLayoutConfigLua,
        RockLayoutLua, RustMluaBuildSpecLua, TreeLua, TreesitterParserBuildSpecLua,
    };
    use crate::definitions::LuxDefinition;

    macro_rules! submit_definitions {
        ($($name:literal => $ty:ty),+ $(,)?) => {
            $(
                inventory::submit! {
                    LuxDefinition {
                        name: $name,
                        build: || Type::class(TypedClassBuilder::new::<$ty>().build()),
                    }
                }
            )+
        };
    }

    submit_definitions! {
        "PackageSpec" => PackageSpecLua,
        "PackageReq" => PackageReqLua,
        "LocalPackageHashes" => LocalPackageHashesLua,
        "LocalPackage" => LocalPackageLua,
        "RockLayout" => RockLayoutLua,
        "Tree" => TreeLua,
        "RockLayoutConfig" => RockLayoutConfigLua,
        "Config" => ConfigLua,
        "ConfigBuilder" => ConfigBuilderLua,
        "LuaDependencySpec" => LuaDependencySpecLua,
        "PlatformSupport" => PlatformSupportLua,
        "RockDescription" => RockDescriptionLua,
        "GitSource" => GitSourceLua,
        "RemoteRockSource" => RemoteRockSourceLua,
        "BustedTestSpec" => BustedTestSpecLua,
        "CommandTestSpec" => CommandTestSpecLua,
        "LuaScriptTestSpec" => LuaScriptTestSpecLua,
        "ModulePaths" => ModulePathsLua,
        "CMakeBuildSpec" => CMakeBuildSpecLua,
        "MakeBuildSpec" => MakeBuildSpecLua,
        "TreesitterParserBuildSpec" => TreesitterParserBuildSpecLua,
        "RustMluaBuildSpec" => RustMluaBuildSpecLua,
        "CommandBuildSpec" => CommandBuildSpecLua,
        "InstallSpec" => InstallSpecLua,
        "BuildSpec" => BuildSpecLua,
        "LocalLuaRockspec" => LocalLuaRockspecLua,
        "RemoteLuaRockspec" => RemoteLuaRockspecLua,
        "PartialLuaRockspec" => PartialLuaRockspecLua,
        "PartialProjectToml" => PartialProjectTomlLua,
        "LocalProjectToml" => LocalProjectTomlLua,
        "RemoteProjectToml" => RemoteProjectTomlLua,
        "RemotePackageDB" => RemotePackageDBLua,
        "LockfileReadOnly" => LockfileReadOnlyLua,
        "LockfileGuard" => LockfileGuardLua,
        "LockfileReadWrite" => LockfileReadWriteLua,
        "Project" => ProjectLua,
        "DownloadedRockspec" => DownloadedRockspecLua,
    }
}

fn map_dependency_type(deps: DependencyType<PackageReqLua>) -> DependencyType<PackageReq> {
    match deps {
        DependencyType::Regular(v) => DependencyType::Regular(v.into_iter().map(|x| x.0).collect()),
        DependencyType::Build(v) => DependencyType::Build(v.into_iter().map(|x| x.0).collect()),
        DependencyType::Test(v) => DependencyType::Test(v.into_iter().map(|x| x.0).collect()),
        DependencyType::External(m) => DependencyType::External(m),
    }
}

fn map_dependency_type_names(deps: DependencyType<PackageNameLua>) -> DependencyType<PackageName> {
    match deps {
        DependencyType::Regular(v) => DependencyType::Regular(v.into_iter().map(|x| x.0).collect()),
        DependencyType::Build(v) => DependencyType::Build(v.into_iter().map(|x| x.0).collect()),
        DependencyType::Test(v) => DependencyType::Test(v.into_iter().map(|x| x.0).collect()),
        DependencyType::External(m) => DependencyType::External(m),
    }
}

#[derive(Deserialize_enum_str, Serialize_enum_str, Default)]
#[serde(rename_all = "snake_case")]
enum EntryTypeLua {
    #[default]
    Entrypoint,
    DependencyOnly,
}

impl From<EntryTypeLua> for EntryType {
    fn from(val: EntryTypeLua) -> Self {
        match val {
            EntryTypeLua::Entrypoint => Self::Entrypoint,
            EntryTypeLua::DependencyOnly => Self::DependencyOnly,
        }
    }
}

#[derive(Deserialize_enum_str, Serialize_enum_str, Default)]
#[serde(rename_all = "snake_case")]
enum BuildBehaviourLua {
    #[default]
    NoForce,
    Force,
}

impl From<BuildBehaviourLua> for BuildBehaviour {
    fn from(val: BuildBehaviourLua) -> Self {
        match val {
            BuildBehaviourLua::NoForce => Self::NoForce,
            BuildBehaviourLua::Force => Self::Force,
        }
    }
}

/// Intermediate struct for deserialization. Takes on two variants:
/// ```lua
/// "say >= 1.3"
///
/// { package = "say >= 1.3", entry_type = "entrypoint", pin = false, opt = false, build_behaviour = "no_force" }
/// ```
#[derive(Deserialize)]
#[serde(untagged)]
enum PackageInstallSpecInput {
    Simple(String),
    Full {
        package: String,
        #[serde(default)]
        entry_type: EntryTypeLua,
        #[serde(default)]
        pin: bool,
        #[serde(default)]
        opt: bool,
        #[serde(default)]
        build_behaviour: BuildBehaviourLua,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::TempDir;
    use mlua::prelude::*;

    const BASIC_PROJECT: &str = r#"
package = "test-package"
version = "0.1.0"
lua = "5.1"

[dependencies]
say = ">=1.0"

[source]
url = "https://example.com/test/test"

[build]
type = "builtin"
"#;

    fn setup_lua() -> (TempDir, Lua) {
        let tree = TempDir::new().unwrap();
        let lua = Lua::new();
        lua.globals().set("lux", crate::lux(&lua).unwrap()).unwrap();
        lua.globals()
            .set("user_tree", tree.path().to_str().unwrap())
            .unwrap();
        (tree, lua)
    }

    fn create_project(toml: &str) -> (TempDir, Lua) {
        let project = TempDir::new().unwrap();
        std::fs::write(project.join("lux.toml"), toml).unwrap();
        let (_, lua) = setup_lua();
        lua.globals()
            .set("project_location", project.path())
            .unwrap();
        (project, lua)
    }

    #[test]
    fn lua_api_test_lua_version_lua() {
        let (_tree, lua) = setup_lua();
        lua.load(
            r#"
            local config = lux.config.builder()
                :lua_version("5.1")
                :user_tree(user_tree)
                :build()
            assert(config, "config should not be nil")
            assert(config:user_tree("5.1"), "user_tree should not be nil")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_package_version_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:local_rockspec()
            local version = rockspec:version()
            assert(type(version) == "string", "version should be a string")
            assert(version == "0.1.0-1", "version should be 0.1.0-1")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_package_name_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:local_rockspec()
            local name = rockspec:package()
            assert(type(name) == "string", "name should be a string")
            assert(name == "test-package", "name should be test-package")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_package_version_req_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:local_rockspec()
            local lua_req = rockspec:lua()
            assert(type(lua_req) == "string", "lua version req should be a string")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_platform_identifier_lua() {
        let (_tree, lua) = setup_lua();
        lua.load(
            r#"
            local config = lux.config.builder()
                :lua_version("5.1")
                :user_tree(user_tree)
                :build()
            local tree = config:user_tree("5.1")
            assert(tree, "tree should not be nil")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_rockspec_format_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:local_rockspec()
            local format = rockspec:format()
            assert(format == nil or type(format) == "string", "format should be nil or string")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_pinned_state_lua() {
        let (_tree, lua) = setup_lua();
        lua.load(
            r#"
            local config = lux.config.builder()
                :lua_version("5.1")
                :user_tree(user_tree)
                :build()
            local tree = config:user_tree("5.1")
            local lockfile = tree:lockfile()
            local rocks = lockfile:rocks()
            assert(type(rocks) == "table", "rocks should be a table")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_opt_state_lua() {
        let (_tree, lua) = setup_lua();
        lua.load(
            r#"
            local config = lux.config.builder()
                :lua_version("5.1")
                :user_tree(user_tree)
                :build()
            local tree = config:user_tree("5.1")
            local lockfile = tree:lockfile()
            local version = lockfile:version()
            assert(type(version) == "string", "lockfile version should be a string")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_local_package_id_lua() {
        let (_tree, lua) = setup_lua();
        lua.load(
            r#"
            local config = lux.config.builder()
                :lua_version("5.1")
                :user_tree(user_tree)
                :build()
            local tree = config:user_tree("5.1")
            local lockfile = tree:lockfile()
            local rocks = lockfile:rocks()
            for id, rock in pairs(rocks) do
                assert(type(id) == "string", "package id should be a string")
                assert(#id == 64, "package id should be 64 chars")
                break
            end
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_lock_constraint_lua() {
        let (_tree, lua) = setup_lua();
        lua.load(
            r#"
            local config = lux.config.builder()
                :lua_version("5.1")
                :user_tree(user_tree)
                :build()
            local tree = config:user_tree("5.1")
            local lockfile = tree:lockfile()
            local rocks = lockfile:rocks()
            for id, rock in pairs(rocks) do
                local constraint = rock:constraint()
                assert(type(constraint) == "string", "constraint should be a string")
                break
            end
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_external_dependency_spec_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:local_rockspec()
            local ext_deps = rockspec:external_dependencies()
            assert(type(ext_deps) == "table", "external_dependencies should be a table")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_rock_layout_config_lua() {
        let (_tree, lua) = setup_lua();
        lua.load(
            r#"
            local config = lux.config.builder()
                :lua_version("5.1")
                :entrypoint_layout({ layout = "default" })
                :build()
            assert(config, "config should not be nil")

            local config2 = lux.config.builder()
                :lua_version("5.1")
                :entrypoint_layout({ layout = "nvim" })
                :build()
            assert(config2, "nvim config should not be nil")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_config_lua() {
        let (tree, cache, data) = (
            TempDir::new().unwrap(),
            TempDir::new().unwrap(),
            TempDir::new().unwrap(),
        );
        let (_tree, lua) = setup_lua();
        lua.globals().set("user_tree", tree.path()).unwrap();
        lua.globals().set("cache", cache.path()).unwrap();
        lua.globals().set("data", data.path()).unwrap();
        lua.load(
            r#"
            local config = lux.config.builder()
                :lua_version("5.1")
                :user_tree(user_tree)
                :cache_dir(cache)
                :data_dir(data)
                :build()
            assert(config, "config should not be nil")
            assert(config:server(), "server should not be nil")
            assert(type(config:extra_servers()) == "table", "extra_servers should be a table")
            assert(type(config:verbose()) == "boolean", "verbose should be a boolean")
            assert(type(config:no_progress()) == "boolean", "no_progress should be a boolean")
            assert(type(config:no_prompt()) == "boolean", "no_prompt should be a boolean")
            assert(type(config:timeout()) == "number", "timeout should be a number")
            assert(config:cache_dir(), "cache_dir should not be nil")
            assert(config:data_dir(), "data_dir should not be nil")
            assert(config:entrypoint_layout(), "entrypoint_layout should not be nil")
            assert(type(config:variables()) == "table", "variables should be a table")
            assert(config:make_cmd(), "make_cmd should not be nil")
            assert(config:cmake_cmd(), "cmake_cmd should not be nil")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_config_builder_lua() {
        let tree = TempDir::new().unwrap();
        let cache = TempDir::new().unwrap();
        let data = TempDir::new().unwrap();
        let (_tree, lua) = setup_lua();
        lua.globals().set("tree", tree.path()).unwrap();
        lua.globals().set("cache", cache.path()).unwrap();
        lua.globals().set("data", data.path()).unwrap();

        lua.load(
            r#"
            local config = lux.config.builder()
                :dev(true)
                :server("https://example.com")
                :extra_servers({"https://example.com", "https://example2.com"})
                :namespace("example")
                :lua_dir("lua")
                :lua_version("5.1")
                :user_tree(tree)
                :verbose(true)
                :no_progress(false)
                :no_prompt(false)
                :timeout(10)
                :cache_dir(cache)
                :data_dir(data)
                :entrypoint_layout({ layout = "nvim" })
                :user_agent("test-agent")
                :wrap_bin_scripts(true)
                :build()

            assert(config, "built config should not be nil")
            assert(config:server() == "https://example.com/", "server mismatch")
            assert(#config:extra_servers() == 2, "extra_servers count")
            assert(config:namespace() == "example", "namespace mismatch")
            assert(config:verbose(), "verbose should be true")
            assert(config:timeout() == 10, "timeout mismatch")
            assert(config:user_tree("5.1"), "user_tree should not be nil")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_tree_lua() {
        let (_tree, lua) = setup_lua();
        lua.load(
            r#"
            local config = lux.config.builder()
                :lua_version("5.1")
                :user_tree(user_tree)
                :build()
            local tree = config:user_tree("5.1")
            assert(tree, "tree should not be nil")
            assert(type(tree:root()) == "string", "root should be a string")
            assert(type(tree:bin()) == "string", "bin should be a string")
            assert(tree:lockfile(), "lockfile should not be nil")
            assert(type(tree:match_rocks("foo")) == "table", "match_rocks should return a table")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_lockfile_read_only_lua() {
        let (_tree, lua) = setup_lua();
        lua.load(
            r#"
            local config = lux.config.builder()
                :lua_version("5.1")
                :user_tree(user_tree)
                :build()
            local tree = config:user_tree("5.1")
            local lockfile = tree:lockfile()
            assert(lockfile, "lockfile should not be nil")
            assert(type(lockfile:version()) == "string", "version should be a string")
            assert(type(lockfile:rocks()) == "table", "rocks should be a table")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_lockfile_guard_lua() {
        let (_tree, lua) = setup_lua();
        lua.load(
            r#"
            local config = lux.config.builder()
                :lua_version("5.1")
                :user_tree(user_tree)
                :build()
            local tree = config:user_tree("5.1")
            local lockfile = tree:lockfile()
            lockfile:map_then_flush(function(guard)
                assert(guard, "guard should not be nil")
                assert(type(guard:version()) == "string", "version should be a string")
                assert(type(guard:rocks()) == "table", "rocks should be a table")
            end)
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_package_req_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:local_rockspec()
            local deps = rockspec:dependencies()
            assert(#deps > 0, "should have dependencies")
            local dep = deps[1]
            local req = dep:package_req()
            assert(type(req:name()) == "string", "name should be a string")
            assert(type(req:version_req()) == "string", "version_req should be a string")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_lua_dependency_spec_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:local_rockspec()
            local deps = rockspec:dependencies()
            assert(#deps > 0, "should have dependencies")
            local dep = deps[1]
            assert(type(dep:name()) == "string", "name should be a string")
            assert(type(dep:version_req()) == "string", "version_req should be a string")
            assert(type(dep:package_req()) == "userdata", "package_req should be userdata")
            local pkg_req = dep:package_req()
            assert(pkg_req:name(), "package_req name should not be nil")
            assert(pkg_req:version_req(), "package_req version_req should not be nil")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_partial_project_toml_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local toml = project:toml()
            assert(toml, "toml should not be nil")
            assert(type(toml:package()) == "string", "package should be a string")
            assert(toml:package() == "test-package", "package should be test-package")
            assert(toml:to_local(), "to_local should not be nil")
            assert(toml:to_remote(), "to_remote should not be nil")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_local_project_toml_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local toml = project:toml()
            local local_toml = toml:to_local()
            assert(local_toml, "local_toml should not be nil")
            assert(type(local_toml:package()) == "string", "package should be a string")
            assert(local_toml:package() == "test-package")
            assert(type(local_toml:version()) == "string", "version should be a string")
            assert(local_toml:version() == "0.1.0-1")
            assert(local_toml:description(), "description should not be nil")
            assert(type(local_toml:dependencies()) == "table", "dependencies should be a table")
            assert(type(local_toml:build_dependencies()) == "table")
            assert(type(local_toml:test_dependencies()) == "table")
            assert(local_toml:build(), "build should not be nil")
            assert(local_toml:test(), "test should not be nil")
            assert(local_toml:to_lua_rockspec(), "to_lua_rockspec should not be nil")
            assert(type(local_toml:to_lua_rockspec_string()) == "string")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_remote_project_toml_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local toml = project:toml()
            local remote_toml = toml:to_remote()
            assert(remote_toml, "remote_toml should not be nil")
            assert(type(remote_toml:package()) == "string")
            assert(remote_toml:package() == "test-package")
            assert(type(remote_toml:version()) == "string")
            assert(remote_toml:version() == "0.1.0-1")
            assert(remote_toml:description(), "description should not be nil")
            assert(type(remote_toml:dependencies()) == "table")
            assert(remote_toml:build(), "build should not be nil")
            assert(remote_toml:source(), "source should not be nil")
            assert(remote_toml:to_lua_rockspec(), "to_lua_rockspec should not be nil")
            assert(type(remote_toml:to_lua_rockspec_string()) == "string")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_local_lua_rockspec_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:local_rockspec()
            assert(rockspec, "rockspec should not be nil")
            assert(type(rockspec:package()) == "string")
            assert(rockspec:package() == "test-package")
            assert(type(rockspec:version()) == "string")
            assert(rockspec:version() == "0.1.0-1")
            assert(rockspec:description(), "description should not be nil")
            assert(rockspec:supported_platforms(), "supported_platforms should not be nil")
            assert(type(rockspec:lua()) == "string", "lua should be a string")
            assert(type(rockspec:dependencies()) == "table")
            assert(type(rockspec:build_dependencies()) == "table")
            assert(type(rockspec:test_dependencies()) == "table")
            assert(type(rockspec:external_dependencies()) == "table")
            assert(rockspec:build(), "build should not be nil")
            assert(rockspec:source(), "source should not be nil")
            assert(rockspec:test(), "test should not be nil")
            assert(type(rockspec:to_lua_rockspec_string()) == "string")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_remote_lua_rockspec_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:remote_rockspec()
            assert(rockspec, "rockspec should not be nil")
            assert(type(rockspec:package()) == "string")
            assert(rockspec:package() == "test-package")
            assert(type(rockspec:version()) == "string")
            assert(rockspec:version() == "0.1.0-1")
            assert(rockspec:description(), "description should not be nil")
            assert(rockspec:supported_platforms(), "supported_platforms should not be nil")
            assert(type(rockspec:lua()) == "string")
            assert(type(rockspec:dependencies()) == "table")
            assert(type(rockspec:build_dependencies()) == "table")
            assert(type(rockspec:test_dependencies()) == "table")
            assert(type(rockspec:external_dependencies()) == "table")
            assert(rockspec:build(), "build should not be nil")
            assert(rockspec:source(), "source should not be nil")
            assert(rockspec:test(), "test should not be nil")
            assert(type(rockspec:to_lua_rockspec_string()) == "string")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_rock_description_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:local_rockspec()
            local desc = rockspec:description()
            assert(desc, "description should not be nil")
            assert(type(desc:labels()) == "table", "labels should be a table")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_platform_support_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:local_rockspec()
            local platforms = rockspec:supported_platforms()
            assert(platforms, "supported_platforms should not be nil")
            assert(type(platforms:is_supported("linux")) == "boolean", "is_supported should return a boolean")
            assert(type(platforms:is_supported("windows")) == "boolean")
            assert(type(platforms:is_supported("unix")) == "boolean")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_remote_rock_source_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:local_rockspec()
            local source = rockspec:source()
            assert(source, "source should not be nil")
            assert(source:source_spec(), "source_spec should not be nil")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_build_spec_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:local_rockspec()
            local build = rockspec:build()
            assert(build, "build should not be nil")
            assert(type(build:install()) == "userdata", "install should be userdata")
            assert(type(build:copy_directories()) == "table", "copy_directories should be a table")
            assert(type(build:patches()) == "table", "patches should be a table")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_install_spec_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:local_rockspec()
            local build = rockspec:build()
            local install = build:install()
            assert(install, "install should not be nil")
            assert(type(install:lua()) == "table", "lua should be a table")
            assert(type(install:lib()) == "table", "lib should be a table")
            assert(type(install:conf()) == "table", "conf should be a table")
            assert(type(install:bin()) == "table", "bin should be a table")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_project_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            assert(project, "project should not be nil")
            assert(type(project:toml_path()) == "string", "toml_path should be a string")
            assert(type(project:extra_rockspec_path()) == "string")
            assert(type(project:root()) == "string", "root should be a string")
            assert(project:toml(), "toml should not be nil")
            assert(project:local_rockspec(), "local_rockspec should not be nil")
            assert(project:remote_rockspec(), "remote_rockspec should not be nil")
            assert(not project:extra_rockspec(), "extra_rockspec should be nil")
            assert(type(project:project_files()) == "table", "project_files should be a table")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_workspace_lua() {
        let workspace = TempDir::new().unwrap();
        std::fs::write(workspace.join("lux.toml"), BASIC_PROJECT).unwrap();
        let (_tree, lua) = setup_lua();
        lua.globals()
            .set("workspace_location", workspace.path())
            .unwrap();

        lua.load(
            r#"
            local config = lux.config.builder()
                :lua_version("5.1")
                :user_tree(user_tree)
                :build()
            local workspace = lux.workspace.new(workspace_location)
            assert(workspace, "workspace should not be nil")
            assert(type(workspace:root()) == "string", "root should be a string")
            assert(type(workspace:lockfile_path()) == "string")
            assert(workspace:tree(config), "tree should not be nil")
            assert(workspace:test_tree(config), "test_tree should not be nil")
            assert(type(workspace:members()) == "table", "members should be a table")
            assert(workspace:single_member_or_select("test-package"),
                "single_member_or_select should not be nil")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_extra_rockspec_lua_build_fn() {
        let (project_dir, lua) = create_project(BASIC_PROJECT);
        lua.globals()
            .set("project_dir", project_dir.path())
            .unwrap();
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local extra_path = project:extra_rockspec_path()
            local f = io.open(extra_path, "w")
            if f then
                f:write([[
package = "test-package"
lua = "5.1"

build = (function()
    return { type = "builtin", modules = { foo = "foo.lua" } }
end)()
]])
                f:close()
            end
            local project2 = lux.project.new(project_location)
            local extra = project2:extra_rockspec()
            assert(extra ~= nil, "extra_rockspec should not be nil after writing file")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_dependency_type_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:local_rockspec()
            local deps = rockspec:dependencies()
            assert(type(deps) == "table", "dependencies should be a table")
            assert(#deps > 0, "should have at least one dependency")

            local build_deps = rockspec:build_dependencies()
            assert(type(build_deps) == "table", "build_dependencies should be a table")

            local test_deps = rockspec:test_dependencies()
            assert(type(test_deps) == "table", "test_dependencies should be a table")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_rock_matches_lua() {
        let (_tree, lua) = setup_lua();
        lua.load(
            r#"
            local config = lux.config.builder()
                :lua_version("5.1")
                :user_tree(user_tree)
                :build()
            local tree = config:user_tree("5.1")
            local matches = tree:match_rocks("nonexistent-package-xyz")
            assert(matches, "match_rocks should return a table")
            assert(type(matches.is_found) == "function", "is_found should be a function")
            assert(type(matches:is_found()) == "boolean", "is_found() should return a boolean")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_rock_source_spec_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:local_rockspec()
            local source = rockspec:source()
            local spec = source:source_spec()
            assert(type(spec) == "table", "source_spec should be a table")
            assert(spec.url, "url source should have url key")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_test_spec_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:local_rockspec()
            local test = rockspec:test()
            assert(type(test) == "table", "test should be a table")
            assert(test.auto_detect, "default test should be auto_detect")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_module_spec_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:local_rockspec()
            local build = rockspec:build()
            assert(build, "build should not be nil")
            assert(build:build_backend(), "build_backend should not be nil")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_make_build_spec_lua() {
        let (_project, lua) = create_project(BASIC_PROJECT);
        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:local_rockspec()
            local build = rockspec:build()
            local backend = build:build_backend()
            assert(backend, "build_backend should not be nil")
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn lua_api_test_command_build_spec_lua() {
        let project = TempDir::new().unwrap();
        std::fs::write(
            project.join("lux.toml"),
            r#"
package = "test-cmd"
version = "0.1.0"
lua = "5.1"

[source]
url = "https://example.com/test/test"

[build]
type = "command"
build_command = "make all"
install_command = "make install"
"#,
        )
        .unwrap();
        let (_tree, lua) = setup_lua();
        lua.globals()
            .set("project_location", project.path())
            .unwrap();

        lua.load(
            r#"
            local project = lux.project.new(project_location)
            local rockspec = project:local_rockspec()
            local build = rockspec:build()
            local backend = build:build_backend()
            assert(backend, "command build_backend should not be nil")
            assert(backend:build_command() == "make all", "build_command mismatch")
            assert(backend:install_command() == "make install", "install_command mismatch")
        "#,
        )
        .exec()
        .unwrap();
    }
}
