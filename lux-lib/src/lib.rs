pub mod build;
pub mod config;
pub mod git;
pub mod hash;
pub mod lockfile;
pub mod lua;
pub mod lua_installation;
pub mod lua_rockspec;
pub mod lua_version;
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

/// User-Agent string sent on all outbound HTTP traffic to luarocks-compatible servers.
pub const USER_AGENT: &str = concat!("lux/", env!("CARGO_PKG_VERSION"));

/// Returns a reqwest::Client preconfigured with the lux User-Agent header.
/// Use this for every outbound HTTP request to a luarocks server so the
/// upstream operator can see adoption / version distribution.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .expect("failed to build default reqwest client")
}

// The largest known files (Lua manifests) use up roughly ~500k steps.
pub const ROCKSPEC_FUEL_LIMIT: i32 = 1_000_000;
