use std::io;

use crate::{
    fs,
    lockfile::{FlushLockfileError, LocalPackageId, PinnedState},
    package::PackageSpec,
    tree::{InstallTree, Tree, TreeError},
};
use fs_extra::dir::CopyOptions;
use itertools::Itertools;
use miette::Diagnostic;
use thiserror::Error;

// TODO(vhyrro): Differentiate pinned LocalPackages at the type level?

#[derive(Error, Debug, Diagnostic)]
pub enum PinError {
    #[error("package with ID '{0}' not found in the lockfile")]
    #[diagnostic(help("this is probably a bug"))]
    PackageNotFound(LocalPackageId),
    #[error("rock {rock} is already {}pinned!", if *.pin_state == PinnedState::Unpinned { "un" } else { "" })]
    PinStateUnchanged {
        pin_state: PinnedState,
        rock: PackageSpec,
    },
    #[error("cannot change the pin state of '{rock}', since a second version of '{rock}' is already installed with 'pin: {}'", .pin_state.as_bool())]
    PinStateConflict {
        pin_state: PinnedState,
        rock: PackageSpec,
    },
    #[error(transparent)]
    #[diagnostic(transparent)]
    FlushLockfile(#[from] FlushLockfileError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] TreeError),
    #[error("failed to move the old package")]
    #[diagnostic(help("make sure Lux has write access to the install directory"))]
    MoveItemsFailure(#[from] fs_extra::error::Error),
    #[error("cannot change pin state of {rock}, because it is not an entrypoint")]
    #[diagnostic(help("Lux does not allow pinning dependencies, as doing so could break the version requirement in a future update."))]
    NotAnEntrypoint { rock: PackageSpec },
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::FsError),
}

pub fn set_pinned_state(
    package_id: &LocalPackageId,
    tree: &Tree,
    pin: PinnedState,
) -> Result<(), PinError> {
    let lockfile = tree.lockfile()?;
    let mut package = lockfile
        .get(package_id)
        .ok_or_else(|| PinError::PackageNotFound(package_id.clone()))?
        .clone();

    if !lockfile.is_entrypoint(&package.id()) {
        return Err(PinError::NotAnEntrypoint {
            rock: package.to_package(),
        });
    }

    if pin == package.pinned() {
        return Err(PinError::PinStateUnchanged {
            pin_state: package.pinned(),
            rock: package.to_package(),
        });
    }

    let old_package = package.clone();
    let package_root = tree.root_for(&package);
    let items = fs::sync::read_dir(&package_root)?
        .filter_map(Result::ok)
        .map(|dir| dir.path())
        .collect_vec();

    package.spec.pinned = pin;

    if lockfile.get(&package.id()).is_some() {
        return Err(PinError::PinStateConflict {
            pin_state: package.pinned(),
            rock: package.to_package(),
        });
    }

    let new_root = tree.root_for(&package);

    fs::sync::create_dir_all(&new_root)?;

    fs_extra::move_items(&items, new_root, &CopyOptions::new())?;

    lockfile.map_then_flush(|lockfile| {
        lockfile.remove(&old_package);
        lockfile.add_entrypoint(&package);

        Ok::<_, io::Error>(())
    })?;

    Ok(())
}
