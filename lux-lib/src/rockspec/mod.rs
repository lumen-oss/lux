use std::{
    collections::HashMap,
    fmt::Display,
    ops::{Deref, DerefMut},
    path::PathBuf,
};

use itertools::Itertools;
use lua_dependency::LuaDependencySpec;
use mlua::IntoLua;
use serde::{Deserialize, Serialize};
pub mod lua_dependency;

use crate::{
    config::{Config, LuaVersion},
    lua_rockspec::{
        BuildSpec, DeploySpec, ExternalDependencySpec, LuaVersionError, PerPlatform,
        PlatformSupport, RemoteRockSource, RockDescription, RockspecFormat, TestSpec,
    },
    package::{PackageName, PackageVersion, PackageVersionReq},
};

pub trait Rockspec {
    type Error: Display + std::fmt::Debug;

    fn package(&self) -> &PackageName;
    fn version(&self) -> &PackageVersion;
    fn description(&self) -> &RockDescription;
    fn supported_platforms(&self) -> &PlatformSupport;
    fn lua(&self) -> &PackageVersionReq;
    fn dependencies(&self) -> &PerPlatform<Vec<LuaDependencySpec>>;
    fn build_dependencies(&self) -> &PerPlatform<Vec<LuaDependencySpec>>;
    fn external_dependencies(&self) -> &PerPlatform<HashMap<String, ExternalDependencySpec>>;
    fn test_dependencies(&self) -> &PerPlatform<Vec<LuaDependencySpec>>;

    fn build(&self) -> &PerPlatform<BuildSpec>;
    fn test(&self) -> &PerPlatform<TestSpec>;
    fn source(&self) -> &PerPlatform<RemoteRockSource>;
    fn deploy(&self) -> &PerPlatform<DeploySpec>;

    fn build_mut(&mut self) -> &mut PerPlatform<BuildSpec>;
    fn test_mut(&mut self) -> &mut PerPlatform<TestSpec>;
    fn source_mut(&mut self) -> &mut PerPlatform<RemoteRockSource>;
    fn deploy_mut(&mut self) -> &mut PerPlatform<DeploySpec>;

    fn format(&self) -> &Option<RockspecFormat>;

    /// Shorthand to extract the binaries that are part of the rockspec.
    fn binaries(&self) -> RockBinaries {
        RockBinaries(
            self.build()
                .current_platform()
                .install
                .bin
                .keys()
                .map_into()
                .collect(),
        )
    }

    /// Converts the rockspec to a string that can be uploaded to a luarocks server.
    fn to_lua_remote_rockspec_string(&self) -> Result<String, Self::Error>;
}

pub trait LuaVersionCompatibility {
    /// Ensures that the rockspec is compatible with the specified lua version.
    /// Returns an error if the rockspec is not compatible.
    fn validate_lua_version(&self, lua_version: &LuaVersion) -> Result<(), LuaVersionError>;

    /// Ensures that the rockspec is compatible with the lua version established in the config.
    /// Returns an error if the rockspec is not compatible.
    fn validate_lua_version_from_config(&self, config: &Config) -> Result<(), LuaVersionError>;

    /// Ensures that the rockspec is compatible with the lua version established in the config,
    /// and returns the lua version from the config if it is compatible.
    fn lua_version_matches(&self, config: &Config) -> Result<LuaVersion, LuaVersionError>;

    /// Checks if the rockspec supports the given lua version.
    fn supports_lua_version(&self, lua_version: &LuaVersion) -> bool;

    /// Returns the lua version required by the rockspec.
    fn lua_version(&self) -> Option<LuaVersion>;
}

impl<T: Rockspec> LuaVersionCompatibility for T {
    fn validate_lua_version(&self, version: &LuaVersion) -> Result<(), LuaVersionError> {
        if self.supports_lua_version(version) {
            Ok(())
        } else {
            Err(LuaVersionError::LuaVersionUnsupported(
                version.clone(),
                self.package().to_owned(),
                self.version().to_owned(),
            ))
        }
    }

    fn validate_lua_version_from_config(&self, config: &Config) -> Result<(), LuaVersionError> {
        let _ = self.lua_version_matches(config)?;
        Ok(())
    }

    fn lua_version_matches(&self, config: &Config) -> Result<LuaVersion, LuaVersionError> {
        let version = LuaVersion::from_config(config)?.clone();
        if self.supports_lua_version(&version) {
            Ok(version)
        } else {
            Err(LuaVersionError::LuaVersionUnsupported(
                version,
                self.package().to_owned(),
                self.version().to_owned(),
            ))
        }
    }

    fn supports_lua_version(&self, lua_version: &LuaVersion) -> bool {
        self.lua().matches(&lua_version.as_version())
    }

    fn lua_version(&self) -> Option<LuaVersion> {
        for (possibility, version) in [
            ("5.4.0", LuaVersion::Lua54),
            ("5.3.0", LuaVersion::Lua53),
            ("5.2.0", LuaVersion::Lua52),
            ("5.1.0", LuaVersion::Lua51),
        ] {
            let possibility = unsafe { possibility.parse().unwrap_unchecked() };
            if self.lua().matches(&possibility) {
                return Some(version);
            }
        }
        None
    }
}

#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialOrd, Ord, Hash, PartialEq, Eq)]
pub struct RockBinaries(Vec<PathBuf>);

impl Deref for RockBinaries {
    type Target = Vec<PathBuf>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for RockBinaries {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl IntoLua for RockBinaries {
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        self.0.into_lua(lua)
    }
}
