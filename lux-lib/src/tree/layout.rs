use crate::{
    fs,
    lockfile::{LockedPackageSpec, OptState},
    tree::{Slot as TreeSlot, Tree},
};
use std::path::PathBuf;

/// A custom layout for installed packages, applied in addition to the standard tree layout.
pub trait CustomRockLayout: std::fmt::Debug + Send + Sync {
    /// Arrange the package's files where external tools expect them.
    fn apply_layout(
        &self,
        tree: &Tree,
        slot: &TreeSlot,
        package: &LockedPackageSpec,
    ) -> fs::Result<()>;

    /// Remove the files arranged by [`Self::apply_layout`].
    fn remove_layout(
        &self,
        tree: &Tree,
        slot: &TreeSlot,
        package: &LockedPackageSpec,
    ) -> fs::Result<()>;

    /// Whether the layout applies to dependency packages, not just entrypoints.
    fn layout_dependencies(&self) -> bool {
        false
    }
}

/// A [`CustomRockLayout`] for Neovim plugins. Packages are exposed under `<tree>/site/pack/lux/{start,opt}/<package>`
#[derive(Clone, Debug, Default)]
pub struct NvimLayout;

impl NvimLayout {
    fn target_for(slot: &TreeSlot, package: &LockedPackageSpec) -> PathBuf {
        let subdir = match package.opt() {
            OptState::Required => "start",
            OptState::Optional => "opt",
        };
        slot.root()
            .join("site/pack/lux")
            .join(subdir)
            .join(package.name().to_string())
    }
}

impl CustomRockLayout for NvimLayout {
    fn apply_layout(
        &self,
        _tree: &Tree,
        slot: &TreeSlot,
        package: &LockedPackageSpec,
    ) -> fs::Result<()> {
        let layout = slot.layout_for(package);
        let custom_dir = Self::target_for(slot, package);
        fs::sync::create_dir_all(&custom_dir)?;

        fs::sync::link_dir(fs::sync::canonicalize(&layout.src)?, custom_dir.join("lua"))?;
        fs::sync::link_dir(fs::sync::canonicalize(&layout.lib)?, custom_dir.join("lib"))?;

        if layout.etc.is_dir() {
            for entry in fs::sync::read_dir(&layout.etc)? {
                let entry = entry?;
                let target = custom_dir.join(entry.file_name());
                let path = fs::sync::canonicalize(entry.path())?;
                if path.is_dir() {
                    fs::sync::link_dir(&path, &target)?;
                } else {
                    fs::sync::link_file(&path, &target)?;
                }
            }
        }

        Ok(())
    }

    fn remove_layout(
        &self,
        _tree: &Tree,
        slot: &TreeSlot,
        package: &LockedPackageSpec,
    ) -> fs::Result<()> {
        let target = Self::target_for(slot, package);
        if target.is_dir() {
            // SAFETY: does not follow symlinks, only removes them
            fs::sync::remove_dir_all(target)?;
        }
        Ok(())
    }
}

/// A [`CustomRockLayout`] for [Rojo](https://rojo.space/).
/// Packages are copied into a `Packages` directory at the workspace root
/// so `rojo serve` can pick them up.
#[derive(Clone, Debug, Default)]
pub struct RojoLayout;

impl RojoLayout {
    fn packages_dir(tree: &Tree) -> PathBuf {
        tree.root()
            .parent()
            .unwrap_or_else(|| unreachable!("tree root has no parent"))
            .join("Packages")
    }

    fn index_dir(tree: &Tree, package: &LockedPackageSpec) -> PathBuf {
        Self::packages_dir(tree).join(format!(
            "_Index/{}@{}",
            package.name(),
            package.version().to_version_string()
        ))
    }

    fn content_dir(tree: &Tree, package: &LockedPackageSpec) -> PathBuf {
        Self::index_dir(tree, package).join(package.name().to_string())
    }

    fn stub_path(tree: &Tree, package: &LockedPackageSpec) -> PathBuf {
        Self::packages_dir(tree).join(format!("{}.lua", package.name()))
    }
}

impl CustomRockLayout for RojoLayout {
    fn apply_layout(
        &self,
        tree: &Tree,
        slot: &TreeSlot,
        package: &LockedPackageSpec,
    ) -> fs::Result<()> {
        if !tree.version().is_luau() {
            return Ok(());
        }

        // NOTE: Rojo does not follow symlinks (https://github.com/rojo-rbx/rojo/issues/392),
        // so we hard link the package sources.
        let layout = slot.layout_for(package);
        let content_dir = Self::content_dir(tree, package);
        if content_dir.is_dir() {
            fs::sync::remove_dir_all(&content_dir)?;
        }
        fs::sync::link_dir_all(&layout.src, &content_dir)?;

        let name = package.name().to_string();
        let has_init = ["init.luau", "init.lua"]
            .iter()
            .map(|file| content_dir.join(file))
            .any(|path| path.is_file());
        if !has_init {
            let entrypoint = ["luau", "lua"].iter().find_map(|ext| {
                let path = content_dir.join(format!("{name}.{ext}"));
                path.is_file().then_some((*ext, path))
            });
            match entrypoint {
                Some((ext, entrypoint)) => {
                    fs::sync::hard_link(&entrypoint, content_dir.join(format!("init.{ext}")))?;
                }
                None => {
                    return Err(fs::Error::Other(format!(
                        "no `init.luau`, `init.lua`, `{name}.luau`, or `{name}.lua` entrypoint found in `{}`",
                        layout.src.display()
                    )));
                }
            }
        }

        let stub = Self::stub_path(tree, package);
        if let Some(parent) = stub.parent() {
            fs::sync::create_dir_all(parent)?;
        }
        fs::sync::write(
            &stub,
            format!(
                "return require(script.Parent._Index[\"{name}@{version}\"][\"{name}\"])\n",
                name = package.name(),
                version = package.version().to_version_string(),
            ),
        )?;

        Ok(())
    }

    fn remove_layout(
        &self,
        tree: &Tree,
        _slot: &TreeSlot,
        package: &LockedPackageSpec,
    ) -> fs::Result<()> {
        if !tree.version().is_luau() {
            return Ok(());
        }

        let index_dir = Self::index_dir(tree, package);
        if index_dir.is_dir() {
            fs::sync::remove_dir_all(&index_dir)?;
        }

        let stub = Self::stub_path(tree, package);
        if stub.is_file() {
            fs::sync::remove_file(&stub)?;
        }

        Ok(())
    }

    fn layout_dependencies(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::package;
    use crate::remote;
    use assert_fs::prelude::{PathChild, PathCopy};
    use std::path::PathBuf;

    use crate::{
        config,
        lockfile::{self, LockedPackage, LockedPackageHashes},
        lua,
        transaction::Transaction,
        tree::{
            EntryType, InstallSession, InstallView, NvimLayout, RojoLayout, Slot as TreeSlot, Tree,
        },
    };

    fn mock_hashes() -> LockedPackageHashes {
        LockedPackageHashes {
            rockspec: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                .parse()
                .unwrap(),
            source: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                .parse()
                .unwrap(),
        }
    }

    fn sample_tree() -> (assert_fs::TempDir, Tree, TreeSlot) {
        let temp = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(temp.to_path_buf()))
            .entrypoint_layout(NvimLayout)
            .build()
            .unwrap();
        let tree = config.user_tree(lua::Version::Lua51).unwrap();
        let slot = tree.regular().clone();
        temp.child("dependencies/5.1")
            .copy_from(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("resources/test/sample-tree/dependencies/5.1"),
                &["**"],
            )
            .unwrap();
        (temp, tree, slot)
    }

    fn sample_package() -> LockedPackage {
        LockedPackage::from(
            &package::Spec::parse("neorg".into(), "8.0.0-1".into()).unwrap(),
            lockfile::Constraint::Unconstrained,
            remote::package::Source::Test,
            None,
            mock_hashes(),
        )
    }

    #[test]
    fn nvim_layout_creates_symlinks_for_entrypoints() {
        let (_temp, tree, slot) = sample_tree();
        let package = sample_package();
        let transaction = Transaction::new(&tree, &slot).unwrap();

        transaction.prepare(&package).unwrap();
        transaction
            .finalize(&package, EntryType::Entrypoint)
            .unwrap();
        transaction.commit().unwrap();

        let custom_dir = slot
            .root()
            .join("site/pack/lux/start")
            .join(package.name().to_string());
        assert!(custom_dir.join("lua").symlink_metadata().is_ok());
        assert!(custom_dir.join("lib").symlink_metadata().is_ok());
        assert!(custom_dir.join("conf").symlink_metadata().is_err());
        assert!(custom_dir.join("doc").symlink_metadata().is_err());
    }

    #[test]
    fn nvim_layout_links_etc_entries_after_finalize() {
        let (_temp, tree, slot) = sample_tree();
        let package = sample_package();
        let transaction = Transaction::new(&tree, &slot).unwrap();

        transaction.prepare(&package).unwrap();

        let etc = transaction.layout_for(&package.spec).etc;
        std::fs::create_dir_all(etc.join("plugin")).unwrap();
        std::fs::write(etc.join("plugin/foo.vim"), "lua _G.foo = 1\n").unwrap();

        transaction
            .finalize(&package, EntryType::Entrypoint)
            .unwrap();
        transaction.commit().unwrap();

        let custom_dir = slot
            .root()
            .join("site/pack/lux/start")
            .join(package.name().to_string());
        assert!(custom_dir.join("plugin").symlink_metadata().is_ok());
    }

    #[test]
    fn nvim_layout_skips_dependencies() {
        let (_temp, tree, slot) = sample_tree();
        let package = sample_package();
        let transaction = Transaction::new(&tree, &slot).unwrap();

        transaction.prepare(&package).unwrap();
        transaction
            .finalize(&package, EntryType::DependencyOnly)
            .unwrap();
        transaction.commit().unwrap();

        let custom_dir = slot
            .root()
            .join("site/pack/lux/start")
            .join(package.name().to_string());
        assert!(!custom_dir.exists());
    }

    #[test]
    fn nvim_layout_removes_symlinks() {
        let (_temp, tree, slot) = sample_tree();
        let package = sample_package();
        let transaction = Transaction::new(&tree, &slot).unwrap();

        transaction.prepare(&package).unwrap();
        transaction
            .finalize(&package, EntryType::Entrypoint)
            .unwrap();
        transaction.commit().unwrap();
        let custom_dir = slot
            .root()
            .join("site/pack/lux/start")
            .join(package.name().to_string());
        assert!(custom_dir.exists());

        let cleanup = Transaction::new(&tree, &slot).unwrap();
        cleanup.cleanup(&package, EntryType::Entrypoint).unwrap();
        cleanup.commit().unwrap();
        assert!(!custom_dir.exists());
    }

    fn luau_tree() -> (assert_fs::TempDir, Tree, TreeSlot) {
        let temp = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(temp.path().join(".lux")))
            .entrypoint_layout(RojoLayout)
            .build()
            .unwrap();
        let tree = config.user_tree(lua::Version::Luau).unwrap();
        let slot = tree.regular().clone();
        (temp, tree, slot)
    }

    #[test]
    fn rojo_layout_renames_entrypoint_and_writes_stub() {
        let (temp, tree, slot) = luau_tree();
        let package = sample_package();
        let transaction = Transaction::new(&tree, &slot).unwrap();

        transaction.prepare(&package).unwrap();
        let src = transaction.layout_for(&package.spec).src;
        std::fs::write(src.join("neorg.luau"), "return {}\n").unwrap();

        transaction
            .finalize(&package, EntryType::Entrypoint)
            .unwrap();
        transaction.commit().unwrap();

        let packages = temp.path().join("Packages");
        let content = packages.join("_Index/neorg@8.0.0/neorg");
        assert!(content.join("init.luau").is_file());
        assert!(content.join("neorg.luau").is_file());
        assert_eq!(
            std::fs::read_to_string(packages.join("neorg.lua")).unwrap(),
            "return require(script.Parent._Index[\"neorg@8.0.0\"][\"neorg\"])\n"
        );
    }

    #[test]
    fn rojo_layout_handles_lua_entrypoint() {
        let (temp, tree, slot) = luau_tree();
        let package = sample_package();
        let transaction = Transaction::new(&tree, &slot).unwrap();

        transaction.prepare(&package).unwrap();
        let src = transaction.layout_for(&package.spec).src;
        std::fs::write(src.join("neorg.lua"), "return {}\n").unwrap();

        transaction
            .finalize(&package, EntryType::Entrypoint)
            .unwrap();
        transaction.commit().unwrap();

        let content = temp.path().join("Packages/_Index/neorg@8.0.0/neorg");
        assert!(content.join("init.lua").is_file());
        assert!(content.join("neorg.lua").is_file());
    }

    #[test]
    fn rojo_layout_applies_to_dependencies() {
        let (temp, tree, slot) = luau_tree();
        let package = sample_package();
        let transaction = Transaction::new(&tree, &slot).unwrap();

        transaction.prepare(&package).unwrap();
        let src = transaction.layout_for(&package.spec).src;
        std::fs::write(src.join("init.luau"), "return {}\n").unwrap();

        transaction
            .finalize(&package, EntryType::DependencyOnly)
            .unwrap();
        transaction.commit().unwrap();

        assert!(temp
            .path()
            .join("Packages/_Index/neorg@8.0.0/neorg/init.luau")
            .is_file());
    }

    #[test]
    fn rojo_layout_skips_non_luau() {
        let temp = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(temp.path().join(".lux")))
            .entrypoint_layout(RojoLayout)
            .build()
            .unwrap();
        let tree = config.user_tree(lua::Version::Lua51).unwrap();
        let slot = tree.regular().clone();
        let package = sample_package();
        let transaction = Transaction::new(&tree, &slot).unwrap();

        transaction.prepare(&package).unwrap();
        transaction
            .finalize(&package, EntryType::Entrypoint)
            .unwrap();
        transaction.commit().unwrap();

        assert!(!temp.path().join("Packages").exists());
    }

    #[test]
    fn rojo_layout_removes_artifacts() {
        let (temp, tree, slot) = luau_tree();
        let package = sample_package();
        let transaction = Transaction::new(&tree, &slot).unwrap();

        transaction.prepare(&package).unwrap();
        let src = transaction.layout_for(&package.spec).src;
        std::fs::write(src.join("init.luau"), "return {}\n").unwrap();
        transaction
            .finalize(&package, EntryType::Entrypoint)
            .unwrap();
        transaction.commit().unwrap();

        let packages = temp.path().join("Packages");
        assert!(packages.join("neorg.lua").is_file());

        let cleanup = Transaction::new(&tree, &slot).unwrap();
        cleanup.cleanup(&package, EntryType::Entrypoint).unwrap();
        cleanup.commit().unwrap();
        assert!(!packages.join("neorg.lua").exists());
        assert!(!packages.join("_Index/neorg@8.0.0").exists());
    }
}
