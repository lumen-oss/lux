use crate::package;
use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
    path::PathBuf,
};

use itertools::Itertools;
use miette::Diagnostic;
use serde::{Deserialize, Serialize};
use thiserror::Error;
pub mod build;
mod deploy;
mod description;
mod display;
mod external_dependency;
mod format;
pub mod lua;
mod lua_dependency;
mod platform;
mod rock_source;
mod serde_util;
mod test_spec;
pub mod toml;

pub use deploy::*;
pub use description::*;
pub use external_dependency::*;
pub use format::*;
pub use lua_dependency::*;
pub use platform::*;
pub use rock_source::*;
pub use test_spec::*;

pub(crate) use display::*;
pub(crate) use lux_macros::DisplayAsLuaKV;
pub(crate) use serde_util::*;

use crate::config::Config;

use self::build::BuildSpec;

#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Lua(Box<lua::Error>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    ProjectToml(Box<toml::Error>),
    #[error(
        r#"
The lua version {0} is not supported by {1} version {2}.

HINT: If Lux has auto-detected an incompatible Lua installation,
      use `--lua-version` to specify the Lua version to use.
      Valid versions are: '5.1', '5.2', '5.3', '5.4', '5.5', 'jit', 'jit52' and 'luau'.
"#
    )]
    UnsupportedLuaVersion(crate::lua::Version, package::Name, Box<package::Version>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaVersion(#[from] crate::lua::version::Error),
}

impl From<lua::Error> for Error {
    fn from(err: lua::Error) -> Self {
        Self::Lua(Box::new(err))
    }
}

impl From<toml::Error> for Error {
    fn from(err: toml::Error) -> Self {
        Self::ProjectToml(Box::new(err))
    }
}

pub trait Rockspec {
    fn package(&self) -> &package::Name;
    fn version(&self) -> &package::Version;
    fn description(&self) -> &RockDescription;
    fn supported_platforms(&self) -> &PlatformSupport;
    fn lua(&self) -> &package::VersionReq;
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

    fn format(&self) -> &Option<Format>;

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
    fn to_lua_remote_rockspec_string(&self) -> Result<String, Error>;
}

pub trait LuaVersionCompatibility {
    /// Ensures that the rockspec is compatible with the specified lua version.
    /// Returns an error if the rockspec is not compatible.
    fn validate_lua_version(&self, lua_version: &crate::lua::Version) -> Result<(), Error>;

    /// Ensures that the rockspec is compatible with the lua version established in the config.
    /// Returns an error if the rockspec is not compatible.
    fn validate_lua_version_from_config(&self, config: &Config) -> Result<(), Error>;

    /// Ensures that the rockspec is compatible with the lua version established in the config,
    /// and returns the lua version from the config if it is compatible.
    fn lua_version_matches(&self, config: &Config) -> Result<crate::lua::Version, Error>;

    /// Checks if the rockspec supports the given lua version.
    fn supports_lua_version(&self, lua_version: &crate::lua::Version) -> bool;

    /// Returns the lua version required by the rockspec.
    fn lua_version(&self) -> Option<crate::lua::Version>;
}

impl<T: Rockspec> LuaVersionCompatibility for T {
    fn validate_lua_version(&self, version: &crate::lua::Version) -> Result<(), Error> {
        if self.supports_lua_version(version) {
            Ok(())
        } else {
            Err(Error::UnsupportedLuaVersion(
                version.clone(),
                self.package().to_owned(),
                Box::new(self.version().to_owned()),
            ))
        }
    }

    fn validate_lua_version_from_config(&self, config: &Config) -> Result<(), Error> {
        let _ = self.lua_version_matches(config)?;
        Ok(())
    }

    fn lua_version_matches(&self, config: &Config) -> Result<crate::lua::Version, Error> {
        let version = crate::lua::Version::from(config)?.clone();
        if self.supports_lua_version(&version) {
            Ok(version)
        } else {
            Err(Error::UnsupportedLuaVersion(
                version,
                self.package().to_owned(),
                Box::new(self.version().to_owned()),
            ))
        }
    }

    fn supports_lua_version(&self, lua_version: &crate::lua::Version) -> bool {
        if self.lua().is_luau() {
            return matches!(lua_version, crate::lua::Version::Luau);
        }
        self.lua().matches(&lua_version.as_version())
    }

    fn lua_version(&self) -> Option<crate::lua::Version> {
        if self.lua().is_luau() {
            return Some(crate::lua::Version::Luau);
        }
        for (possibility, version) in [
            ("5.5.0", crate::lua::Version::Lua54),
            ("5.4.0", crate::lua::Version::Lua54),
            ("5.3.0", crate::lua::Version::Lua53),
            ("5.2.0", crate::lua::Version::Lua52),
            ("5.1.0", crate::lua::Version::Lua51),
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
