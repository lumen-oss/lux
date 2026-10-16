use crate::{
    lockfile::{
        LockedPackage, LockedPackageLockType, Lockfile, ReadOnly, ReadWrite, WorkspaceLockfile,
    },
    tree,
};

/// The result of reconciling a workspace's lockfile with an install tree's lockfile.
#[derive(Debug)]
pub struct SyncReport {
    pub(crate) added: Vec<LockedPackage>,
    pub(crate) removed: Vec<LockedPackage>,
}

impl SyncReport {
    pub fn added(&self) -> &[LockedPackage] {
        &self.added
    }
    pub fn removed(&self) -> &[LockedPackage] {
        &self.removed
    }

    pub(crate) fn merge(&mut self, other: SyncReport) {
        self.added.extend(other.added);
        self.removed.extend(other.removed);
    }
}

/// Determines which packages are present in one lockfile but not the other, producing the
/// packages to add (with their entry type) and the report of packages that must be removed.
pub(crate) fn reconcile_locks(
    workspace_lockfile: &WorkspaceLockfile<ReadWrite>,
    dest_lockfile: &Lockfile<ReadOnly>,
    lock_type: &LockedPackageLockType,
) -> (Vec<(tree::EntryType, LockedPackage)>, SyncReport) {
    let mut to_add: Vec<(tree::EntryType, LockedPackage)> = Vec::new();
    let mut report = SyncReport {
        added: Vec::new(),
        removed: Vec::new(),
    };

    for (id, local_package) in workspace_lockfile.rocks(lock_type) {
        if dest_lockfile.get(id).is_none() {
            let entry_type = if workspace_lockfile.is_entrypoint(&local_package.id(), lock_type) {
                tree::EntryType::Entrypoint
            } else {
                tree::EntryType::DependencyOnly
            };
            to_add.push((entry_type, local_package.clone()));
        }
    }
    for (id, local_package) in dest_lockfile.rocks() {
        if workspace_lockfile.get(id, lock_type).is_none() {
            report.removed.push(local_package.clone());
        }
    }

    (to_add, report)
}
