use std::{collections::HashMap, sync::Mutex};

use crate::{
    fs,
    lockfile::{LockedPackage, LockedPackageId, LockedPackageSpec},
    store::StagingEntry,
    tree::{
        EntryType, Error, InstallSession, InstallView, RockLayout, Slot, StagingTree, Tree,
        UninstallTarget,
    },
};

#[derive(Debug)]
pub struct Transaction<'a> {
    tree: &'a Tree,
    staging: StagingTree,
    pending: Mutex<HashMap<LockedPackageId, StagingEntry>>,
}

impl<'a> Transaction<'a> {
    pub fn new(tree: &'a Tree, target: &Slot) -> Result<Self, Error> {
        Ok(Self {
            tree,
            staging: StagingTree::new(target)?,
            pending: Mutex::new(HashMap::new()),
        })
    }

    pub fn commit(mut self) -> Result<(), Error> {
        self.staging.commit()?;
        Ok(())
    }

    /// Adds an entrypoint to the staging lockfile, then commits the transaction.
    pub(crate) fn commit_with_entrypoint(mut self, package: &LockedPackage) -> Result<(), Error> {
        self.staging
            .slot()
            .lockfile()?
            .write_guard()
            .add_entrypoint(package);
        self.staging.commit()?;
        Ok(())
    }

    fn pending_entry(&self, id: &LockedPackageId) -> Option<StagingEntry> {
        self.pending
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(id)
    }
}

impl InstallView for Transaction<'_> {
    fn slot(&self) -> &Slot {
        self.staging.slot()
    }

    fn build_slot(&self) -> Result<Slot, Error> {
        self.tree.build()
    }

    fn build_transaction(&self) -> Result<Transaction<'_>, Error> {
        let slot = self.tree.build()?;
        Transaction::new(self.tree, &slot)
    }

    fn layout_for(&self, package: &LockedPackageSpec) -> RockLayout {
        if let Some(entry) = self
            .pending
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&package.id())
        {
            entry.layout()
        } else {
            self.staging.slot().layout_for(package)
        }
    }
}

impl InstallSession for Transaction<'_> {
    fn prepare(&self, package: &LockedPackage) -> Result<(), Error> {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if pending.contains_key(&package.id()) {
            return Ok(());
        }

        let entry = self.tree.store().staging_entry(package)?;
        entry.prepare()?;
        pending.insert(package.id(), entry);
        Ok(())
    }

    fn reuse_complete(
        &self,
        package: &LockedPackage,
        entry_type: EntryType,
    ) -> Result<Option<LockedPackage>, Error> {
        let Some(entry) = self.tree.store().find_complete(package) else {
            return Ok(None);
        };

        self.staging
            .publish(self.tree, package, Some(&entry), entry_type)?;

        Ok(Some(package.clone()))
    }

    fn finalize(&self, package: &LockedPackage, entry_type: EntryType) -> Result<(), Error> {
        let pending = self
            .pending_entry(&package.id())
            .map(StagingEntry::promote)
            .transpose()?;
        self.staging
            .publish(self.tree, package, pending.as_ref(), entry_type)
    }

    fn cleanup(&self, package: &LockedPackage, entry_type: EntryType) -> Result<(), Error> {
        let slot = self.staging.slot();
        if let Some(layout) = self.tree.entrypoint_layout() {
            if entry_type.is_entrypoint() || layout.layout_dependencies() {
                layout.remove_layout(self.tree, slot, &package.spec)?;
            }
        }

        // Dropping the staging entry removes its staging directory.
        drop(self.pending_entry(&package.id()));

        let layout = slot.layout_for(&package.spec);
        fs::sync::remove_path(&layout.root)?;

        let lockfile = slot.lockfile()?;
        for relative_binary_path in lockfile
            .binaries(&package.id())
            .into_iter()
            .flat_map(|binaries| binaries.iter())
        {
            let binary_path = slot.bin().join(relative_binary_path);
            if binary_path.symlink_metadata().is_ok() {
                fs::sync::remove_file(binary_path)?;
            }

            let unwrapped_binary_path = slot.unwrapped_bin().join(relative_binary_path);
            if unwrapped_binary_path.symlink_metadata().is_ok() {
                fs::sync::remove_file(unwrapped_binary_path)?;
            }
        }

        Ok(())
    }
}

impl UninstallTarget for Transaction<'_> {}
