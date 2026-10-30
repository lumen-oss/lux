use std::io::{self, Cursor};

use crate::remote;
use crate::{
    build::{
        external_dependency::{self, ExternalDependencyInfo},
        BuildBehaviour,
    },
    config::Config,
    hash::HasIntegrity,
    lockfile::{self, LockedPackage, LockedPackageHashes, LockedPackageSpec},
    rockspec,
    rockspec::Rockspec,
    tree::{self, InstallTree},
};
use crate::{fs, lockfile::RemotePackageSourceUrl, rockspec::LuaVersionCompatibility};
use bytes::Bytes;
use miette::Diagnostic;
use thiserror::Error;

use super::{install_rock_manifest, InstallRockManifestError, RockManifestSources};

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("IO operation failed")]
    Io(#[from] io::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Lockfile(#[from] lockfile::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] tree::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    ExternalSpec(#[from] external_dependency::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaVersion(#[from] rockspec::Error),
    #[error("failed to unpack packed rock")]
    Zip(#[from] zip::result::ZipError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    InstallRockManifest(#[from] InstallRockManifestError),
}

pub(crate) struct BinaryRockInstall<'a, T>
where
    T: InstallTree,
{
    rockspec: &'a rockspec::lua::Remote,
    spec: LockedPackageSpec,
    rock_bytes: Bytes,
    source: remote::package::Source,
    entry_type: tree::EntryType,
    behaviour: BuildBehaviour,
    config: &'a Config,
    tree: &'a T,
}

impl<'a, T> BinaryRockInstall<'a, T>
where
    T: InstallTree,
{
    pub(crate) fn new(
        rockspec: &'a rockspec::lua::Remote,
        spec: LockedPackageSpec,
        rock_bytes: Bytes,
        source: remote::package::Source,
        entry_type: tree::EntryType,
        config: &'a Config,
        tree: &'a T,
    ) -> Self {
        Self {
            rockspec,
            spec,
            rock_bytes,
            source,
            config,
            tree,
            entry_type,
            behaviour: BuildBehaviour::Ignore,
        }
    }

    pub(crate) fn behaviour(self, behaviour: BuildBehaviour) -> Self {
        Self { behaviour, ..self }
    }

    #[tracing::instrument(name = "Installing binary rock", skip_all)]
    pub(crate) async fn install(self) -> Result<LockedPackage, Error> {
        let rockspec = self.rockspec;
        for (name, dep) in rockspec.external_dependencies().current_platform() {
            let _ = ExternalDependencyInfo::probe(name, dep, self.config.external_deps())?;
        }

        rockspec.validate_lua_version_from_config(self.config)?;

        let hashes = LockedPackageHashes {
            rockspec: rockspec.hash().await?,
            source: self.rock_bytes.hash().await?,
        };
        let source_url = match &self.source {
            remote::package::Source::LuarocksBinaryRock(url) => {
                Some(RemotePackageSourceUrl::Url { url: url.clone() })
            }
            _ => None,
        };
        let package = LockedPackage::new(self.spec, self.source, source_url, hashes);
        match self.tree.lockfile()?.get(&package.id()) {
            Some(package)
                if matches!(
                    self.behaviour,
                    BuildBehaviour::Ignore | BuildBehaviour::Conflict
                ) =>
            {
                Ok(package.clone())
            }
            _ => {
                let unpack_dir = fs::tempfile::tempdir()?;
                let cursor = Cursor::new(self.rock_bytes);
                let mut zip = zip::ZipArchive::new(cursor)?;
                zip.extract(&unpack_dir)?;
                self.tree.prepare(&package.spec)?;
                let layout = self.tree.layout_for(&package.spec);
                let sources = RockManifestSources {
                    root: unpack_dir.path().to_path_buf(),
                    lib: unpack_dir.path().join("lib"),
                    lua: unpack_dir.path().join("lua"),
                    bin: unpack_dir.path().join("bin"),
                };
                install_rock_manifest(sources, &layout, &self.tree.bin()).await?;
                // rename <name>-<version>.rockspec
                let rockspec_path =
                    layout
                        .etc
                        .join(format!("{}-{}.rockspec", package.name(), package.version()));
                if rockspec_path.is_file() {
                    fs::tokio::copy(&rockspec_path, layout.rockspec_path()).await?;
                    fs::tokio::remove_file(&rockspec_path).await?;
                }
                self.tree.finalize(&package, self.entry_type)?;
                self.tree
                    .lockfile()?
                    .write_guard()
                    .set_binaries(&package, rockspec.binaries());
                Ok(package)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use assert_fs::TempDir;
    use io::{Read, Write};
    use is_executable::IsExecutable;
    use zip::write::SimpleFileOptions;

    #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "impure_tests"))]
    use crate::luarocks::rock_manifest::RockManifest;
    use crate::{
        config::ConfigBuilder,
        lockfile::{LockConstraint, LockedPackageId, OptState, PinnedState},
        lua,
        operations::{unpack_rockspec, DownloadedPackedRockBytes},
        package,
    };

    use super::*;
    #[cfg(feature = "impure_tests")]
    use flaky_test::flaky_test;

    impl rockspec::lua::Remote {
        fn to_locked_package_spec(&self) -> LockedPackageSpec {
            LockedPackageSpec::new(
                self.package(),
                self.version(),
                LockConstraint::Unconstrained,
                Vec::new(),
                Vec::new(),
                &PinnedState::default(),
                &OptState::default(),
            )
        }
    }

    #[tokio::test]
    async fn install_binary_rock() {
        let rock = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample-project-0.1.0-1.all.rock");
        let content = std::fs::read(rock).unwrap();
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
        let tree = config
            .user_tree(config.lua_version().unwrap().clone())
            .unwrap();
        let local_package = BinaryRockInstall::new(
            &rockspec,
            rockspec.to_locked_package_spec(),
            rock.bytes,
            remote::package::Source::Test,
            tree::EntryType::Entrypoint,
            &config,
            &tree,
        )
        .install()
        .await
        .unwrap();
        assert_eq!(
            local_package.hashes().source,
            "sha256-zDmnjE2b8GxX+6j5vTCWUEBxe0aFXFVhC4JOrnAgQ2g="
                .parse()
                .unwrap()
        );
        let foo_bar_module = tree
            .layout_for(&local_package.spec)
            .src
            .join("foo")
            .join("bar.lua");
        assert!(foo_bar_module.is_file());
    }

    fn packed_rock_with_bin() -> Vec<u8> {
        let rockspec = r#"
rockspec_format = "3.0"
package = "test-binary-bin"
version = "1.0-1"
source = { url = "https://example.com/test-binary-bin.zip" }
"#;
        let manifest = r#"rock_manifest = { bin = { mytool = "0" } }"#;
        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
            let options = SimpleFileOptions::default();
            for (name, content) in [
                ("test-binary-bin-1.0-1.rockspec", rockspec.as_bytes()),
                ("bin/mytool", b"#!/bin/sh\necho hi\n".as_slice()),
                ("rock_manifest", manifest.as_bytes()),
            ] {
                zip.start_file(name, options).unwrap();
                zip.write_all(content).unwrap();
            }
            zip.finish().unwrap();
        }
        buf
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn install_binary_rock_makes_bin_executable() {
        let rock = DownloadedPackedRockBytes {
            name: "test-binary-bin".into(),
            version: "1.0-1".parse().unwrap(),
            bytes: Bytes::from(packed_rock_with_bin()),
            file_name: "test-binary-bin-1.0-1.all.rock".into(),
            url: "https://test.org".parse().unwrap(),
        };
        let rockspec = unpack_rockspec(&rock).await.unwrap();
        let install_root = TempDir::new().unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .user_tree(Some(install_root.to_path_buf()))
            .build()
            .unwrap();
        let tree = config
            .user_tree(config.lua_version().unwrap().clone())
            .unwrap();
        BinaryRockInstall::new(
            &rockspec,
            rockspec.to_locked_package_spec(),
            rock.bytes,
            remote::package::Source::Test,
            tree::EntryType::Entrypoint,
            &config,
            &tree,
        )
        .install()
        .await
        .unwrap();
        assert!(tree.bin().join("mytool").is_executable());
    }

    #[tokio::test]
    async fn install_binary_rock_preserves_dependencies() {
        let rock = DownloadedPackedRockBytes {
            name: "test-binary-bin".into(),
            version: "1.0-1".parse().unwrap(),
            bytes: Bytes::from(packed_rock_with_bin()),
            file_name: "test-binary-bin-1.0-1.all.rock".into(),
            url: "https://test.org".parse().unwrap(),
        };
        let rockspec = unpack_rockspec(&rock).await.unwrap();
        let dependency = LockedPackageId::new(
            &package::Name::from("transitive-dependency"),
            &"1.0-1".parse().unwrap(),
            PinnedState::default(),
            OptState::default(),
            LockConstraint::Unconstrained,
        );
        let spec = LockedPackageSpec::new(
            rockspec.package(),
            rockspec.version(),
            LockConstraint::Unconstrained,
            vec![dependency.clone()],
            Vec::new(),
            &PinnedState::default(),
            &OptState::default(),
        );
        let install_root = TempDir::new().unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .lua_version(Some(lua::Version::Lua51))
            .user_tree(Some(install_root.to_path_buf()))
            .build()
            .unwrap();
        let tree = config.user_tree(lua::Version::Lua51).unwrap();

        let installed = BinaryRockInstall::new(
            &rockspec,
            spec,
            rock.bytes,
            remote::package::Source::Test,
            tree::EntryType::Entrypoint,
            &config,
            &tree,
        )
        .install()
        .await
        .unwrap();

        assert_eq!(installed.dependencies(), vec![&dependency]);
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
    #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "impure_tests"))]
    #[flaky_test(tokio, times = 5)]
    async fn install_binary_rock_roundtrip() {
        use crate::operations::{Pack, Uninstall};

        let rock = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/toml-edit-0.6.0-1.linux-x86_64.rock");
        let content = std::fs::read(rock).unwrap();
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
        let tree = config
            .user_tree(config.lua_version().unwrap().clone())
            .unwrap();
        let local_package = BinaryRockInstall::new(
            &rockspec,
            rockspec.to_locked_package_spec(),
            rock.bytes,
            remote::package::Source::Test,
            tree::EntryType::Entrypoint,
            &config,
            &tree,
        )
        .install()
        .await
        .unwrap();
        let layout = tree.layout_for(local_package.spec());
        assert!(layout.lib.join("toml_edit.so").is_file());

        let orig_install_tree_integrity = layout.root.hash().await.unwrap();

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
            rockspec.to_locked_package_spec(),
            rock.bytes,
            remote::package::Source::Test,
            tree::EntryType::Entrypoint,
            &config,
            &tree,
        )
        .install()
        .await
        .unwrap();
        let layout = tree.layout_for(local_package.spec());
        assert!(layout.rockspec_path().is_file());
        let new_install_tree_integrity = layout.root.hash().await.unwrap();
        assert_eq!(orig_install_tree_integrity, new_install_tree_integrity);
    }
}
