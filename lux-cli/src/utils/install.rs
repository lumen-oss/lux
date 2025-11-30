//! Utilities for converting a list of packages into a list with the correct build behaviour.

use eyre::Result;
use inquire::Confirm;
use itertools::Itertools;
use lux_lib::{
    build::BuildBehaviour,
    lockfile::{LocalPackageId, OptState, PinnedState},
    operations::install::PackageInstallSpec,
    package::PackageReq,
    tree::{self, RockMatches, Tree},
};

pub fn apply_build_behaviour(
    package_reqs: Vec<PackageReq>,
    pin: PinnedState,
    force: bool,
    tree: &Tree,
) -> Result<Vec<PackageInstallSpec>> {
    let lockfile = tree.lockfile()?;
    Ok(package_reqs
        .into_iter()
        .filter_map(|req| {
            let existing_packages: Vec<LocalPackageId> =
                match tree.match_rocks_and(&req, |rock| pin == rock.pinned()) {
                    Ok(RockMatches::Single(id)) => vec![id],
                    Ok(RockMatches::Many(ids)) => ids.into_iter().collect_vec(),
                    _ => Vec::new(),
                };
            // NOTE: Because the rock layout may change, we must force a rebuild
            // if a package is installed, but it is not an entrypoint.
            let force = force
                || existing_packages
                    .iter()
                    .all(|pkg_id| !lockfile.is_entrypoint(pkg_id));
            let build_behaviour: Option<BuildBehaviour> = if force || existing_packages.is_empty() {
                Some(BuildBehaviour::from(force))
            } else if Confirm::new(&format!("Package {req} already exists. Overwrite?"))
                .with_default(false)
                .prompt()
                .is_ok_and(|is_overwrite_confirmed| is_overwrite_confirmed)
            {
                Some(BuildBehaviour::Force)
            } else {
                None
            };
            build_behaviour.map(|build_behaviour| {
                PackageInstallSpec::new(req, tree::EntryType::Entrypoint)
                    .build_behaviour(build_behaviour)
                    .pin(pin)
                    .opt(OptState::Required)
                    .build()
            })
        })
        .collect())
}
