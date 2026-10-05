use crate::lockfile::LockedPackageLockType;

pub mod build;
pub mod build_local;
pub mod discover;
pub mod download_sources_and_hash;
pub mod emit_lockfile;
pub mod install_packages;
pub mod install_workspace;
pub mod resolve;

/// Packages grouped by the lockfile section they belong to.
/// `None` means that that section shouldn't be modified.
pub(crate) struct Artifacts<T> {
    pub(crate) regular: Option<T>,
    pub(crate) build: Option<T>,
    pub(crate) test: Option<T>,
}

impl<T> Default for Artifacts<T> {
    fn default() -> Self {
        Self {
            regular: None,
            build: None,
            test: None,
        }
    }
}

impl<T> Artifacts<T> {
    pub(crate) fn get_mut(&mut self, section: LockedPackageLockType) -> &mut Option<T> {
        match section {
            LockedPackageLockType::Regular => &mut self.regular,
            LockedPackageLockType::Build => &mut self.build,
            LockedPackageLockType::Test => &mut self.test,
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (LockedPackageLockType, Option<&T>)> {
        [
            (LockedPackageLockType::Regular, self.regular.as_ref()),
            (LockedPackageLockType::Build, self.build.as_ref()),
            (LockedPackageLockType::Test, self.test.as_ref()),
        ]
        .into_iter()
    }
}

impl<T> Extend<(LockedPackageLockType, Option<T>)> for Artifacts<T> {
    fn extend<I: IntoIterator<Item = (LockedPackageLockType, Option<T>)>>(&mut self, iter: I) {
        iter.into_iter()
            .for_each(|(section, value)| *self.get_mut(section) = value);
    }
}

impl<T> FromIterator<(LockedPackageLockType, Option<T>)> for Artifacts<T> {
    fn from_iter<I: IntoIterator<Item = (LockedPackageLockType, Option<T>)>>(iter: I) -> Self {
        let mut artifacts = Artifacts::default();
        artifacts.extend(iter);
        artifacts
    }
}

impl<T> IntoIterator for Artifacts<T> {
    type Item = (LockedPackageLockType, Option<T>);
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
