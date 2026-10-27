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
    lockfile::LockedPackageSpec,
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
    package: &LockedPackageSpec,
    config: &Config,
) -> Result<BuildInfo, LuarocksBuildError> {
    let layout = tree.layout_for(package);
    let lua_version = rockspec.lua_version_matches(config)?;
    fs::tokio::create_dir_all(tree.bin()).await?;
    let lua_version = lua_version.version_compatibility_str();
    let package_dir = luarocks_tree
        .join("lib")
        .join("luarocks")
        .join(format!("rocks-{}", lua_version))
        .join(format!("{}", rockspec.package()))
        .join(format!("{}", rockspec.version()));
    let rockspec_file_name = format!("{}-{}.rockspec", rockspec.package(), rockspec.version());
    copy_package_dir(&package_dir, &layout, &rockspec_file_name).await?;
    recursive_copy_dir(&luarocks_tree.join("bin"), &tree.bin()).await?;
    let src_dir = luarocks_tree.join("share").join("lua").join(&lua_version);
    recursive_copy_dir(&src_dir, &layout.src).await?;
    let lib_dir = luarocks_tree.join("lib").join("lua").join(&lua_version);
    recursive_copy_dir(&lib_dir, &layout.lib).await?;
    Ok(BuildInfo::default())
}

/// Copy the contents of luarocks' per-rock directory into the lux tree layout:
/// `doc` and `conf` map onto their dedicated layout directories, everything
/// else onto the rock's `etc` root (mirroring a binary rock's root entries).
/// luarocks' own bookkeeping files (`rock_manifest` and the installed
/// rockspec) are skipped, as lux generates its own.
async fn copy_package_dir(
    package_dir: &Path,
    layout: &crate::tree::RockLayout,
    rockspec_file_name: &str,
) -> Result<(), fs::FsError> {
    if !package_dir.is_dir() {
        return Ok(());
    }
    for entry in fs::sync::read_dir(package_dir)? {
        let entry = entry?;
        let src = entry.path();
        let fname = entry.file_name();
        let name = fname.to_string_lossy();
        if name == "rock_manifest" || name == rockspec_file_name {
            continue;
        }
        if src.is_file() {
            // Loose files live at the rock's `etc` root, under their own name.
            fs::tokio::create_dir_all(&layout.etc).await?;
            fs::tokio::copy(&src, &layout.etc.join(name.as_ref())).await?;
            continue;
        }
        let dest = match name.as_ref() {
            "doc" => layout.doc.clone(),
            "conf" => layout.conf.clone(),
            other => layout.etc.join(other),
        };
        recursive_copy_dir(&src, &dest).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::RockLayout;

    #[tokio::test]
    async fn copy_package_dir_maps_toolchain_artefacts_into_layout() {
        let tmp = fs::tempfile::tempdir().unwrap();
        let package_dir = tmp.path().join("rocks-5.1/foobar/dev-1");
        fs::sync::create_dir_all(package_dir.join("conf")).unwrap();
        fs::sync::write(package_dir.join("conf/x.conf"), b"-- conf").unwrap();
        fs::sync::create_dir_all(package_dir.join("doc")).unwrap();
        fs::sync::write(package_dir.join("doc/readme.md"), b"-- doc").unwrap();
        // Loose file and directory that must land at the rock's `etc` root.
        fs::sync::write(package_dir.join("foo.txt"), b"-- txt").unwrap();
        fs::sync::create_dir_all(package_dir.join("plugin")).unwrap();
        fs::sync::write(package_dir.join("plugin/foo.vim"), b"-- vim").unwrap();
        // luarocks bookkeeping files that must be skipped.
        fs::sync::write(package_dir.join("rock_manifest"), b"manifest").unwrap();
        fs::sync::write(package_dir.join("foobar-dev-1.rockspec"), b"rockspec").unwrap();

        let layout_root = tmp.path().join("layout/foobar/dev-1");
        let bin_dir = tmp.path().join("layout/bin");
        let layout = RockLayout::new(layout_root.clone(), bin_dir.clone());

        copy_package_dir(&package_dir, &layout, "foobar-dev-1.rockspec")
            .await
            .unwrap();

        assert!(layout.doc.join("readme.md").is_file());
        assert!(layout.conf.join("x.conf").is_file());
        assert!(layout.etc.join("foo.txt").is_file());
        assert!(layout.etc.join("plugin/foo.vim").is_file());
        assert!(!layout.etc.join("rock_manifest").exists());
        assert!(!layout.etc.join("foobar-dev-1.rockspec").exists());
    }
}
