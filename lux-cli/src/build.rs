use eyre::{Context, OptionExt};
use itertools::Itertools;
use std::sync::Arc;

use clap::Args;
use eyre::Result;
use lux_lib::{
    build::{self, BuildBehaviour},
    config::Config,
    luarocks::luarocks_installation::LuaRocksInstallation,
    operations::{Install, PackageInstallSpec, Sync},
    package::PackageName,
    progress::MultiProgress,
    project::Project,
    rockspec::Rockspec,
    tree,
};

#[derive(Args, Default)]
pub struct Build {
    /// Ignore the project's lockfile and don't create one.
    #[arg(long)]
    no_lock: bool,

    /// Build only the dependencies
    #[arg(long)]
    only_deps: bool,
}

pub async fn build(data: Build, config: Config) -> Result<()> {
    let project = Project::current()?.ok_or_eyre("Not in a project!")?;
    let progress_arc = MultiProgress::new_arc();
    let progress = Arc::clone(&progress_arc);

    let tree = project.tree(&config)?;
    let project_toml = project.toml().into_local()?;

    let dependencies = project_toml
        .dependencies()
        .current_platform()
        .iter()
        .filter(|package| !package.name().eq(&PackageName::new("lua".into())))
        .cloned()
        .collect_vec();

    let build_dependencies = project_toml
        .build_dependencies()
        .current_platform()
        .iter()
        .filter(|package| !package.name().eq(&PackageName::new("lua".into())))
        .cloned()
        .collect_vec();

    let luarocks = LuaRocksInstallation::new(&config)?;

    if data.no_lock {
        let dependencies_to_install = dependencies
            .into_iter()
            .filter(|dep| {
                tree.match_rocks(dep.package_req())
                    .is_ok_and(|rock_match| !rock_match.is_found())
            })
            .map(|dep| {
                PackageInstallSpec::new(dep.clone().into_package_req(), tree::EntryType::Entrypoint)
                    .pin(*dep.pin())
                    .opt(*dep.opt())
                    .maybe_source(dep.source().clone())
                    .build()
            });

        Install::new(&tree, &config)
            .packages(dependencies_to_install)
            .project(&project)
            .progress(progress.clone())
            .install()
            .await?;

        let build_dependencies_to_install = build_dependencies
            .into_iter()
            .filter(|dep| {
                tree.match_rocks(dep.package_req())
                    .is_ok_and(|rock_match| !rock_match.is_found())
            })
            .map(|dep| {
                PackageInstallSpec::new(dep.clone().into_package_req(), tree::EntryType::Entrypoint)
                    .pin(*dep.pin())
                    .opt(*dep.opt())
                    .maybe_source(dep.source().clone())
                    .build()
            })
            .collect_vec();

        if !build_dependencies_to_install.is_empty() {
            let bar = progress.map(|p| p.new_bar());
            luarocks.ensure_installed(&bar).await?;
            Install::new(luarocks.tree(), luarocks.config())
                .packages(build_dependencies_to_install)
                .project(&project)
                .progress(progress.clone())
                .install()
                .await?;
        }
    } else {
        let mut project_lockfile = project.lockfile()?.write_guard();

        Sync::new(&tree, &mut project_lockfile, &config)
            .progress(progress.clone())
            .packages(dependencies)
            .sync_dependencies()
            .await
            .wrap_err(
                "
syncing dependencies with the project lockfile failed.
Use --no-lock to force a new build.
",
            )?;

        Sync::new(luarocks.tree(), &mut project_lockfile, luarocks.config())
            .progress(progress.clone())
            .packages(build_dependencies)
            .sync_build_dependencies()
            .await
            .wrap_err(
                "
syncing build dependencies with the project lockfile failed.
Use --no-lock to force a new build.
",
            )?;
    }

    if !data.only_deps {
        let package = build::Build::new(
            &project_toml,
            &tree,
            tree::EntryType::Entrypoint,
            &config,
            &progress.map(|p| p.new_bar()),
        )
        .behaviour(BuildBehaviour::Force)
        .build()
        .await?;

        let lockfile = tree.lockfile()?;
        let dependencies = lockfile
            .rocks()
            .iter()
            .filter_map(|(pkg_id, value)| {
                if lockfile.is_entrypoint(pkg_id) {
                    Some(value)
                } else {
                    None
                }
            })
            .cloned()
            .collect_vec();
        let mut lockfile = lockfile.write_guard();
        lockfile.add_entrypoint(&package);
        for dep in dependencies {
            lockfile.add_dependency(&package, &dep);
        }
    }

    Ok(())
}
