use crate::{
    fs::{self, FsError},
    lockfile::{LocalPackage, OptState},
    tree::{InstallTree, Tree},
};
use std::path::{Path, PathBuf};

/// A custom layout for entrypoint packages.
/// Implementations arrange symlinks so external tools can find files at the locations they expect.
pub trait CustomRockLayout: std::fmt::Debug + Send + Sync {
    fn make_symlinks(&self, tree: &Tree, package: &LocalPackage) -> fs::Result<()>;

    fn remove_symlinks(&self, tree: &Tree, package: &LocalPackage) -> fs::Result<()>;
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
    fn make_symlinks(&self, tree: &Tree, package: &LocalPackage) -> fs::Result<()> {
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

    fn remove_symlinks(&self, tree: &Tree, package: &LocalPackage) -> fs::Result<()> {
        // SAFETY: does not follow symlinks, only removes them
        fs::sync::remove_dir_all(Self::target_for(tree, package))
    }
}

fn try_create_symlink(target: &Path, link: &Path) -> fs::Result<()> {
    if link.symlink_metadata().is_ok() {
        return Ok(());
    }

    let relative = pathdiff::diff_paths(
        target,
        link.parent()
            .ok_or_else(|| FsError::Other("invalid parent directory".into()))?,
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
        tree::{EntryType, InstallTree, NvimLayout},
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

        tree.prepare(&package, EntryType::Entrypoint).unwrap();

        let custom_dir = tree_path
            .join("5.1/site/pack/lux/start")
            .join(package.name().to_string());
        assert!(custom_dir.join("lua").symlink_metadata().is_ok());
        assert!(custom_dir.join("lib").symlink_metadata().is_ok());
        assert!(custom_dir.join("conf").symlink_metadata().is_err());
        assert!(custom_dir.join("doc").symlink_metadata().is_err());
    }

    #[test]
    fn nvim_layout_skips_dependencies() {
        let (_temp, tree_path, tree) = sample_tree();
        let package = sample_package();

        tree.prepare(&package, EntryType::DependencyOnly).unwrap();

        let custom_dir = tree_path
            .join("5.1/site/pack/lux/start")
            .join(package.name().to_string());
        assert!(!custom_dir.exists());
    }

    #[test]
    fn nvim_layout_removes_symlinks() {
        let (_temp, tree_path, tree) = sample_tree();
        let package = sample_package();

        tree.prepare(&package, EntryType::Entrypoint).unwrap();
        let custom_dir = tree_path
            .join("5.1/site/pack/lux/start")
            .join(package.name().to_string());
        assert!(custom_dir.exists());

        tree.cleanup(&package, EntryType::Entrypoint).unwrap();
        assert!(!custom_dir.exists());
    }
}
