use std::{io, path::PathBuf};

use super::{InstallSession, InstallView, RockLayout, Slot, Tree};
use crate::{
    config::Config,
    fs,
    lockfile::{LockedPackage, LockedPackageSpec},
    lua, package,
    transaction::Transaction,
};
use miette::{Diagnostic, Result};
use thiserror::Error;

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
#[error(
    r#"cannot install conflicting packages in flat tree:
package: {name}
version A: {version_a}
version B: {version_b}
"#
)]
struct ConflictingPackageError {
    name: package::Name,
    version_a: package::Version,
    version_b: package::Version,
}

/// A staging tree with a flat hierarchy for distribution outside of Lux.
/// When dropped, tries to remove all build artifacts that are not meant to be distributed,
/// as well as the `etc` directory (failing silently if any artifacts cannot be removed).
/// Unlike a regular Lux tree, **conflicting packages are not supported.**
#[derive(Debug)]
pub struct FlatDistTree {
    tree: Tree,
}

impl FlatDistTree {
    pub fn new(
        root: PathBuf,
        version: lua::Version,
        config: &Config,
    ) -> Result<Self, super::Error> {
        let tree = Tree::new_flat(root, version, config)?;
        Ok(Self { tree })
    }

    fn guard_no_conflicting_package(&self, package: &LockedPackageSpec) -> Result<(), io::Error> {
        let lockfile = self.tree.regular().lockfile().map_err(io::Error::other)?;
        match lockfile.has_rock(&package.clone().into_package_req(), None) {
            Some(existing_package) => {
                if existing_package.version() == package.version() {
                    Ok(())
                } else {
                    Err(io::Error::other(ConflictingPackageError {
                        name: package.name().clone(),
                        version_a: existing_package.version().clone(),
                        version_b: package.version().clone(),
                    }))
                }
            }
            None => Ok(()),
        }
    }
}

impl Drop for FlatDistTree {
    fn drop(&mut self) {
        let build_tree_dir = self.tree.build_base();
        if build_tree_dir.is_dir() {
            let _ = fs::sync::remove_dir_all(build_tree_dir);
        }
        let root = self.tree.regular().root();
        let package_rockspec = root.join("package.rockspec");
        if package_rockspec.is_file() {
            let _ = fs::sync::remove_file(&package_rockspec);
        }
        let lockfile = self.tree.regular().lockfile_path();
        if lockfile.is_file() {
            let _ = fs::sync::remove_file(&lockfile);
        }
        let etc_dir = root.join("etc");
        if etc_dir.is_dir() {
            let _ = fs::sync::remove_dir_all(etc_dir);
        }
    }
}

impl InstallView for FlatDistTree {
    fn slot(&self) -> &Slot {
        self.tree.regular()
    }

    fn build_slot(&self) -> Result<Slot, super::Error> {
        self.tree.build()
    }

    fn build_transaction(&self) -> Result<Transaction<'_>, super::Error> {
        let slot = self.tree.build()?;
        Transaction::new(&self.tree, &slot)
    }

    fn layout_for(&self, _package: &LockedPackageSpec) -> RockLayout {
        RockLayout::new(self.tree.regular().root())
    }
}

impl InstallSession for FlatDistTree {
    fn prepare(&self, package: &LockedPackage) -> Result<(), super::Error> {
        self.guard_no_conflicting_package(&package.spec)
            .map_err(super::Error::Io)?;
        let layout = self.layout_for(&package.spec);
        fs::sync::create_dir_all(&layout.root)?;
        fs::sync::create_dir_all(&layout.lib)?;
        fs::sync::create_dir_all(&layout.src)?;
        Ok(())
    }
}
