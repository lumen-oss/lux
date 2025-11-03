use clap::Args;
use eyre::{eyre, Result};
use inquire::Confirm;
use itertools::Itertools;
use lux_lib::{
    build::BuildBehaviour,
    config::{Config, LuaVersion},
    lockfile::LocalPackageId,
    operations::{self, PackageInstallSpec},
    package::PackageReq,
    progress::MultiProgress,
    tree::{self, RockMatches, TreeError},
};

#[derive(Args)]
pub struct Uninstall {
    /// The package or packages to uninstall from the system.
    packages: Vec<PackageReq>,
}

/// Uninstall one or multiple rocks from the user tree
pub async fn uninstall(uninstall_args: Uninstall, config: Config) -> Result<()> {
    let tree = config.user_tree(LuaVersion::from(&config)?.clone())?;

    let package_matches = uninstall_args
        .packages
        .iter()
        .map(|package_req| tree.match_rocks(package_req))
        .try_collect::<_, Vec<_>, TreeError>()?;

    let (packages, nonexistent_packages, duplicate_packages) = package_matches.into_iter().fold(
        (Vec::new(), Vec::new(), Vec::new()),
        |(mut p, mut n, mut d), rock_match| {
            match rock_match {
                RockMatches::NotFound(req) => n.push(req),
                RockMatches::Single(package) => p.push(package),
                RockMatches::Many(packages) => d.extend(packages),
            };

            (p, n, d)
        },
    );

    if !nonexistent_packages.is_empty() {
        // TODO(vhyrro): Render this in the form of a tree.
        return Err(eyre!(
            "The following packages were not found: {:#?}",
            nonexistent_packages
        ));
    }

    if !duplicate_packages.is_empty() {
        return Err(eyre!(
            "
Multiple packages satisfying your version requirements were found:
{:#?}

Please specify the exact package to uninstall:
> lux uninstall '<name>@<version>'
",
            duplicate_packages,
        ));
    }

    let lockfile = tree.lockfile()?;
    let non_entrypoints = packages
        .iter()
        .filter_map(|pkg_id| {
            if lockfile.is_entrypoint(pkg_id) {
                None
            } else {
                Some(unsafe { lockfile.get_unchecked(pkg_id) }.name().to_string())
            }
        })
        .collect_vec();
    if !non_entrypoints.is_empty() {
        return Err(eyre!(
            "
Cannot uninstall dependencies:
{:#?}
",
            non_entrypoints,
        ));
    }

    let (dependencies, entrypoints): (Vec<LocalPackageId>, Vec<LocalPackageId>) = packages
        .iter()
        .cloned()
        .partition(|pkg_id| lockfile.is_dependency(pkg_id));

    let progress = MultiProgress::new_arc(&config);

    if dependencies.is_empty() {
        operations::Uninstall::new()
            .config(&config)
            .packages(entrypoints)
            .remove()
            .await?;
    } else {
        let package_names = dependencies
            .iter()
            .map(|pkg_id| unsafe { lockfile.get_unchecked(pkg_id) }.name().to_string())
            .collect_vec();
        let prompt = if package_names.len() == 1 {
            format!(
                "
            Package {} can be removed from the entrypoints, but it is also a dependency, so it will have to be reinstalled.
Reinstall?
            ",
                package_names[0]
            )
        } else {
            format!(
                "
            The following packages can be removed from the entrypoints, but are also dependencies:
{package_names:#?}

They will have to be reinstalled.
Reinstall?
            ",
            )
        };
        if Confirm::new(&prompt)
            .with_default(false)
            .prompt()
            .expect("Error prompting for reinstall")
        {
            operations::Uninstall::new()
                .config(&config)
                .packages(entrypoints)
                .progress(progress.clone())
                .remove()
                .await?;

            let reinstall_specs = dependencies
                .iter()
                .map(|pkg_id| {
                    let package = unsafe { lockfile.get_unchecked(pkg_id) };
                    PackageInstallSpec::new(
                        package.clone().into_package_req(),
                        tree::EntryType::DependencyOnly,
                    )
                    .build_behaviour(BuildBehaviour::Force)
                    .pin(package.pinned())
                    .opt(package.opt())
                    .constraint(package.constraint())
                    .build()
                })
                .collect_vec();
            operations::Uninstall::new()
                .config(&config)
                .packages(dependencies)
                .progress(progress.clone())
                .remove()
                .await?;
            operations::Install::new(&config)
                .packages(reinstall_specs)
                .tree(tree)
                .progress(progress.clone())
                .install()
                .await?;
        } else {
            return Err(eyre!("Operation cancelled."));
        }
    };

    let mut has_dangling_rocks = true;
    while has_dangling_rocks {
        let tree = config.user_tree(LuaVersion::from(&config)?.clone())?;
        let lockfile = tree.lockfile()?;
        let dangling_rocks = lockfile
            .rocks()
            .iter()
            .filter_map(|(pkg_id, _)| {
                if lockfile.is_entrypoint(pkg_id) || lockfile.is_dependency(pkg_id) {
                    None
                } else {
                    Some(pkg_id)
                }
            })
            .cloned()
            .collect_vec();
        if dangling_rocks.is_empty() {
            has_dangling_rocks = false
        } else {
            operations::Uninstall::new()
                .config(&config)
                .packages(dangling_rocks)
                .progress(progress.clone())
                .remove()
                .await?;
        }
    }

    Ok(())
}
