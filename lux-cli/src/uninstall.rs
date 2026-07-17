use clap::Args;
use inquire::Confirm;
use itertools::Itertools;
use lux_lib::{
    build::BuildBehaviour,
    config::Config,
    lockfile::LocalPackageId,
    lua_version::LuaVersion,
    operations::{self, PackageInstallSpec},
    package::PackageReq,
    tree::{self, InstallTree, RockMatches, TreeError},
};

use miette::{miette, IntoDiagnostic, Result};

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
        return Err(miette!(
            "The following packages were not found: {:#?}",
            nonexistent_packages
        ));
    }

    if !duplicate_packages.is_empty() {
        return Err(miette!(
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
        return Err(miette!(
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
        if !config.no_prompt()
            && Confirm::new(&prompt)
                .with_default(false)
                .prompt()
                .into_diagnostic()
                .map_err(|_| miette!("Error prompting for reinstall"))?
        {
            operations::Uninstall::new()
                .config(&config)
                .packages(entrypoints)
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
                .remove()
                .await?;
            operations::Install::new(&config)
                .packages(reinstall_specs)
                .tree(tree)
                .install()
                .await?;
        } else {
            return Err(miette!("Operation cancelled."));
        }
    };

    let mut has_dangling_rocks = true;
    while has_dangling_rocks {
        let tree = config.user_tree(LuaVersion::from(&config)?.clone())?;
        let lockfile = tree.lockfile()?;
        let dangling_rocks = lockfile
            .rocks()
            .keys()
            .filter(|pkg_id| !lockfile.is_entrypoint(pkg_id) && !lockfile.is_dependency(pkg_id))
            .cloned()
            .collect_vec();
        if dangling_rocks.is_empty() {
            has_dangling_rocks = false
        } else {
            operations::Uninstall::new()
                .config(&config)
                .packages(dangling_rocks)
                .remove()
                .await?;
        }
    }

    Ok(())
}
