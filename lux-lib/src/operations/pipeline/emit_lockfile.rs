use std::collections::HashMap;

use bon::Builder;

use crate::lockfile::{
    LockedPackageId, LockedPackageLock, LockedPackageLockType, ReadWrite, WorkspaceLockfile,
};

use super::download_sources_and_hash::{DownloadSourcesAndHashArtifacts, DownloadedPackage};

/// Writes the resolved [`DownloadSourcesAndHashArtifacts`] to a workspace lockfile.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(crate) struct EmitLockfile<'a> {
    #[builder(start_fn)]
    pub(crate) lockfile: &'a mut WorkspaceLockfile<ReadWrite>,
    pub(crate) artifacts: &'a DownloadSourcesAndHashArtifacts,
}

impl<State> EmitLockfileBuilder<'_, State>
where
    State: emit_lockfile_builder::State + emit_lockfile_builder::IsComplete,
{
    pub(crate) fn emit(self) {
        let args = self._build();

        for section in [
            LockedPackageLockType::Regular,
            LockedPackageLockType::Build,
            LockedPackageLockType::Test,
        ] {
            args.lockfile
                .sync(&lock_from(args.artifacts.get(section)), &section);
        }
    }
}

fn lock_from(packages: &HashMap<LockedPackageId, DownloadedPackage>) -> LockedPackageLock {
    // FIXME(vhyrro): Create a constructor here instead of mut overrides.
    let mut lock = LockedPackageLock::default();
    for package in packages.values() {
        lock.insert(
            package.package.clone(),
            package.entry_type.is_entrypoint(),
        );
    }
    lock
}
