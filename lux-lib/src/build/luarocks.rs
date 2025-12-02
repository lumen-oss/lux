use crate::build::backend::BuildInfo;
use crate::build::backend::RunBuildArgs;
use crate::lua_rockspec::LuaVersionError;
use crate::rockspec::LuaVersionCompatibility;
use crate::rockspec::Rockspec;
use crate::tree::TreeError;
use std::{io, path::Path};

use crate::{
    config::Config,
    luarocks::luarocks_installation::{ExecLuaRocksError, LuaRocksError, LuaRocksInstallation},
    tree::RockLayout,
};

use tempfile::tempdir;
use thiserror::Error;

use super::utils::recursive_copy_dir;

#[derive(Error, Debug)]
pub enum LuarocksBuildError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("error instantiating luarocks compatibility layer:\n{0}")]
    LuaRocksError(#[from] LuaRocksError),
    #[error("error running 'luarocks make':\n{0}")]
    ExecLuaRocksError(#[from] ExecLuaRocksError),
    #[error(transparent)]
    Tree(#[from] TreeError),
    #[error("{0}")] // We don't know the concrete error type
    Rockspec(String),
    #[error("error installing luarocks compatibility layer:\n{0}")]
    LuaVersion(#[from] LuaVersionError),
}

pub(crate) async fn build<R: Rockspec>(
    rockspec: &R,
    args: RunBuildArgs<'_>,
) -> Result<BuildInfo, LuarocksBuildError> {
    let output_paths = args.output_paths;
    let lua = args.lua;
    let config = args.config;
    let build_dir = args.build_dir;
    let tree = args.tree;
    let progress = args.progress;
    progress.map(|p| {
        p.set_message(format!(
            "Building {} {} with luarocks...",
            rockspec.package(),
            rockspec.version()
        ))
    });
    let rockspec_temp_dir = tempdir()?;
    let rockspec_file = rockspec_temp_dir.path().join(format!(
        "{}-{}.rockspec",
        rockspec.package(),
        rockspec.version()
    ));
    tokio::fs::write(
        &rockspec_file,
        rockspec
            .to_lua_remote_rockspec_string()
            .map_err(|err| LuarocksBuildError::Rockspec(err.to_string()))?,
    )
    .await?;
    let luarocks = LuaRocksInstallation::new(config, tree.build_tree(config)?)?;
    let luarocks_tree = tempdir()?;
    luarocks
        .make(&rockspec_file, build_dir, luarocks_tree.path(), lua)
        .await?;
    install(rockspec, luarocks_tree.path(), output_paths, config).await
}

async fn install<R: Rockspec>(
    rockspec: &R,
    luarocks_tree: &Path,
    output_paths: &RockLayout,
    config: &Config,
) -> Result<BuildInfo, LuarocksBuildError> {
    let lua_version = rockspec.lua_version_matches(config)?;
    std::fs::create_dir_all(&output_paths.bin)?;
    let package_dir = luarocks_tree
        .join("lib")
        .join("lib")
        .join("luarocks")
        .join(format!("lux-{}", &lua_version.version_compatibility_str()))
        .join(format!("{}", rockspec.package()))
        .join(format!("{}", rockspec.version()));
    recursive_copy_dir(&package_dir.join("doc"), &output_paths.doc).await?;
    recursive_copy_dir(&luarocks_tree.join("bin"), &output_paths.bin).await?;
    let src_dir = luarocks_tree
        .join("share")
        .join("lua")
        .join(lua_version.version_compatibility_str());
    recursive_copy_dir(&src_dir, &output_paths.src).await?;
    let lib_dir = luarocks_tree
        .join("lib")
        .join("lua")
        .join(lua_version.version_compatibility_str());
    recursive_copy_dir(&lib_dir, &output_paths.lib).await?;
    Ok(BuildInfo::default())
}
