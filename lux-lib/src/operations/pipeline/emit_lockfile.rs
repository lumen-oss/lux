use std::collections::HashMap;

use crate::lockfile::{LockedPackageId, LockedPackageLock, ReadWrite, WorkspaceLockfile};

use super::{
    Artifacts,
    download_sources_and_hash::{DownloadSourcesAndHashArtifacts, DownloadedPackage},
};

/// The lockfile entries resolved by the pipeline, grouped by section.
pub(crate) struct LockfileHandle(Artifacts<LockedPackageLock>);

impl LockfileHandle {
    pub(crate) fn from_artifacts(artifacts: &DownloadSourcesAndHashArtifacts) -> Self {
        Self(
            artifacts
                .iter()
                .map(|(section, packages)| (section, packages.map(lock_from)))
                .collect(),
        )
    }

    pub(crate) fn commit(&self, lockfile: &mut WorkspaceLockfile<ReadWrite>) {
        self.0.iter().for_each(|(section, lock)| {
            if let Some(lock) = lock {
                lockfile.sync(lock, &section);
            }
        });
    }
}

fn lock_from(packages: &HashMap<LockedPackageId, DownloadedPackage>) -> LockedPackageLock {
    // FIXME(vhyrro): Create a constructor here instead of mut overrides.
    let mut lock = LockedPackageLock::default();
    for package in packages.values() {
        lock.insert(package.package.clone(), package.entry_type.is_entrypoint());
    }
    lock
}
