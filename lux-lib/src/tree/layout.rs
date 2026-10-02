use crate::{
    fs::{self, FsError},
    lockfile::{LocalPackage, OptState},
    tree::{InstallTree, Tree},
};
use std::path::{Path, PathBuf};

/// A custom layout for installed packages, applied in addition to the standard tree layout.
pub trait CustomRockLayout: std::fmt::Debug + Send + Sync {
    /// Arrange the package's files where external tools expect them.
    fn apply_layout(&self, tree: &Tree, package: &LocalPackage) -> fs::Result<()>;

    /// Remove the files arranged by [`Self::apply_layout`].
    fn remove_layout(&self, tree: &Tree, package: &LocalPackage) -> fs::Result<()>;

    /// Whether the layout applies to dependency packages, not just entrypoints.
    fn layout_dependencies(&self) -> bool {
        false
    }
}

/// A [`CustomRockLayout`] for Neovim plugins. Packages are exposed under `<tree>/site/pack/lux/{start,opt}/<package>`
#[derive(Clone, Debug, Default)]
pub struct NvimLayout;

impl NvimLayout {
    fn target_for(tree: &Tree, package: &LocalPackage) -> PathBuf {
        let subdir = match package.spec.opt {
            OptState::Required => "start",
            OptState::Optional => "opt",
        };
        tree.root()
            .join("site/pack/lux")
            .join(subdir)
            .join(package.name().to_string())
    }
}

impl CustomRockLayout for NvimLayout {
    fn apply_layout(&self, tree: &Tree, package: &LocalPackage) -> fs::Result<()> {
        let custom_dir = Self::target_for(tree, package);
        fs::sync::create_dir_all(&custom_dir)?;

        let layout = tree.layout_for(package);
        try_create_symlink(&layout.src, &custom_dir.join("lua"))?;
        try_create_symlink(&layout.lib, &custom_dir.join("lib"))?;

        if layout.etc.is_dir() {
            for entry in fs::sync::read_dir(&layout.etc)? {
                let entry = entry?;
                try_create_symlink(&entry.path(), &custom_dir.join(entry.file_name()))?;
            }
        }

        Ok(())
    }

    fn remove_layout(&self, tree: &Tree, package: &LocalPackage) -> fs::Result<()> {
        let target = Self::target_for(tree, package);
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
        let root = tree.root();
        root.parent()
            .and_then(Path::parent)
            .unwrap_or_else(|| unreachable!("tree root is two levels below the workspace root"))
            .join("Packages")
    }

    fn index_dir(tree: &Tree, package: &LocalPackage) -> PathBuf {
        Self::packages_dir(tree).join(format!(
            "_Index/{}@{}",
            package.name(),
            package.version().to_version_string()
        ))
    }

    fn content_dir(tree: &Tree, package: &LocalPackage) -> PathBuf {
        Self::index_dir(tree, package).join(package.name().to_string())
    }

    fn stub_path(tree: &Tree, package: &LocalPackage) -> PathBuf {
        Self::packages_dir(tree).join(format!("{}.lua", package.name()))
    }
}

impl CustomRockLayout for RojoLayout {
    fn apply_layout(&self, tree: &Tree, package: &LocalPackage) -> fs::Result<()> {
        if !tree.version().is_luau() {
            return Ok(());
        }

        // NOTE: Rojo does not follow symlinks (https://github.com/rojo-rbx/rojo/issues/392).
        let layout = tree.layout_for(package);
        let content_dir = Self::content_dir(tree, package);
        if content_dir.is_dir() {
            fs::sync::remove_dir_all(&content_dir)?;
        }
        copy_dir_recursive(&layout.src, &content_dir)?;

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
                    fs::sync::copy(&entrypoint, content_dir.join(format!("init.{ext}")))?;
                }
                None => {
                    return Err(FsError::Other(format!(
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

    fn remove_layout(&self, tree: &Tree, package: &LocalPackage) -> fs::Result<()> {
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

fn copy_dir_recursive(src: &Path, dest: &Path) -> fs::Result<()> {
    fs::sync::create_dir_all(dest)?;
    for entry in fs::sync::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            fs::sync::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn try_create_symlink(target: &Path, link: &Path) -> fs::Result<()> {
    if link.symlink_metadata().is_ok() {
        return Ok(());
    }

    let relative = pathdiff::diff_paths(
        target,
        link.parent()
            .ok_or_else(|| unreachable!("missing parent directory"))?,
    )
    .ok_or_else(|| {
        FsError::Other(format!(
            "failed to compute relative path for symlink from {} to {}",
            link.display(),
            target.display()
        ))
    })?;

    #[cfg(unix)]
    fs::unix::symlink(relative, link)?;

    #[cfg(windows)]
    fs::windows::symlink_dir(relative, link)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use assert_fs::prelude::PathCopy;
    use std::path::PathBuf;

    use crate::{
        config::ConfigBuilder,
        lockfile::{LocalPackage, LocalPackageHashes, LockConstraint},
        lua_version::LuaVersion,
        package::PackageSpec,
        remote_package_source::RemotePackageSource,
        rockspec::RockBinaries,
        tree::{EntryType, InstallTree, NvimLayout, RojoLayout},
    };

    fn mock_hashes() -> LocalPackageHashes {
        LocalPackageHashes {
            rockspec: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                .parse()
                .unwrap(),
            source: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                .parse()
                .unwrap(),
        }
    }

    fn sample_tree() -> (assert_fs::TempDir, PathBuf, crate::tree::Tree) {
        let tree_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/sample-tree");
        let temp = assert_fs::TempDir::new().unwrap();
        temp.copy_from(&tree_path, &["**"]).unwrap();
        let tree_path = temp.to_path_buf();
        let config = ConfigBuilder::new()
            .unwrap()
            .user_tree(Some(tree_path.clone()))
            .entrypoint_layout(NvimLayout)
            .build()
            .unwrap();
        let tree = config.user_tree(LuaVersion::Lua51).unwrap();
        (temp, tree_path, tree)
    }

    fn sample_package() -> LocalPackage {
        LocalPackage::from(
            &PackageSpec::parse("neorg".into(), "8.0.0-1".into()).unwrap(),
            LockConstraint::Unconstrained,
            RockBinaries::default(),
            RemotePackageSource::Test,
            None,
            mock_hashes(),
        )
    }

    #[test]
    fn nvim_layout_creates_symlinks_for_entrypoints() {
        let (_temp, tree_path, tree) = sample_tree();
        let package = sample_package();

        tree.prepare(&package).unwrap();
        tree.finalize(&package, EntryType::Entrypoint).unwrap();

        let custom_dir = tree_path
            .join("5.1/site/pack/lux/start")
            .join(package.name().to_string());
        assert!(custom_dir.join("lua").symlink_metadata().is_ok());
        assert!(custom_dir.join("lib").symlink_metadata().is_ok());
        assert!(custom_dir.join("conf").symlink_metadata().is_err());
        assert!(custom_dir.join("doc").symlink_metadata().is_err());
    }

    #[test]
    fn nvim_layout_links_etc_entries_after_finalize() {
        let (_temp, tree_path, tree) = sample_tree();
        let package = sample_package();

        tree.prepare(&package).unwrap();

        let etc = tree.layout_for(&package).etc;
        std::fs::create_dir_all(etc.join("plugin")).unwrap();
        std::fs::write(etc.join("plugin/foo.vim"), "lua _G.foo = 1\n").unwrap();

        tree.finalize(&package, EntryType::Entrypoint).unwrap();

        let custom_dir = tree_path
            .join("5.1/site/pack/lux/start")
            .join(package.name().to_string());
        assert!(custom_dir.join("plugin").symlink_metadata().is_ok());
    }

    #[test]
    fn nvim_layout_skips_dependencies() {
        let (_temp, tree_path, tree) = sample_tree();
        let package = sample_package();

        tree.prepare(&package).unwrap();
        tree.finalize(&package, EntryType::DependencyOnly).unwrap();

        let custom_dir = tree_path
            .join("5.1/site/pack/lux/start")
            .join(package.name().to_string());
        assert!(!custom_dir.exists());
    }

    #[test]
    fn nvim_layout_removes_symlinks() {
        let (_temp, tree_path, tree) = sample_tree();
        let package = sample_package();

        tree.prepare(&package).unwrap();
        tree.finalize(&package, EntryType::Entrypoint).unwrap();
        let custom_dir = tree_path
            .join("5.1/site/pack/lux/start")
            .join(package.name().to_string());
        assert!(custom_dir.exists());

        tree.cleanup(&package, EntryType::Entrypoint).unwrap();
        assert!(!custom_dir.exists());
    }

    fn luau_tree() -> (assert_fs::TempDir, crate::tree::Tree) {
        let temp = assert_fs::TempDir::new().unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .user_tree(Some(temp.path().join(".lux")))
            .entrypoint_layout(RojoLayout)
            .build()
            .unwrap();
        let tree = config.user_tree(LuaVersion::Luau).unwrap();
        (temp, tree)
    }

    #[test]
    fn rojo_layout_renames_entrypoint_and_writes_stub() {
        let (temp, tree) = luau_tree();
        let package = sample_package();

        tree.prepare(&package).unwrap();
        let src = tree.layout_for(&package).src;
        std::fs::write(src.join("neorg.luau"), "return {}\n").unwrap();

        tree.finalize(&package, EntryType::Entrypoint).unwrap();

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
        let (temp, tree) = luau_tree();
        let package = sample_package();

        tree.prepare(&package).unwrap();
        let src = tree.layout_for(&package).src;
        std::fs::write(src.join("neorg.lua"), "return {}\n").unwrap();

        tree.finalize(&package, EntryType::Entrypoint).unwrap();

        let content = temp.path().join("Packages/_Index/neorg@8.0.0/neorg");
        assert!(content.join("init.lua").is_file());
        assert!(content.join("neorg.lua").is_file());
    }

    #[test]
    fn rojo_layout_applies_to_dependencies() {
        let (temp, tree) = luau_tree();
        let package = sample_package();

        tree.prepare(&package).unwrap();
        let src = tree.layout_for(&package).src;
        std::fs::write(src.join("init.luau"), "return {}\n").unwrap();

        tree.finalize(&package, EntryType::DependencyOnly).unwrap();

        assert!(temp
            .path()
            .join("Packages/_Index/neorg@8.0.0/neorg/init.luau")
            .is_file());
    }

    #[test]
    fn rojo_layout_skips_non_luau() {
        let temp = assert_fs::TempDir::new().unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .user_tree(Some(temp.path().join(".lux")))
            .entrypoint_layout(RojoLayout)
            .build()
            .unwrap();
        let tree = config.user_tree(LuaVersion::Lua51).unwrap();
        let package = sample_package();

        tree.prepare(&package).unwrap();
        tree.finalize(&package, EntryType::Entrypoint).unwrap();

        assert!(!temp.path().join("Packages").exists());
    }

    #[test]
    fn rojo_layout_removes_artifacts() {
        let (temp, tree) = luau_tree();
        let package = sample_package();

        tree.prepare(&package).unwrap();
        let src = tree.layout_for(&package).src;
        std::fs::write(src.join("init.luau"), "return {}\n").unwrap();
        tree.finalize(&package, EntryType::Entrypoint).unwrap();

        let packages = temp.path().join("Packages");
        assert!(packages.join("neorg.lua").is_file());

        tree.cleanup(&package, EntryType::Entrypoint).unwrap();
        assert!(!packages.join("neorg.lua").exists());
        assert!(!packages.join("_Index/neorg@8.0.0").exists());
    }
}
