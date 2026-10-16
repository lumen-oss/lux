use crate::lockfile::{PackageLock, ReadWrite, WorkspaceLockfile};

use super::{materialise::MaterialiseArtifacts, Artifacts};

/// The lockfile entries resolved by the pipeline, grouped by section.
/// Allows for transactionality when committing to the lockfile.
#[derive(Default)]
pub struct LockfileHandle(Artifacts<PackageLock>);

impl LockfileHandle {
    pub(crate) fn from_artifacts(artifacts: &MaterialiseArtifacts) -> Self {
        Self(
            artifacts
                .iter()
                .map(|(section, packages)| {
                    (
                        section,
                        packages.map(|packages| {
                            PackageLock::from_packages(
                                packages
                                    .values()
                                    .map(|pkg| (pkg.package.clone(), pkg.entry_type)),
                            )
                        }),
                    )
                })
                .collect(),
        )
    }

    pub(crate) fn commit(&self, lockfile: &mut WorkspaceLockfile<ReadWrite>) {
        self.0.iter().for_each(|(section, lock)| {
            if let Some(lock) = lock {
                lockfile.merge(lock, &section);
            }
        });
    }
}
