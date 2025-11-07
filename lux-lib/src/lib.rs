pub mod build;
pub mod config;
pub mod dependencies;
pub mod git;
pub mod hash;
pub mod lockfile;
pub mod lua;
pub mod lua_installation;
pub mod lua_rockspec;
pub mod luarocks;
pub mod manifest;
pub mod operations;
pub mod package;
pub mod path;
pub mod progress;
pub mod project;
pub mod remote_package_db;
pub mod rockspec;
pub mod tree;
pub mod upload;
pub mod which;

pub(crate) mod remote_package_source;
pub(crate) mod variables;

/// An internal string describing the server-side API version that we support.
/// Whenever we connect to a server (like `luarocks.org`), we ensure that these
/// two versions match (meaning we can safely communicate with the server).
pub const TOOL_VERSION: &str = "1.0.0";
