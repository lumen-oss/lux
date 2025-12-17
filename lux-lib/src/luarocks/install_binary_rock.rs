use std::{
    collections::HashMap,
    io::{self, Cursor},
    path::{Path, PathBuf},
};

use bytes::Bytes;
use tempfile::tempdir;
use thiserror::Error;

use crate::{
    build::{
        external_dependency::{ExternalDependencyError, ExternalDependencyInfo},
        utils::recursive_copy_dir,
        BuildBehaviour,
    },
    config::Config,
    hash::HasIntegrity,
    lockfile::{
        LocalPackage, LocalPackageHashes, LockConstraint, LockfileError, OptState, PinnedState,
    },
    lua_rockspec::{LuaVersionError, RemoteLuaRockspec},
    luarocks::rock_manifest::RockManifest,
    package::PackageSpec,
    progress::{Progress, ProgressBar},
    remote_package_source::RemotePackageSource,
    rockspec::Rockspec,
    tree::{self, Tree, TreeError},
};
use crate::{lockfile::RemotePackageSourceUrl, rockspec::LuaVersionCompatibility};

use super::rock_manifest::RockManifestError;

#[derive(Error, Debug)]
pub enum InstallBinaryRockError {
    #[error("IO operation failed: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Lockfile(#[from] LockfileError),
    #[error(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    ExternalDependencyError(#[from] ExternalDependencyError),
    #[error(transparent)]
    LuaVersionError(#[from] LuaVersionError),
    #[error("failed to unpack packed rock: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("rock_manifest not found. Cannot install rock files that were packed using LuaRocks version 1")]
    RockManifestNotFound,
    #[error(transparent)]
    RockManifestError(#[from] RockManifestError),
    #[error(
        "the entry {0} listed in the `rock_manifest` is neither a file nor a directory: {1:?}"
    )]
    NotAFileOrDirectory(String, std::fs::Metadata),
}

pub(crate) struct BinaryRockInstall<'a> {
    rockspec: &'a RemoteLuaRockspec,
    rock_bytes: Bytes,
    source: RemotePackageSource,
    pin: PinnedState,
    opt: OptState,
    entry_type: tree::EntryType,
    constraint: LockConstraint,
    behaviour: BuildBehaviour,
    config: &'a Config,
    tree: &'a Tree,
    progress: &'a Progress<ProgressBar>,
}

impl<'a> BinaryRockInstall<'a> {
    pub(crate) fn new(
        rockspec: &'a RemoteLuaRockspec,
        source: RemotePackageSource,
        rock_bytes: Bytes,
        entry_type: tree::EntryType,
        config: &'a Config,
        tree: &'a Tree,
        progress: &'a Progress<ProgressBar>,
    ) -> Self {
        Self {
            rockspec,
            rock_bytes,
            source,
            config,
            tree,
            progress,
            constraint: LockConstraint::default(),
            behaviour: BuildBehaviour::default(),
            pin: PinnedState::default(),
            opt: OptState::default(),
            entry_type,
        }
    }

    pub(crate) fn pin(self, pin: PinnedState) -> Self {
        Self { pin, ..self }
    }

    pub(crate) fn opt(self, opt: OptState) -> Self {
        Self { opt, ..self }
    }

    pub(crate) fn constraint(self, constraint: LockConstraint) -> Self {
        Self { constraint, ..self }
    }

    pub(crate) fn behaviour(self, behaviour: BuildBehaviour) -> Self {
        Self { behaviour, ..self }
    }

    pub(crate) async fn install(self) -> Result<LocalPackage, InstallBinaryRockError> {
        let rockspec = self.rockspec;
        self.progress.map(|p| {
            p.set_message(format!(
                "Unpacking and installing {}@{}...",
                rockspec.package(),
                rockspec.version()
            ))
        });
        for (name, dep) in rockspec.external_dependencies().current_platform() {
            let _ = ExternalDependencyInfo::probe(name, dep, self.config.external_deps())?;
        }

        rockspec.validate_lua_version_from_config(self.config)?;

        let hashes = LocalPackageHashes {
            rockspec: rockspec.hash()?,
            source: self.rock_bytes.hash()?,
        };
        let source_url = match &self.source {
            RemotePackageSource::LuarocksBinaryRock(url) => {
                Some(RemotePackageSourceUrl::Url { url: url.clone() })
            }
            _ => None,
        };
        let mut package = LocalPackage::from(
            &PackageSpec::new(rockspec.package().clone(), rockspec.version().clone()),
            self.constraint,
            rockspec.binaries(),
            self.source,
            source_url,
            hashes,
        );
        package.spec.pinned = self.pin;
        package.spec.opt = self.opt;
        match self.tree.lockfile()?.get(&package.id()) {
            Some(package) if self.behaviour == BuildBehaviour::NoForce => Ok(package.clone()),
            _ => {
                let unpack_dir = tempdir()?;
                let cursor = Cursor::new(self.rock_bytes);
                let mut zip = zip::ZipArchive::new(cursor)?;
                zip.extract(&unpack_dir)?;
                // let lua_dir = unpack_dir.join("lua");
                // if lua_dir.is_dir() {
                //     let src_dir = unpack_dir.join("lua");
                //     tokio::fs::rename(lua_dir, src_dir).await?;
                // }
                let rock_manifest_file = unpack_dir.path().join("rock_manifest");
                if !rock_manifest_file.is_file() {
                    return Err(InstallBinaryRockError::RockManifestNotFound);
                }
                let rock_manifest_content = tokio::fs::read_to_string(rock_manifest_file).await?;
                let output_paths = match self.entry_type {
                    tree::EntryType::Entrypoint => self.tree.entrypoint(&package)?,
                    tree::EntryType::DependencyOnly => self.tree.dependency(&package)?,
                };
                let rock_manifest = RockManifest::new(&rock_manifest_content)?;
                install_manifest_entries(
                    &rock_manifest.lib.entries,
                    &unpack_dir.path().join("lib"),
                    &output_paths.lib,
                )
                .await?;
                install_manifest_entries(
                    &rock_manifest.lua.entries,
                    &unpack_dir.path().join("lua"),
                    &output_paths.src,
                )
                .await?;
                install_manifest_entries(
                    &rock_manifest.bin.entries,
                    &unpack_dir.path().join("bin"),
                    &output_paths.bin,
                )
                .await?;
                install_manifest_entries(
                    &rock_manifest.doc.entries,
                    &unpack_dir.path().join("doc"),
                    &output_paths.doc,
                )
                .await?;
                install_manifest_entries(
                    &rock_manifest.root.entries,
                    unpack_dir.path(),
                    &output_paths.etc,
                )
                .await?;
                // rename <name>-<version>.rockspec
                let rockspec_path = output_paths.etc.join(format!(
                    "{}-{}.rockspec",
                    package.name(),
                    package.version()
                ));
                if rockspec_path.is_file() {
                    tokio::fs::copy(&rockspec_path, output_paths.rockspec_path()).await?;
                    tokio::fs::remove_file(&rockspec_path).await?;
                }
                Ok(package)
            }
        }
    }
}

async fn install_manifest_entries<T>(
    entry: &HashMap<PathBuf, T>,
    src: &Path,
    dest: &Path,
) -> Result<(), InstallBinaryRockError> {
    for relative_src_path in entry.keys() {
        let target = dest.join(relative_src_path);
        let src_path = src.join(relative_src_path);
        if src_path.is_dir() {
            recursive_copy_dir(&src_path, &target).await?;
        } else if src_path.is_file() {
            if let Some(target_parent_dir) = target.parent() {
                tokio::fs::create_dir_all(target_parent_dir).await?;
            }
            tokio::fs::copy(src.join(relative_src_path), target).await?;
        } else {
            let metadata = tokio::fs::metadata(&src_path).await?;
            return Err(InstallBinaryRockError::NotAFileOrDirectory(
                src_path.to_string_lossy().to_string(),
                metadata,
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod test {

    use io::Read;

    use crate::{
        config::{ConfigBuilder, LuaVersion},
        operations::{unpack_rockspec, DownloadedPackedRockBytes, Pack, Uninstall},
        progress::MultiProgress,
    };

    use super::*;

    #[tokio::test]
    async fn install_binary_rock() {
        let content = std::fs::read("resources/test/sample-project-0.1.0-1.all.rock").unwrap();
        let rock_bytes = Bytes::copy_from_slice(&content);
        let packed_rock_file_name = "sample-project-0.1.0-1.all.rock".to_string();
        let cursor = Cursor::new(rock_bytes.clone());
        let mut zip = zip::ZipArchive::new(cursor).unwrap();
        let manifest_index = zip.index_for_path("rock_manifest").unwrap();
        let mut manifest_file = zip.by_index(manifest_index).unwrap();
        let mut content = String::new();
        manifest_file.read_to_string(&mut content).unwrap();
        let rock = DownloadedPackedRockBytes {
            name: "sample-project".into(),
            version: "0.1.0-1".parse().unwrap(),
            bytes: rock_bytes,
            file_name: packed_rock_file_name.clone(),
            url: "https://test.org".parse().unwrap(),
        };
        let rockspec = unpack_rockspec(&rock).await.unwrap();
        let install_root = assert_fs::TempDir::new().unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .user_tree(Some(install_root.to_path_buf()))
            .build()
            .unwrap();
        let progress = MultiProgress::new(&config);
        let bar = progress.map(MultiProgress::new_bar);
        let tree = config
            .user_tree(LuaVersion::from_config(&config).unwrap())
            .unwrap();
        let local_package = BinaryRockInstall::new(
            &rockspec,
            RemotePackageSource::Test,
            rock.bytes,
            tree::EntryType::Entrypoint,
            &config,
            &tree,
            &bar,
        )
        .install()
        .await
        .unwrap();
        let rock_layout = tree.entrypoint_layout(&local_package);
        let foo_bar_module = rock_layout.src.join("foo").join("bar.lua");
        assert!(foo_bar_module.is_file());
    }

    /// This relatively large integration test case tests the following:
    ///
    /// - Install a packed rock that was packed using luarocks 3.11 from the test resources.
    /// - Pack the rock using our own `Pack` implementation.
    /// - Verify that the `rock_manifest` entry of the original packed rock and our own packed rock
    ///   are equal (this means luarocks should be able to install our packed rock).
    /// - Uninstall the local package.
    /// - Install the package from our packed rock.
    /// - Verify that the contents of the install directories when installing from both packed rocks
    ///   are the same.
    #[tokio::test]
    async fn install_binary_rock_roundtrip() {
        if std::env::var("LUX_SKIP_IMPURE_TESTS").unwrap_or("0".into()) == "1" {
            println!("Skipping impure test");
            return;
        }
        let content = std::fs::read("resources/test/toml-edit-0.6.0-1.linux-x86_64.rock").unwrap();
        let rock_bytes = Bytes::copy_from_slice(&content);
        let packed_rock_file_name = "toml-edit-0.6.0-1.linux-x86_64.rock".to_string();
        let cursor = Cursor::new(rock_bytes.clone());
        let mut zip = zip::ZipArchive::new(cursor).unwrap();
        let manifest_index = zip.index_for_path("rock_manifest").unwrap();
        let mut manifest_file = zip.by_index(manifest_index).unwrap();
        let mut content = String::new();
        manifest_file.read_to_string(&mut content).unwrap();
        let orig_manifest = RockManifest::new(&content).unwrap();
        let rock = DownloadedPackedRockBytes {
            name: "toml-edit".into(),
            version: "0.6.0-1".parse().unwrap(),
            bytes: rock_bytes,
            file_name: packed_rock_file_name.clone(),
            url: "https://test.org".parse().unwrap(),
        };
        let rockspec = unpack_rockspec(&rock).await.unwrap();
        let install_root = assert_fs::TempDir::new().unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .user_tree(Some(install_root.to_path_buf()))
            .build()
            .unwrap();
        let progress = MultiProgress::new(&config);
        let bar = progress.map(MultiProgress::new_bar);
        let tree = config
            .user_tree(LuaVersion::from_config(&config).unwrap())
            .unwrap();
        let local_package = BinaryRockInstall::new(
            &rockspec,
            RemotePackageSource::Test,
            rock.bytes,
            tree::EntryType::Entrypoint,
            &config,
            &tree,
            &bar,
        )
        .install()
        .await
        .unwrap();
        let rock_layout = tree.entrypoint_layout(&local_package);

        assert!(rock_layout.lib.join("toml_edit.so").is_file());

        let orig_install_tree_integrity = rock_layout.rock_path.hash().unwrap();

        let pack_dest_dir = assert_fs::TempDir::new().unwrap();
        let packed_rock = Pack::new(
            pack_dest_dir.to_path_buf(),
            tree.clone(),
            local_package.clone(),
        )
        .pack()
        .await
        .unwrap();
        assert_eq!(
            packed_rock
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            packed_rock_file_name.clone()
        );

        // let's make sure our own pack/unpack implementation roundtrips correctly
        Uninstall::new()
            .config(&config)
            .package(local_package.id())
            .remove()
            .await
            .unwrap();
        let content = std::fs::read(&packed_rock).unwrap();
        let rock_bytes = Bytes::copy_from_slice(&content);
        let cursor = Cursor::new(rock_bytes.clone());
        let mut zip = zip::ZipArchive::new(cursor).unwrap();
        let manifest_index = zip.index_for_path("rock_manifest").unwrap();
        let mut manifest_file = zip.by_index(manifest_index).unwrap();
        let mut content = String::new();
        manifest_file.read_to_string(&mut content).unwrap();
        let packed_manifest = RockManifest::new(&content).unwrap();
        assert_eq!(packed_manifest, orig_manifest);
        let rock = DownloadedPackedRockBytes {
            name: "toml-edit".into(),
            version: "0.6.0-1".parse().unwrap(),
            bytes: rock_bytes,
            file_name: packed_rock_file_name.clone(),
            url: "https://test.org".parse().unwrap(),
        };
        let rockspec = unpack_rockspec(&rock).await.unwrap();
        let local_package = BinaryRockInstall::new(
            &rockspec,
            RemotePackageSource::Test,
            rock.bytes,
            tree::EntryType::Entrypoint,
            &config,
            &tree,
            &bar,
        )
        .install()
        .await
        .unwrap();
        let rock_layout = tree.entrypoint_layout(&local_package);
        assert!(rock_layout.rockspec_path().is_file());
        let new_install_tree_integrity = rock_layout.rock_path.hash().unwrap();
        assert_eq!(orig_install_tree_integrity, new_install_tree_integrity);
    }
}
