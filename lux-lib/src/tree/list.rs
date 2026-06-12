use crate::{lockfile::LocalPackage, tree::InstallTree};

use super::{Tree, TreeError};

// TODO: Due to the whininess of the borrow checker, we resort to cloning package information
// whenever returning it. In the future, it'd be greatly beneficial to instead return mutable
// references to the packages, which would allow for in-place manipulation of the lockfile.
// Cloning isn't destructive, but it's sure expensive.

impl Tree {
    pub fn as_rock_list(&self) -> Result<Vec<LocalPackage>, TreeError> {
        let rock_list = self.list()?;

        Ok(rock_list.values().flatten().cloned().collect())
    }
}
