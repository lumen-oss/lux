use crate::lockfile::LockedPackageLockType;

pub(crate) mod build;
pub(crate) mod build_project;
pub(crate) mod discover;
pub(crate) mod download_sources_and_hash;
pub(crate) mod emit_lockfile;
pub mod install_packages;
pub mod install_workspace;
pub(crate) mod resolve;

/// Packages grouped by the lockfile section they belong to.
#[derive(Default)]
pub(crate) struct Artifacts<T> {
    pub(crate) regular: T,
    pub(crate) build: T,
    pub(crate) test: T,
}

impl<T> Artifacts<T> {
    pub(crate) fn get(&self, section: LockedPackageLockType) -> &T {
        match section {
            LockedPackageLockType::Regular => &self.regular,
            LockedPackageLockType::Build => &self.build,
            LockedPackageLockType::Test => &self.test,
        }
    }

    pub(crate) fn get_mut(&mut self, section: LockedPackageLockType) -> &mut T {
        match section {
            LockedPackageLockType::Regular => &mut self.regular,
            LockedPackageLockType::Build => &mut self.build,
            LockedPackageLockType::Test => &mut self.test,
        }
    }
}

impl<T: Default> Extend<(LockedPackageLockType, T)> for Artifacts<T> {
    fn extend<I: IntoIterator<Item = (LockedPackageLockType, T)>>(&mut self, iter: I) {
        iter.into_iter()
            .for_each(|(section, value)| *self.get_mut(section) = value);
    }
}

impl<T> IntoIterator for Artifacts<T> {
    type Item = (LockedPackageLockType, T);
    type IntoIter = std::array::IntoIter<Self::Item, 3>;

    fn into_iter(self) -> Self::IntoIter {
        [
            (LockedPackageLockType::Regular, self.regular),
            (LockedPackageLockType::Build, self.build),
            (LockedPackageLockType::Test, self.test),
        ]
        .into_iter()
    }
}
