use std::{collections::HashMap, path::PathBuf};

use crate::{
    config::Config,
    lockfile::{LockedPackage, LockedPackageSpec, Lockfile, ReadOnly},
    lua, package,
    tree::{
        EntryType, Error, InstallTree, Installable, RockLayout, RockMatches, Stageable,
        StagingTree, Tree,
    },
};

#[derive(Debug)]
pub struct Transaction<'a, T: Stageable> {
    tree: &'a T,
    staging: StagingTree,
    committed: bool,
}

impl<'a, T: Stageable> Transaction<'a, T> {
    pub fn new(tree: &'a T) -> Result<Self, Error> {
        Ok(Self {
            tree,
            staging: tree.stage()?,
            committed: false,
        })
    }

    pub fn staging(&self) -> &StagingTree {
        &self.staging
    }

    pub fn commit(mut self) -> Result<(), Error> {
        self.tree.commit_stage(&self.staging)?;
        self.committed = true;
        Ok(())
    }
}

impl<T: Stageable> Drop for Transaction<'_, T> {
    fn drop(&mut self) {
        if !self.committed {
            self.tree.abort_stage(&self.staging);
        }
    }
}

impl<T: Stageable> InstallTree for Transaction<'_, T> {
    fn version(&self) -> &lua::Version {
        self.staging.version()
    }

    fn root(&self) -> PathBuf {
        self.staging.root()
    }

    fn bin(&self) -> PathBuf {
        self.staging.bin()
    }

    fn unwrapped_bin(&self) -> PathBuf {
        self.staging.unwrapped_bin()
    }

    fn layout_for(&self, package: &LockedPackageSpec) -> RockLayout {
        self.staging.layout_for(package)
    }

    fn lockfile(&self) -> Result<Lockfile<ReadOnly>, Error> {
        self.staging.lockfile()
    }

    fn lockfile_path(&self) -> PathBuf {
        self.staging.lockfile_path()
    }

    fn build_tree(&self, config: &Config) -> Result<Tree, Error> {
        self.staging.build_tree(config)
    }

    fn test_tree(&self, config: &Config) -> Result<Tree, Error> {
        self.staging.test_tree(config)
    }

    fn list(&self) -> Result<HashMap<package::Name, Vec<LockedPackage>>, Error> {
        self.staging.list()
    }

    fn match_rocks(&self, req: &package::Req) -> Result<RockMatches, Error> {
        self.staging.match_rocks(req)
    }

    fn prepare(&self, package: &LockedPackage) -> Result<(), Error> {
        self.staging.prepare(package)
    }

    fn finalize(&self, package: &LockedPackage, entry_type: EntryType) -> Result<(), Error> {
        self.staging.finalize(package, entry_type)
    }

    fn cleanup(&self, package: &LockedPackage, entry_type: EntryType) -> Result<(), Error> {
        self.staging.cleanup(package, entry_type)
    }
}

impl<T: Stageable> Installable for Transaction<'_, T> {}
