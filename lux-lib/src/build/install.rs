use std::{
    path::{Path, PathBuf},
    process::Stdio,
    string::FromUtf8Error,
};

use itertools::Itertools;
use miette::Diagnostic;
use path_slash::PathExt;
use thiserror::Error;
use tokio::process::Command;
use walkdir::{DirEntry, WalkDir};
use which::which;

use crate::{
    config::Config,
    fs, lua,
    path::{self, Paths},
    rockspec,
    tree::InstallTree,
};

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Paths(#[from] path::Error),
    #[error(transparent)]
    Utf8(#[from] FromUtf8Error),
    #[error("no Lua executable found")]
    NoLuaBinary {
        #[help]
        help: String,
    },
}

/// Copies a lua source file to a specific destination. The destination is described by a
/// `module.path` syntax (equivalent to the syntax provided to Lua's `require()` function).
#[tracing::instrument(
    name = "Copying Lua module",
    level = "trace",
    skip_all,
    fields(
        source = source.to_slash_lossy().to_string(),
        target = target_module.to_string(),
        target_dir = target_dir.to_slash_lossy().to_string(),
    ),
)]
pub(crate) fn lua_module(
    source: &PathBuf,
    target_module: &rockspec::build::Module,
    target_dir: &Path,
) -> Result<(), fs::Error> {
    // Some rockspecs abuse `install.lua` to install non-lua files, such as teal (`.tl`) files.
    let target_module_path = source
        .extension()
        .map(|ext| target_module.to_file_path(&format!(".{}", ext.to_string_lossy().as_ref())))
        .unwrap_or_else(|| target_module.to_lua_path());
    let target = target_dir.join(target_module_path);

    if let Some(parent) = target.parent() {
        fs::sync::create_dir_all(parent)?;
    }

    fs::sync::copy(source, &target)?;
    fs::make_writable_sync(&target)?;

    Ok(())
}

/// Returns the file path of the installed binary
#[tracing::instrument(
    name = "Installing binary",
    level = "trace",
    skip_all,
    fields(
        source = source.to_slash_lossy().to_string(),
        target,
    ),
)]
pub(crate) async fn binary(
    source: &Path,
    target: &str,
    tree: &impl InstallTree,
    lua: &lua::Installation,
    deploy: &rockspec::DeploySpec,
    config: &Config,
) -> Result<PathBuf, Error> where
{
    fs::tokio::create_dir_all(&tree.bin()).await?;
    let paths = Paths::new(tree)?;
    let script = if config.wrap_bin_scripts()
        && deploy.wrap_bin_scripts
        && is_compatible_lua_script(source, lua, &paths, config).await
    {
        wrapped_binary(source, target, tree, lua, config).await?
    } else {
        let target = tree.bin().join(target);
        fs::tokio::copy(source, &target).await?;
        target
    };

    #[cfg(unix)]
    fs::set_executable_permissions(&script).await?;

    Ok(script)
}

#[tracing::instrument("Installing wrapped binary", level = "trace", skip(tree))]
async fn wrapped_binary(
    source: &Path,
    target: &str,
    tree: &impl InstallTree,
    lua: &lua::Installation,
    config: &Config,
) -> Result<PathBuf, Error> {
    let unwrapped_bin_dir = tree.unwrapped_bin();
    fs::tokio::create_dir_all(&unwrapped_bin_dir).await?;
    let unwrapped_bin = unwrapped_bin_dir.join(target);
    fs::tokio::copy(source, &unwrapped_bin).await?;
    fs::make_writable(&unwrapped_bin).await?;

    #[cfg(target_family = "unix")]
    let target = tree.bin().join(target);
    #[cfg(target_family = "windows")]
    let target = tree.bin().join(format!("{}.bat", target));

    let lua_bin = lua.lua_binary_or_config_override(config).ok_or({
        let bin_name = config
            .variables()
            .get("LUA")
            .cloned()
            .or(lua
                .bin()
                .as_ref()
                .map(|bin| bin.to_slash_lossy().to_string()))
            .unwrap_or("lua".to_string());
        Error::NoLuaBinary {
            help: format!(
                "make sure {bin_name} (version {}) is on the PATH or has been installed by Lux",
                lua.version
            ),
        }
    })?;

    #[cfg(target_family = "unix")]
    let content = format!(
        r#"#!/bin/sh

exec "{0}" "{1}" "$@"
"#,
        lua_bin,
        unwrapped_bin.display(),
    );
    #[cfg(target_family = "windows")]
    let content = format!(
        r#"@echo off
setlocal

{0} "{1}" %*

exit /b %ERRORLEVEL%
"#,
        lua_bin,
        unwrapped_bin.display(),
    );

    fs::tokio::write(&target, content).await?;
    Ok(target)
}

/// Tries to load the file with Lua. If the file can be loaded,
/// we treat it as a valid Lua script.
///
/// NOTE: Lua may successfully load very short bash scripts (like `echo "Hello"`).
/// But that's unlikely to be the case in practise.
/// If a script is mistaken for a Lua script, package authors can disable
/// wrapping with the `deploy.wrap_bin_scripts` rockspec config.
#[tracing::instrument(level = "trace")]
async fn is_compatible_lua_script(
    file: &Path,
    lua: &lua::Installation,
    paths: &Paths,
    config: &Config,
) -> bool {
    let lua_path = paths.package_path_prepended().joined();
    let lua_cpath = paths.package_cpath_prepended().joined();
    let path = paths.path_prepended().joined();
    if let Some(lua_bin) = &lua.lua_binary_or_config_override(config) {
        let lua_bin_path = PathBuf::from(lua_bin);
        if lua_bin_path.is_file() || which(lua_bin).is_ok() {
            Command::new(lua_bin)
                .arg("-e")
                .arg(format!(
                    "if loadfile('{}') then os.exit(0) else os.exit(1) end",
                    // On Windows, Lua escapes path separators, so we ensure forward slashes
                    file.to_slash_lossy()
                ))
                .stderr(Stdio::null())
                .stdout(Stdio::null())
                .env("LUA_PATH", lua_path)
                .env("LUA_CPATH", lua_cpath)
                .env("PATH", path)
                .status()
                .await
                .is_ok_and(|status| status.success())
        } else {
            is_compatible_lua_script_fallback(file, lua, config).await
        }
    } else {
        false
    }
}

/// Fallback function for `is_compatible_lua_script`.
/// Uses the bundled Lua interpreter. This may be less reliable,
/// as it cannot load scripts for an incompatible Lua version.
/// But it works for test dependencies like nlua.
#[tracing::instrument(level = "trace")]
async fn is_compatible_lua_script_fallback(
    file: &Path,
    lua_installation: &lua::Installation,
    config: &Config,
) -> bool {
    if let Ok(file_content) = fs::tokio::read_to_string(&file).await {
        let file_content_without_comments = file_content
            .lines()
            .filter(|line| !line.trim_start().starts_with("#"))
            .collect_vec()
            .join("\n");
        lua_installation
            .lua_binary_or_config_override(config)
            .is_some_and(|_| {
                // Try to compile the content without executing it
                ottavino::Lua::core()
                    .try_enter(|ctx| {
                        ottavino::Closure::load(
                            ctx,
                            None,
                            file_content_without_comments.as_bytes(),
                        )?;
                        Ok(())
                    })
                    .is_ok()
            })
    } else {
        false
    }
}

#[tracing::instrument(level = "trace")]
pub(crate) fn detect_binaries(build_dir: &Path) -> Vec<PathBuf> {
    WalkDir::new(build_dir.join("src").join("bin"))
        .into_iter()
        .chain(WalkDir::new(build_dir.join("bin")))
        .filter_map(|file| file.ok())
        .filter(|file| file.clone().into_path().is_file())
        .map(DirEntry::into_path)
        .collect()
}

#[cfg(test)]
mod tests {
    use tokio::process::Command;

    use crate::{config::ConfigBuilder, lua, tree::Tree};

    use super::*;

    #[tokio::test]
    async fn test_is_compatible_lua_script() {
        let lua_version = lua::detect::installed_version().or(Some(lua::Version::Lua51));
        let config = ConfigBuilder::new()
            .unwrap()
            .lua_version(lua_version)
            .build()
            .unwrap();
        let lua_version = config.lua_version().unwrap();
        let lua = lua::Installation::new(lua_version, &config).await.unwrap();
        let valid_script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample_lua_bin_script_valid");
        let temp_dir = assert_fs::TempDir::new().unwrap().to_path_buf();
        let tree = Tree::new(temp_dir, lua_version.clone(), &config).unwrap();
        let paths = Paths::new(&tree).unwrap();
        assert!(is_compatible_lua_script(&valid_script, &lua, &paths, &config).await);
        let invalid_script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample_lua_bin_script_invalid");
        assert!(!is_compatible_lua_script(&invalid_script, &lua, &paths, &config).await);
    }

    #[tokio::test]
    async fn test_wrapped_binary() {
        let lua_version = lua::detect::installed_version().or(Some(lua::Version::Lua51));
        let temp = assert_fs::TempDir::new().unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .lua_version(lua_version)
            .user_tree(Some(temp.to_path_buf()))
            .build()
            .unwrap();
        let lua_version = config.lua_version().unwrap();
        let lua = lua::Installation::new(lua_version, &config).await.unwrap();
        let tree = config.user_tree(lua_version.clone()).unwrap();
        let valid_script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample_lua_bin_script_valid");
        let script_name = "test_script";
        let script_path = wrapped_binary(&valid_script, script_name, &tree, &lua, &config)
            .await
            .unwrap();

        #[cfg(unix)]
        fs::set_executable_permissions(&script_path).await.unwrap();

        assert!(Command::new(script_path.to_string_lossy().to_string())
            .status()
            .await
            .is_ok_and(|status| status.success()));
    }
}
