use std::{collections::HashMap, io, path::PathBuf};

use super::{InstallTree, RockLayout, Tree, TreeError};
use crate::{
    config::{tree::RockLayoutConfig, Config},
    fs,
    lockfile::{LocalPackage, Lockfile, ReadOnly},
    lua_version::LuaVersion,
    package::{PackageName, PackageVersion},
    tree::mk_rock_layout,
};
use miette::{Diagnostic, Result};
use thiserror::Error;

const SRC_DIR_NAME: &str = "lua";
const LIB_DIR_NAME: &str = "lib";

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
    name: PackageName,
    version_a: PackageVersion,
    version_b: PackageVersion,
}

/// A staging tree with a flat hierarchy for distribution outside of Lux.
/// When dropped, tries to remove all build artifacts that are not meant to be distributed,
/// as well as the `etc` directory (failing silently if any artifacts cannot be removed).
/// Unlike a regular Lux tree, **conflicting packages are not supported.**
#[derive(Clone, Debug)]
pub struct FlatDistTree(Tree);

impl FlatDistTree {
    pub fn new(root: PathBuf, version: LuaVersion, config: &Config) -> Result<Self, TreeError> {
        let version_dir = root.join(version.to_string());
        let test_tree_dir = version_dir.join("test_dependencies");
        let build_tree_dir = version_dir.join("build_dependencies");
        let tree = Tree::new_with_paths(root, test_tree_dir, build_tree_dir, version, config)?;
        Ok(Self(tree))
    }

    fn guard_no_conflicting_package(&self, package: &LocalPackage) -> Result<(), io::Error> {
        let lockfile = self.lockfile().map_err(io::Error::other)?;
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
        let build_tree_dir = &self.0.build_tree_dir;
        if build_tree_dir.is_dir() {
            let _ = fs::sync::remove_dir_all(build_tree_dir);
        }
        let package_rockspec = self.root().join("package.rockspec");
        if package_rockspec.is_file() {
            let _ = fs::sync::remove_file(&package_rockspec);
        }
        let lockfile = self.lockfile_path();
        if lockfile.is_file() {
            let _ = fs::sync::remove_file(&lockfile);
        }
        let etc_dir = self.root().join("etc");
        if etc_dir.is_dir() {
            let _ = fs::sync::remove_dir_all(etc_dir);
        }
    }
}

impl InstallTree for FlatDistTree {
    fn version(&self) -> &LuaVersion {
        self.0.version()
    }

    fn root(&self) -> PathBuf {
        self.0.root()
    }

    fn root_for(&self, _package: &LocalPackage) -> PathBuf {
        self.0.root()
    }

    fn bin(&self) -> PathBuf {
        self.0.bin()
    }

    fn unwrapped_bin(&self) -> PathBuf {
        self.0.unwrapped_bin()
    }

    fn entrypoint(&self, package: &LocalPackage) -> io::Result<RockLayout> {
        self.guard_no_conflicting_package(package)?;
        Ok(mk_rock_layout(
            SRC_DIR_NAME,
            LIB_DIR_NAME,
            self,
            package,
            &self.0.entrypoint_layout,
        ))
    }

    fn dependency(&self, package: &LocalPackage) -> io::Result<RockLayout> {
        self.guard_no_conflicting_package(package)?;
        Ok(mk_rock_layout(
            SRC_DIR_NAME,
            LIB_DIR_NAME,
            self,
            package,
            &RockLayoutConfig::default(),
        ))
    }

    fn lockfile(&self) -> Result<Lockfile<ReadOnly>, TreeError> {
        self.0.lockfile()
    }

    fn lockfile_path(&self) -> PathBuf {
        self.0.lockfile_path()
    }

    fn build_tree(&self, config: &Config) -> Result<Tree, TreeError> {
        self.0.build_tree(config)
    }

    fn test_tree(&self, config: &Config) -> Result<Tree, TreeError> {
        self.0.test_tree(config)
    }

    fn installed_rock_layout(&self, package: &LocalPackage) -> Result<RockLayout, TreeError> {
        let lockfile = self.lockfile()?;
        let layout_config = if lockfile.is_entrypoint(&package.id()) {
            self.0.entrypoint_layout.clone()
        } else {
            RockLayoutConfig::default()
        };
        Ok(mk_rock_layout(
            SRC_DIR_NAME,
            LIB_DIR_NAME,
            self,
            package,
            &layout_config,
        ))
    }

    fn list(&self) -> Result<HashMap<PackageName, Vec<LocalPackage>>, TreeError> {
        self.0.list()
    }

    fn match_rocks(
        &self,
        req: &crate::package::PackageReq,
    ) -> Result<super::RockMatches, TreeError> {
        self.0.match_rocks(req)
    }
}
