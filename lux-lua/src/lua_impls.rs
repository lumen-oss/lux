#![allow(unused)]

//! Lua implementations for various types in lux-lib. These are used to convert between Lua and
//! Rust types when calling Rust functions from Lua.

use std::{collections::HashMap, path::PathBuf, str::FromStr, time::Duration};

use itertools::Itertools;
use mlua::prelude::*;
use serde::{Deserialize, Serialize};
use serde_enum_str::{Deserialize_enum_str, Serialize_enum_str};
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
    progress::{HasProgress, Progress, ProgressBar},
    project::{
        project_toml::{LocalProjectToml, PartialProjectToml, RemoteProjectToml},
        Project,
    },
    remote_package_db::RemotePackageDB,
    rockspec::{
        lua_dependency::{DependencyType, LuaDependencySpec, LuaDependencyType},
        Rockspec,
    },
    tree::{EntryType, RockLayout, RockMatches, Tree},
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

#[derive(Debug, Clone)]
pub struct PackageSpecLua(pub PackageSpec);

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

impl LuaUserData for PackageSpecLua {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |_, this| Ok(this.0.name().to_string()));
        fields.add_field_method_get("version", |_, this| Ok(this.0.version().to_string()));
    }
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("to_package_req", |_, this, ()| {
            Ok(PackageReqLua(this.0.clone().into_package_req()))
        });
    }
}

#[derive(Debug, Clone)]
pub struct PackageReqLua(pub PackageReq);

impl FromLua for PackageReqLua {
    fn from_lua(value: LuaValue, lua: &Lua) -> LuaResult<Self> {
        let str: String = lua.from_value(value)?;
        PackageReq::parse(&str).map(PackageReqLua).into_lua_err()
    }
}

impl LuaUserData for PackageReqLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("name", |_, this, ()| Ok(this.0.name().to_string()));
        methods.add_method("version_req", |_, this, ()| {
            Ok(this.0.version_req().to_string())
        });
        methods.add_method("matches", |_, this, package: PackageSpecLua| {
            Ok(this.0.matches(&package.0))
        });
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PinnedStateLua(pub PinnedState);

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
pub struct LocalPackageHashesLua(pub LocalPackageHashes);

impl LuaUserData for LocalPackageHashesLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("rockspec", |_, this, ()| Ok(this.0.rockspec.to_hex().1));
        methods.add_method("source", |_, this, ()| Ok(this.0.source.to_hex().1));
    }
}
impl_from_lua_userdata!(LocalPackageHashesLua);

#[derive(Debug, Clone)]
pub struct LocalPackageLua(pub LocalPackage);

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

impl LuaUserData for LocalPackageLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
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
}

pub struct RockLayoutLua(pub RockLayout);

impl LuaUserData for RockLayoutLua {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("rock_path", |_, this| Ok(this.0.rock_path.clone()));
        fields.add_field_method_get("etc", |_, this| Ok(this.0.etc.clone()));
        fields.add_field_method_get("lib", |_, this| Ok(this.0.lib.clone()));
        fields.add_field_method_get("src", |_, this| Ok(this.0.src.clone()));
        fields.add_field_method_get("bin", |_, this| Ok(this.0.bin.clone()));
        fields.add_field_method_get("conf", |_, this| Ok(this.0.conf.clone()));
        fields.add_field_method_get("doc", |_, this| Ok(this.0.doc.clone()));
    }
}

pub struct RockMatchesLua(pub RockMatches);

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

#[derive(Debug, Clone)]
pub struct TreeLua(pub Tree);

impl LuaUserData for TreeLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("root", |_, this, ()| Ok(this.0.root()));
        methods.add_method("root_for", |_, this, package: LocalPackageLua| {
            Ok(this.0.root_for(&package.0))
        });
        methods.add_method("bin", |_, this, ()| Ok(this.0.bin()));
        methods.add_method("match_rocks", |lua, this, req: PackageReqLua| {
            this.0
                .match_rocks(&req.0)
                .map(|m| RockMatchesLua(m).into_lua(lua))
                .map_err(|err| LuaError::RuntimeError(err.to_string()))?
        });
        methods.add_method("rock_layout", |_, this, package: LocalPackageLua| {
            this.0
                .installed_rock_layout(&package.0)
                .map(RockLayoutLua)
                .map_err(|err| LuaError::RuntimeError(err.to_string()))
        });
        methods.add_method("lockfile", |_, this, ()| {
            this.0.lockfile().map(LockfileReadOnlyLua).into_lua_err()
        });
    }
}
impl_from_lua_userdata!(TreeLua);

#[derive(Debug, Clone)]
pub struct RockLayoutConfigLua(pub RockLayoutConfig);

impl LuaUserData for RockLayoutConfigLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_function("new", |_, ()| {
            Ok(RockLayoutConfigLua(RockLayoutConfig::default()))
        });
        methods.add_function("new_nvim_layout", |_, ()| {
            Ok(RockLayoutConfigLua(RockLayoutConfig::new_nvim_layout()))
        });
    }
}
impl_from_lua_userdata!(RockLayoutConfigLua);

#[derive(Debug, Clone)]
pub struct ConfigLua(pub Config);

impl LuaUserData for ConfigLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_function("builder", |_, ()| {
            ConfigBuilder::new().map(ConfigBuilderLua).into_lua_err()
        });
        methods.add_method("server", |_, this, ()| Ok(this.0.server().to_string()));
        methods.add_method("extra_servers", |_, this, ()| {
            Ok(this
                .0
                .extra_servers()
                .iter()
                .map(|url| url.to_string())
                .collect_vec())
        });
        methods.add_method("only_sources", |_, this, ()| {
            Ok(this.0.only_sources().cloned())
        });
        methods.add_method("namespace", |_, this, ()| Ok(this.0.namespace().cloned()));
        methods.add_method("lua_dir", |_, this, ()| Ok(this.0.lua_dir().cloned()));
        methods.add_method("user_tree", |_, this, lua_version: LuaVersionLua| {
            this.0.user_tree(lua_version.0).map(TreeLua).into_lua_err()
        });
        methods.add_method("verbose", |_, this, ()| Ok(this.0.verbose()));
        methods.add_method("no_progress", |_, this, ()| Ok(this.0.no_progress()));
        methods.add_method("timeout", |_, this, ()| Ok(this.0.timeout().as_secs()));
        methods.add_method("cache_dir", |_, this, ()| Ok(this.0.cache_dir().clone()));
        methods.add_method("data_dir", |_, this, ()| Ok(this.0.data_dir().clone()));
        methods.add_method("entrypoint_layout", |_, this, ()| {
            Ok(RockLayoutConfigLua(this.0.entrypoint_layout().clone()))
        });
        methods.add_method("variables", |_, this, ()| Ok(this.0.variables().clone()));
        methods.add_method("make_cmd", |_, this, ()| Ok(this.0.make_cmd()));
        methods.add_method("cmake_cmd", |_, this, ()| Ok(this.0.cmake_cmd()));
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

#[derive(Clone)]
pub struct ConfigBuilderLua(pub ConfigBuilder);

impl LuaUserData for ConfigBuilderLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("dev", |_, this, dev: Option<bool>| {
            Ok(ConfigBuilderLua(this.0.clone().dev(dev)))
        });
        methods.add_method("server", |_, this, server: Option<LuaUrl>| {
            Ok(ConfigBuilderLua(
                this.0.clone().server(server.map(|url| url.0)),
            ))
        });
        methods.add_method("extra_servers", |_, this, servers: Option<Vec<LuaUrl>>| {
            Ok(ConfigBuilderLua(this.0.clone().extra_servers(
                servers.map(|urls| urls.into_iter().map(|url| url.0).collect()),
            )))
        });
        methods.add_method("only_sources", |_, this, sources: Option<String>| {
            Ok(ConfigBuilderLua(this.0.clone().only_sources(sources)))
        });
        methods.add_method("namespace", |_, this, namespace: Option<String>| {
            Ok(ConfigBuilderLua(this.0.clone().namespace(namespace)))
        });
        methods.add_method("lua_dir", |_, this, lua_dir: Option<PathBuf>| {
            Ok(ConfigBuilderLua(this.0.clone().lua_dir(lua_dir)))
        });
        methods.add_method(
            "lua_version",
            |_, this, lua_version: Option<LuaVersionLua>| {
                Ok(ConfigBuilderLua(
                    this.0.clone().lua_version(lua_version.map(|v| v.0)),
                ))
            },
        );
        methods.add_method("user_tree", |_, this, tree: Option<PathBuf>| {
            Ok(ConfigBuilderLua(this.0.clone().user_tree(tree)))
        });
        methods.add_method("verbose", |_, this, verbose: Option<bool>| {
            Ok(ConfigBuilderLua(this.0.clone().verbose(verbose)))
        });
        methods.add_method("no_progress", |_, this, no_progress: Option<bool>| {
            Ok(ConfigBuilderLua(this.0.clone().no_progress(no_progress)))
        });
        methods.add_method("timeout", |_, this, timeout: Option<u64>| {
            Ok(ConfigBuilderLua(
                this.0.clone().timeout(timeout.map(Duration::from_secs)),
            ))
        });
        methods.add_method("cache_dir", |_, this, cache_dir: Option<PathBuf>| {
            Ok(ConfigBuilderLua(this.0.clone().cache_dir(cache_dir)))
        });
        methods.add_method("data_dir", |_, this, data_dir: Option<PathBuf>| {
            Ok(ConfigBuilderLua(this.0.clone().data_dir(data_dir)))
        });
        methods.add_method(
            "entrypoint_layout",
            |_, this, layout: Option<RockLayoutConfigLua>| {
                Ok(ConfigBuilderLua(this.0.clone().entrypoint_layout(
                    layout.map(|l| l.0).unwrap_or_default(),
                )))
            },
        );
        methods.add_method("generate_luarc", |_, this, generate: Option<bool>| {
            Ok(ConfigBuilderLua(this.0.clone().generate_luarc(generate)))
        });
        methods.add_method("build", |_, this, ()| {
            this.0.clone().build().map(ConfigLua).into_lua_err()
        });
    }
}

#[derive(Debug, Clone)]
pub struct LuaDependencySpecLua(pub LuaDependencySpec);

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

impl LuaUserData for LuaDependencySpecLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("name", |_, this, ()| Ok(this.0.name().to_string()));
        methods.add_method("version_req", |_, this, ()| {
            Ok(this.0.version_req().to_string())
        });
        methods.add_method("matches", |_, this, package: PackageSpecLua| {
            Ok(this.0.matches(&package.0))
        });
        methods.add_method("package_req", |_, this, ()| {
            Ok(PackageReqLua(this.0.package_req().clone()))
        });
    }
}

pub struct DependencyTypeLua<T>(pub DependencyType<T>);

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

#[derive(Debug, Clone)]
pub struct ExternalDependencySpecLua(pub ExternalDependencySpec);

impl IntoLua for ExternalDependencySpecLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let table = lua.create_table()?;
        if let Some(path) = self.0.header {
            table.set("header", path.to_string_lossy().to_string())?;
        }
        if let Some(path) = self.0.library {
            table.set("library", path.to_string_lossy().to_string())?;
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
pub struct PlatformSupportLua(pub PlatformSupport);

impl LuaUserData for PlatformSupportLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method(
            "is_supported",
            |_, this, platform: PlatformIdentifierLua| Ok(this.0.is_supported(&platform.0)),
        );
    }
}

#[derive(Debug, Clone)]
pub struct RockspecFormatLua(pub RockspecFormat);

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
pub struct RockDescriptionLua(pub RockDescription);

impl LuaUserData for RockDescriptionLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("summary", |_, this, ()| Ok(this.0.summary.clone()));
        methods.add_method("detailed", |_, this, ()| Ok(this.0.detailed.clone()));
        methods.add_method("license", |_, this, ()| Ok(this.0.license.clone()));
        methods.add_method("homepage", |_, this, ()| {
            Ok(this.0.homepage.as_ref().map(|url| url.to_string()))
        });
        methods.add_method("issues_url", |_, this, ()| Ok(this.0.issues_url.clone()));
        methods.add_method("maintainer", |_, this, ()| Ok(this.0.maintainer.clone()));
        methods.add_method("labels", |_, this, ()| Ok(this.0.labels.clone()));
    }
}

pub struct RockSourceSpecLua(pub RockSourceSpec);

impl IntoLua for RockSourceSpecLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let table = lua.create_table()?;
        match self.0 {
            RockSourceSpec::Git(git) => {
                table.set("git", GitSourceLua(git))?;
            }
            RockSourceSpec::File(path) => {
                table.set("file", path.to_string_lossy().to_string())?;
            }
            RockSourceSpec::Url(url) => {
                table.set("url", url.to_string())?;
            }
        };
        Ok(LuaValue::Table(table))
    }
}

#[derive(Debug, Clone)]
pub struct GitSourceLua(pub GitSource);

impl LuaUserData for GitSourceLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("url", |_, this, ()| Ok(this.0.url.to_string()));
        methods.add_method("checkout_ref", |_, this, ()| {
            Ok(this.0.checkout_ref.clone())
        });
    }
}

#[derive(Debug, Clone)]
pub struct RemoteRockSourceLua(pub RemoteRockSource);

impl LuaUserData for RemoteRockSourceLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("source_spec", |_, this, ()| {
            Ok(RockSourceSpecLua(this.0.source_spec.clone()))
        });
        methods.add_method("archive_name", |_, this, ()| {
            Ok(this.0.archive_name.clone())
        });
        methods.add_method("unpack_dir", |_, this, ()| Ok(this.0.unpack_dir.clone()));
    }
}

#[derive(Debug, Clone)]
pub struct BustedTestSpecLua(pub BustedTestSpec);

impl LuaUserData for BustedTestSpecLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("flags", |_, this, ()| Ok(this.0.flags().clone()));
    }
}

#[derive(Debug, Clone)]
pub struct CommandTestSpecLua(pub CommandTestSpec);

impl LuaUserData for CommandTestSpecLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("command", |_, this, ()| Ok(this.0.command().to_string()));
        methods.add_method("flags", |_, this, ()| Ok(this.0.flags().clone()));
    }
}

#[derive(Debug, Clone)]
pub struct LuaScriptTestSpecLua(pub LuaScriptTestSpec);

impl LuaUserData for LuaScriptTestSpecLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("script", |_, this, ()| Ok(this.0.script().clone()));
        methods.add_method("flags", |_, this, ()| Ok(this.0.flags().clone()));
    }
}

pub struct TestSpecLua(pub TestSpec);

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

#[derive(Debug, Clone)]
pub struct LuaModuleLua(pub LuaModule);

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

#[derive(Debug, Clone)]
pub struct ModuleSpecLua(pub ModuleSpec);

impl IntoLua for ModuleSpecLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let table = lua.create_table()?;
        match self.0 {
            ModuleSpec::SourcePath(path) => table.set("source", path)?,
            ModuleSpec::SourcePaths(paths) => table.set("sources", paths)?,
            ModuleSpec::ModulePaths(mp) => table.set("modules", ModulePathsLua(mp))?,
        }
        Ok(LuaValue::Table(table))
    }
}

#[derive(Debug, Clone)]
pub struct ModulePathsLua(pub ModulePaths);

impl LuaUserData for ModulePathsLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("sources", |_, this, ()| Ok(this.0.sources.clone()));
        methods.add_method("libraries", |_, this, ()| Ok(this.0.libraries.clone()));
        methods.add_method("defines", |_, this, ()| {
            Ok(this
                .0
                .defines
                .iter()
                .cloned()
                .collect::<HashMap<_, Option<_>>>())
        });
        methods.add_method("incdirs", |_, this, ()| Ok(this.0.incdirs.clone()));
        methods.add_method("libdirs", |_, this, ()| Ok(this.0.libdirs.clone()));
    }
}

pub struct BuiltinBuildSpecLua(pub BuiltinBuildSpec);

impl IntoLua for BuiltinBuildSpecLua {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let table = lua.create_table()?;
        for (module, spec) in self.0.modules {
            table.set(module.as_str().to_string(), ModuleSpecLua(spec))?;
        }
        Ok(LuaValue::Table(table))
    }
}

#[derive(Debug, Clone)]
pub struct CMakeBuildSpecLua(pub CMakeBuildSpec);

impl LuaUserData for CMakeBuildSpecLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("cmake_lists_content", |_, this, ()| {
            Ok(this.0.cmake_lists_content.clone())
        });
        methods.add_method("build_pass", |_, this, ()| Ok(this.0.build_pass));
        methods.add_method("install_pass", |_, this, ()| Ok(this.0.install_pass));
        methods.add_method("variables", |_, this, ()| Ok(this.0.variables.clone()));
    }
}

#[derive(Debug, Clone)]
pub struct MakeBuildSpecLua(pub MakeBuildSpec);

impl LuaUserData for MakeBuildSpecLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("makefile", |_, this, ()| Ok(this.0.makefile.clone()));
        methods.add_method("build_target", |_, this, ()| {
            Ok(this.0.build_target.clone())
        });
        methods.add_method("build_pass", |_, this, ()| Ok(this.0.build_pass));
        methods.add_method("install_target", |_, this, ()| {
            Ok(this.0.install_target.clone())
        });
        methods.add_method("install_pass", |_, this, ()| Ok(this.0.install_pass));
        methods.add_method("build_variables", |_, this, ()| {
            Ok(this.0.build_variables.clone())
        });
        methods.add_method("install_variables", |_, this, ()| {
            Ok(this.0.install_variables.clone())
        });
        methods.add_method("variables", |_, this, ()| Ok(this.0.variables.clone()));
    }
}

#[derive(Debug, Clone)]
pub struct TreesitterParserBuildSpecLua(pub TreesitterParserBuildSpec);

impl LuaUserData for TreesitterParserBuildSpecLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("lang", |_, this, ()| Ok(this.0.lang.clone()));
        methods.add_method("parser", |_, this, ()| Ok(this.0.parser));
        methods.add_method("generate", |_, this, ()| Ok(this.0.generate));
        methods.add_method("location", |_, this, ()| Ok(this.0.location.clone()));
        methods.add_method("queries", |_, this, ()| Ok(this.0.queries.clone()));
    }
}

#[derive(Debug, Clone)]
pub struct RustMluaBuildSpecLua(pub RustMluaBuildSpec);

impl LuaUserData for RustMluaBuildSpecLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("modules", |_, this, ()| Ok(this.0.modules.clone()));
        methods.add_method("target_path", |_, this, ()| Ok(this.0.target_path.clone()));
        methods.add_method("default_features", |_, this, ()| {
            Ok(this.0.default_features)
        });
        methods.add_method("features", |_, this, ()| Ok(this.0.features.clone()));
        methods.add_method("cargo_extra_args", |_, this, ()| {
            Ok(this.0.cargo_extra_args.clone())
        });
        methods.add_method("include", |_, this, ()| Ok(this.0.include.clone()));
    }
}

#[derive(Debug, Clone)]
pub struct CommandBuildSpecLua(pub CommandBuildSpec);

impl LuaUserData for CommandBuildSpecLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("build_command", |_, this, ()| {
            Ok(this.0.build_command.clone())
        });
        methods.add_method("install_command", |_, this, ()| {
            Ok(this.0.install_command.clone())
        });
    }
}

pub struct BuildBackendSpecLua(pub BuildBackendSpec);

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

#[derive(Debug, Clone)]
pub struct InstallSpecLua(pub InstallSpec);

impl LuaUserData for InstallSpecLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("lua", |_, this, ()| {
            Ok(this
                .0
                .lua
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), v.clone()))
                .collect::<HashMap<String, PathBuf>>())
        });
        methods.add_method("lib", |_, this, ()| Ok(this.0.lib.clone()));
        methods.add_method("conf", |_, this, ()| Ok(this.0.conf.clone()));
        methods.add_method("bin", |_, this, ()| Ok(this.0.bin.clone()));
    }
}

#[derive(Debug, Clone)]
pub struct BuildSpecLua(pub BuildSpec);

impl LuaUserData for BuildSpecLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("build_backend", |_, this, ()| {
            Ok(this.0.build_backend.clone().map(BuildBackendSpecLua))
        });
        methods.add_method("install", |_, this, ()| {
            Ok(InstallSpecLua(this.0.install.clone()))
        });
        methods.add_method("copy_directories", |_, this, ()| {
            Ok(this.0.copy_directories.clone())
        });
        methods.add_method("patches", |_, this, ()| Ok(this.0.patches.clone()));
    }
}

#[derive(Debug, Clone)]
pub struct LocalLuaRockspecLua(pub LocalLuaRockspec);

impl LuaUserData for LocalLuaRockspecLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
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
}

#[derive(Debug, Clone)]
pub struct RemoteLuaRockspecLua(pub RemoteLuaRockspec);

impl LuaUserData for RemoteLuaRockspecLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
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
}

pub struct PartialLuaRockspecLua(pub PartialLuaRockspec);

impl LuaUserData for PartialLuaRockspecLua {}

#[derive(Debug, Clone)]
pub struct PartialProjectTomlLua(pub PartialProjectToml);

impl LuaUserData for PartialProjectTomlLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("package", |_, this, ()| {
            Ok(PackageNameLua(this.0.package().clone()))
        });
        methods.add_method("to_local", |_, this, ()| {
            this.0.into_local().map(LocalProjectTomlLua).into_lua_err()
        });
        methods.add_method("to_remote", |_, this, specrev: Option<SpecRevLua>| {
            this.0
                .into_remote(specrev.map(|s| s.0))
                .map(RemoteProjectTomlLua)
                .into_lua_err()
        });
    }
}
impl_from_lua_userdata!(PartialProjectTomlLua);

pub struct LocalProjectTomlLua(pub LocalProjectToml);

impl LuaUserData for LocalProjectTomlLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
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
}

pub struct RemoteProjectTomlLua(pub RemoteProjectToml);

impl LuaUserData for RemoteProjectTomlLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
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
}

#[derive(Debug, Clone)]
pub struct RemotePackageDBLua(pub RemotePackageDB);

impl LuaUserData for RemotePackageDBLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
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
        methods.add_method("latest_match", |_, this, package_req: PackageReqLua| {
            Ok(this
                .0
                .latest_match(&package_req.0, None)
                .map(PackageSpecLua))
        });
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

#[derive(Clone)]
pub struct LockfileReadOnlyLua(pub Lockfile<ReadOnly>);

impl LuaUserData for LockfileReadOnlyLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
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
        methods.add_method("get", |_, this, id: LocalPackageIdLua| {
            Ok(this.0.get(&id.0).cloned().map(LocalPackageLua))
        });
        methods.add_method("map_then_flush", |_, this, f: LuaFunction| {
            let lockfile = this.0.clone().write_guard();
            f.call::<()>(LockfileGuardLua(lockfile))?;
            Ok(())
        });
    }
}

pub struct LockfileGuardLua(pub LockfileGuard);

impl LuaUserData for LockfileGuardLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
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
        methods.add_method("get", |_, this, id: LocalPackageIdLua| {
            Ok(this.0.get(&id.0).cloned().map(LocalPackageLua))
        });
    }
}

#[derive(Clone)]
pub struct LockfileReadWriteLua(pub Lockfile<ReadWrite>);

impl LuaUserData for LockfileReadWriteLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
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
        methods.add_method("get", |_, this, id: String| {
            Ok(this
                .0
                .get(unsafe { &LocalPackageId::from_unchecked(id) })
                .cloned()
                .map(LocalPackageLua))
        });
    }
}

#[derive(Debug, Clone)]
pub struct ProjectLua(pub Project);

impl LuaUserData for ProjectLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("toml_path", |_, this, ()| Ok(this.0.toml_path()));
        methods.add_method("luarc_path", |_, this, ()| Ok(this.0.luarc_path()));
        methods.add_method("extra_rockspec_path", |_, this, ()| {
            Ok(this.0.extra_rockspec_path())
        });
        methods.add_method("lockfile_path", |_, this, ()| Ok(this.0.lockfile_path()));
        methods.add_method("root", |_, this, ()| Ok(this.0.root().as_ref().to_owned()));
        methods.add_method("toml", |_, this, ()| {
            Ok(PartialProjectTomlLua(this.0.toml().clone()))
        });
        methods.add_method("local_rockspec", |_, this, ()| {
            this.0
                .local_rockspec()
                .map(LocalLuaRockspecLua)
                .into_lua_err()
        });
        methods.add_method("remote_rockspec", |_, this, specrev: Option<SpecRevLua>| {
            this.0
                .remote_rockspec(specrev.map(|s| s.0))
                .map(RemoteLuaRockspecLua)
                .into_lua_err()
        });
        methods.add_method("tree", |_, this, config: ConfigLua| {
            this.0.tree(&config.0).map(TreeLua).into_lua_err()
        });
        methods.add_method("test_tree", |_, this, config: ConfigLua| {
            this.0.test_tree(&config.0).map(TreeLua).into_lua_err()
        });
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
        methods.add_async_method_mut(
            "add",
            |_, mut this, (deps, config): (DependencyTypeLua<PackageReqLua>, ConfigLua)| async move {
                let _guard = lux_lib::lua::lua_runtime().enter();
                let deps = map_dependency_type(deps.0);
                let package_db =
                    RemotePackageDB::from_config(&config.0, &Progress::<ProgressBar>::no_progress())
                        .await
                        .into_lua_err()?;
                this.0.add(deps, &package_db).await.into_lua_err()
            },
        );
        methods.add_async_method_mut(
            "remove",
            |_, mut this, deps: DependencyTypeLua<PackageNameLua>| async move {
                let _guard = lux_lib::lua::lua_runtime().enter();
                let deps = map_dependency_type_names(deps.0);
                this.0.remove(deps).await.into_lua_err()
            },
        );
        methods.add_method("project_files", |_, this, ()| Ok(this.0.project_files()));
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

impl Into<EntryType> for EntryTypeLua {
    fn into(self) -> EntryType {
        match self {
            EntryTypeLua::Entrypoint => EntryType::Entrypoint,
            EntryTypeLua::DependencyOnly => EntryType::DependencyOnly,
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

impl Into<BuildBehaviour> for BuildBehaviourLua {
    fn into(self) -> BuildBehaviour {
        match self {
            Self::NoForce => BuildBehaviour::NoForce,
            Self::Force => BuildBehaviour::Force,
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

pub struct PackageInstallSpecLua(pub PackageInstallSpec);

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

pub struct SyncReportLua(pub SyncReport);

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

#[derive(Clone)]
pub struct DownloadedRockspecLua(pub DownloadedRockspec);

impl LuaUserData for DownloadedRockspecLua {
    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("rockspec", |_, this, ()| {
            Ok(RemoteLuaRockspecLua(this.0.rockspec.clone()))
        });
    }
}
impl_from_lua_userdata!(DownloadedRockspecLua);
