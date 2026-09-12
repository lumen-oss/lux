use crate::build::backend::BuildInfo;
use crate::build::backend::RunBuildArgs;
use crate::fs;
use crate::lua_rockspec::LuaVersionError;
use crate::rockspec::LuaVersionCompatibility;
use crate::rockspec::Rockspec;
use crate::tree::InstallTree;
use crate::tree::TreeError;
use std::path::Path;

use crate::{
    config::Config,
    lockfile::LocalPackage,
    luarocks::luarocks_installation::{ExecLuaRocksError, LuaRocksError, LuaRocksInstallation},
};

use super::utils::recursive_copy_dir;
use crate::fs::tempfile::tempdir;
use miette::Diagnostic;
use thiserror::Error;

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum LuarocksBuildError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::FsError),
    #[error("error instantiating luarocks compatibility layer")]
    #[diagnostic(forward(0))]
    LuaRocksError(#[from] LuaRocksError),
    #[error("error running 'luarocks make'")]
    #[diagnostic(forward(0))]
    ExecLuaRocksError(#[from] ExecLuaRocksError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Rockspec(Box<dyn Diagnostic + Send + Sync>),
    #[error("error installing luarocks compatibility layer")]
    #[diagnostic(forward(0))]
    LuaVersion(#[from] LuaVersionError),
}

#[tracing::instrument(name = "Delegating to luarocks",
    skip_all,
    level = "info"
    fields(backend = build_backend_name),
)]
pub(crate) async fn build<R: Rockspec, T: InstallTree>(
    build_backend_name: &str,
    rockspec: &R,
    args: RunBuildArgs<'_, T>,
) -> Result<BuildInfo, LuarocksBuildError> {
    let package = args.package;
    let lua = args.lua;
    let config = args.config;
    let build_dir = args.build_dir;
    let tree = args.tree;

    let rockspec_temp_dir = tempdir()?;
    let rockspec_file = rockspec_temp_dir.path().join(format!(
        "{}-{}.rockspec",
        rockspec.package(),
        rockspec.version()
    ));
    fs::tokio::write(
        &rockspec_file,
        rockspec
            .to_lua_remote_rockspec_string()
            .map_err(|err| LuarocksBuildError::Rockspec(Box::new(err)))?,
    )
    .await?;
    let luarocks = LuaRocksInstallation::new(config, tree.build_tree(config)?)?;
    let luarocks_tree = tempdir()?;
    luarocks
        .make(&rockspec_file, build_dir, luarocks_tree.path(), lua)
        .await?;
    install(rockspec, luarocks_tree.path(), tree, package, config).await
}

async fn install<R: Rockspec, T: InstallTree>(
    rockspec: &R,
    luarocks_tree: &Path,
    tree: &T,
    package: &LocalPackage,
    config: &Config,
) -> Result<BuildInfo, LuarocksBuildError> {
    let layout = tree.layout_for(package);
    let lua_version = rockspec.lua_version_matches(config)?;
    fs::tokio::create_dir_all(tree.bin()).await?;
    let lua_version = lua_version.version_compatibility_str();
    let package_dir = luarocks_tree
        .join("lib")
        .join("lib")
        .join("luarocks")
        .join(format!("lux-{}", lua_version))
        .join(format!("{}", rockspec.package()))
        .join(format!("{}", rockspec.version()));
    recursive_copy_dir(&package_dir.join("doc"), &layout.doc).await?;
    recursive_copy_dir(&luarocks_tree.join("bin"), &tree.bin()).await?;
    let src_dir = luarocks_tree.join("share").join("lua").join(&lua_version);
    recursive_copy_dir(&src_dir, &layout.src).await?;
    let lib_dir = luarocks_tree.join("lib").join("lua").join(&lua_version);
    recursive_copy_dir(&lib_dir, &layout.lib).await?;
    Ok(BuildInfo::default())
}
