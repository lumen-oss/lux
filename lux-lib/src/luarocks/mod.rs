use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::{build::utils::recursive_copy_dir, fs, tree::RockLayout};
use miette::Diagnostic;
use thiserror::Error;

use self::rock_manifest::{RockManifest, RockManifestError};

pub mod install_binary_rock;
pub mod luarocks_installation;
pub mod rock_manifest;

/// Retrieves the target compilation platform and returns it as a luarocks identifier.
pub(crate) fn current_platform_luarocks_identifier() -> String {
    let platform = match std::env::consts::OS {
        "macos" => "macosx",
        p => p,
    };
    format!("{}-{}", platform, std::env::consts::ARCH)
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum InstallRockManifestError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error("rock_manifest not found")]
    #[diagnostic(help("the rock may have been packed using LuaRocks version 1"))]
    RockManifestNotFound,
    #[error(transparent)]
    #[diagnostic(transparent)]
    RockManifest(#[from] RockManifestError),
}

pub(crate) struct RockManifestSources {
    pub root: PathBuf,
    pub lib: PathBuf,
    pub lua: PathBuf,
    pub bin: PathBuf,
}

#[tracing::instrument(level = "trace", skip(entry))]
async fn install_manifest_entries<T>(
    entry: &HashMap<PathBuf, T>,
    src: &Path,
    dest: &Path,
) -> Result<(), fs::Error> {
    for relative_src_path in entry.keys() {
        let target = dest.join(relative_src_path);
        let src_path = src.join(relative_src_path);
        if src_path.is_dir() {
            recursive_copy_dir(&src_path, &target).await?;
        } else if src_path.is_file() {
            if let Some(target_parent_dir) = target.parent() {
                fs::tokio::create_dir_all(target_parent_dir).await?;
            }
            fs::tokio::copy(&src_path, target).await?;
        } else {
            return Err(fs::Error::Other(format!(
                "the entry {} listed in the `rock_manifest` is neither a file nor a directory",
                src_path.display()
            )));
        }
    }
    Ok(())
}

async fn install_bin_manifest_entries(
    entries: &HashMap<PathBuf, String>,
    src: &Path,
    dest: &Path,
) -> Result<(), fs::Error> {
    install_manifest_entries(entries, src, dest).await?;
    #[cfg(unix)]
    for relative_src_path in entries.keys() {
        crate::build::utils::set_executable_permissions(&dest.join(relative_src_path)).await?;
    }
    Ok(())
}

pub(crate) async fn install_rock_manifest(
    sources: RockManifestSources,
    layout: &RockLayout,
    bin_dest: &Path,
) -> Result<(), InstallRockManifestError> {
    let rock_manifest_file = sources.root.join("rock_manifest");
    if !rock_manifest_file.is_file() {
        return Err(InstallRockManifestError::RockManifestNotFound);
    }
    let rock_manifest_content = fs::tokio::read_to_string(rock_manifest_file).await?;
    let rock_manifest = RockManifest::new(&rock_manifest_content)?;
    install_manifest_entries(&rock_manifest.lib.entries, &sources.lib, &layout.lib).await?;
    install_manifest_entries(&rock_manifest.lua.entries, &sources.lua, &layout.src).await?;
    install_bin_manifest_entries(&rock_manifest.bin.entries, &sources.bin, bin_dest).await?;
    install_manifest_entries(
        &rock_manifest.doc.entries,
        &sources.root.join("doc"),
        &layout.doc,
    )
    .await?;
    install_manifest_entries(
        &rock_manifest.conf.entries,
        &sources.root.join("conf"),
        &layout.conf,
    )
    .await?;
    install_manifest_entries(&rock_manifest.root.entries, &sources.root, &layout.etc).await?;
    Ok(())
}
