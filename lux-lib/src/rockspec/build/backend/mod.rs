//! Build backends, serialized from the build table.

pub(crate) mod builtin;
mod cmake;
mod make;
mod rust_binary;
mod rust_mlua;
mod treesitter;

pub use builtin::{Builtin, Module, ModulePaths, ModuleSpec, ParseLuaModuleError};
pub use cmake::CMake;
pub use make::Make;
pub use rust_binary::RustBinary;
pub use rust_mlua::RustMlua;
pub use treesitter::Treesitter;

impl Default for Spec {
    fn default() -> Self {
        Self::Builtin(Builtin::default())
    }
}

/// Encodes extra information about each backend.
/// When selecting a backend, one may provide extra parameters
/// to `build = { ... }` in order to further customize the behaviour of the build step.
///
/// Luarocks provides several default build types, these are also reflected in `lux`
/// for compatibility.
#[derive(Debug, PartialEq, Clone)]
pub enum Spec {
    Builtin(Builtin),
    Make(Make),
    CMake(CMake),
    Command(Command),
    LuaRock(String),
    RustMlua(RustMlua),
    RustBinary(RustBinary),
    TreesitterParser(Treesitter),
    /// Build from the source rockspec, if present.
    /// Otherwise, fall back to the builtin build and copy all directories.
    /// This is currently unimplemented by luarocks, but we don't ever publish rockspecs
    /// that implement this.
    /// It could be implemented as a custom build backend.
    Source,
}

impl Spec {
    pub(crate) fn can_use_build_dependencies(&self) -> bool {
        match self {
            Self::Make(_) | Self::CMake(_) | Self::Command(_) | Self::LuaRock(_) => true,
            Self::Builtin(_)
            | Self::RustMlua(_)
            | Self::RustBinary(_)
            | Self::TreesitterParser(_)
            | Self::Source => false,
        }
    }
}

/// Specification for building a rock with the `command` build backend
#[derive(Debug, PartialEq, Clone)]
pub struct Command {
    pub build_command: Option<String>,
    pub install_command: Option<String>,
}
