//! Utilities for converting a list of packages into a list with the correct build behaviour.

use inquire::Confirm;
use lux_lib::{
    build::BuildBehaviour,
    config::Config,
    lockfile::{OptState, PinnedState},
    operations::install::PackageInstallSpec,
    package::PackageReq,
    tree::{self, InstallTree, Tree},
};
use miette::Result;

pub fn apply_build_behaviour(
    package_reqs: Vec<PackageReq>,
    pin: PinnedState,
    force: bool,
    tree: &Tree,
    config: &Config,
) -> Result<Vec<PackageInstallSpec>> {
    let lockfile = tree.lockfile()?;
    Ok(package_reqs
        .into_iter()
        .filter_map(|req| {
            // Look up any existing entrypoint by name, regardless of version.
            let build_behaviour = match lockfile.entrypoint(req.name()) {
                Some(existing) => {
                    let overwrite = force
                        || (!config.no_prompt()
                            && Confirm::new(&format!(
                                "Package {}@{} already exists. Overwrite?",
                                existing.name(),
                                existing.version()
                            ))
                            .with_default(false)
                            .prompt()
                            .is_ok_and(|overwrite_confirmed| overwrite_confirmed));
                    overwrite.then_some(BuildBehaviour::Force)
                }
                // No entrypoint exists, so always force install. This will force dependencies
                // designated as entrypoints to be rebuilt, since their layouts may change during
                // reinstall.
                None => Some(BuildBehaviour::Force),
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
