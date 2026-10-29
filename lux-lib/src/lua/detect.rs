use std::io;
use std::path::Path;

use miette::Diagnostic;
use thiserror::Error;
use which::which;

use super::version::{self, Version};
use crate::package;

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("failed to run {0}: {1}")]
    RunCommand(String, io::Error),
    #[error("failed to parse Lua version from output: {0}")]
    ParseVersion(String),
    #[error(transparent)]
    #[diagnostic(transparent)]
    PackageVersionParse(#[from] package::version::ParseError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Version(#[from] version::Error),
}

pub fn installed_version() -> Option<Version> {
    which("lua").ok().or(which("luajit").ok()).and_then(|cmd| {
        version_from_path(&cmd)
            .ok()
            .and_then(|version| Version::from_version(version).ok())
    })
}

pub(super) fn version_from_path(path: &Path) -> Result<package::Version, Error> {
    let output = match std::process::Command::new(path).arg("-v").output() {
        Ok(output) => Ok(output),
        Err(err) => Err(Error::RunCommand(path.to_string_lossy().to_string(), err)),
    }?;
    let output_vec = if output.stderr.is_empty() {
        output.stdout
    } else {
        // Yes, Lua 5.1 prints to stderr (-‸ლ)
        output.stderr
    };
    let output_str = String::from_utf8_lossy(&output_vec).to_string();
    parse_version_from_output(&output_str)
}

fn parse_version_from_output(output: &str) -> Result<package::Version, Error> {
    let version_str = output
        .trim_start_matches("Lua")
        .trim_start_matches("JIT")
        .split_whitespace()
        .next()
        .map(|s| s.to_string())
        .ok_or(Error::ParseVersion(output.to_string()))?;
    Ok(package::Version::parse(&version_str)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parse_luajit_version() {
        let luajit_output =
            "LuaJIT 2.1.1713773202 -- Copyright (C) 2005-2023 Mike Pall. https://luajit.org/";
        parse_version_from_output(luajit_output).unwrap();
    }

    #[tokio::test]
    async fn parse_lua_51_version() {
        let lua_output = "Lua 5.1.5  Copyright (C) 1994-2012 Lua.org, PUC-Rio";
        parse_version_from_output(lua_output).unwrap();
    }
}
