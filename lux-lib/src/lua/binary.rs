use std::fmt;
use std::fmt::Display;
use std::path::PathBuf;

use super::version::Version;
use crate::config::Config;

#[derive(Clone)]
pub enum Binary {
    /// The regular Lua interpreter.
    Lua { version: Version },
    /// Custom Lua interpreter.
    Custom(String),
}

impl Display for Binary {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Binary::Lua { version } => write!(f, "lua {version}"),
            Binary::Custom(cmd) => write!(f, "{cmd}"),
        }
    }
}

impl Binary {
    /// Construct a new `Binary` for the given `Version`,
    /// potentially prioritising an overridden value in the config.
    pub fn new(version: Version, config: &Config) -> Self {
        let variable = if version.is_luau() { "LUAU" } else { "LUA" };
        match config.variables().get(variable).cloned() {
            Some(bin) => Self::Custom(bin),
            None => Self::Lua { version },
        }
    }
}

impl From<PathBuf> for Binary {
    fn from(value: PathBuf) -> Self {
        Self::Custom(value.to_string_lossy().to_string())
    }
}
