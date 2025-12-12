use clap::Args;
use eyre::eyre;
use eyre::Context;
use eyre::Result;
use itertools::Itertools;
use lux_lib::config::{Config, LuaVersion};
use lux_lib::lockfile::PinnedState;
use lux_lib::operations;
use lux_lib::package::PackageName;
use lux_lib::package::PackageReq;
use lux_lib::progress::MultiProgress;
use lux_lib::project::Project;
use lux_lib::rockspec::lua_dependency;
use lux_lib::tree::RockMatches;

#[derive(Args)]
pub struct ChangePin {
    /// Installed package or dependency to pin.
    /// If pinning a dependency in a project, this should
    /// be the package name.
    package: Vec<PackageReq>,

    /// Pin a development dependency.
    /// Also called `dev`.
    #[arg(short, long, alias = "dev", visible_short_aliases = ['d', 'b'])]
    build: Option<Vec<PackageName>>,

    /// Pin a test dependency.
    #[arg(short, long)]
    test: Option<Vec<PackageName>>,
}

pub async fn set_pinned_state(data: ChangePin, config: Config, pin: PinnedState) -> Result<()> {
    match Project::current()? {
        Some(mut project) => {
            let progress = MultiProgress::new_arc(&config);
            if data.package.iter().any(|pkg| !pkg.version_req().is_any()) {
                return Err(eyre!(
                    "Cannot pin project dependencies using version constraints."
                ));
            }
            let packages = data
                .package
                .iter()
                .map(|pkg| pkg.name())
                .cloned()
                .collect_vec();
            if !packages.is_empty() {
                project
                    .set_pinned_state(lua_dependency::LuaDependencyType::Regular(packages), pin)
                    .await?;
                operations::Sync::new(&project, &config)
                    .progress(progress.clone())
                    .sync_dependencies()
                    .await
                    .wrap_err("syncing dependencies with the project lockfile failed.")?;
            }
            let build_packages = data.build.unwrap_or_default();
            if !build_packages.is_empty() {
                project
                    .set_pinned_state(
                        lua_dependency::LuaDependencyType::Build(build_packages),
                        pin,
                    )
                    .await?;
                operations::Sync::new(&project, &config)
                    .progress(progress.clone())
                    .sync_build_dependencies()
                    .await
                    .wrap_err("syncing build dependencies with the project lockfile failed.")?;
            }
            let test_packages = data.test.unwrap_or_default();
            if !test_packages.is_empty() {
                project
                    .set_pinned_state(lua_dependency::LuaDependencyType::Test(test_packages), pin)
                    .await?;
                operations::Sync::new(&project, &config)
                    .progress(progress.clone())
                    .sync_test_dependencies()
                    .await
                    .wrap_err("syncing test dependencies with the project lockfile failed.")?;
            }
        }
        None => {
            let tree = config.user_tree(LuaVersion::from_config(&config)?.clone())?;

            for package in &data.package {
                match tree.match_rocks_and(package, |package| pin != package.pinned())? {
                    RockMatches::Single(rock) => {
                        operations::set_pinned_state(&rock, &tree, pin)?;
                    }
                    RockMatches::Many(_) => {
                        todo!("Add an error here about many conflicting types and to use `all:`")
                    }
                    RockMatches::NotFound(_) => return Err(eyre!("Rock {} not found!", package)),
                }
            }
        }
    }
    Ok(())
}
