use clap::Args;
use eyre::eyre;
use eyre::Context;
use eyre::Result;
use itertools::Itertools;
use lux_lib::config::Config;
use lux_lib::lockfile::PinnedState;
use lux_lib::lua_version::LuaVersion;
use lux_lib::operations;
use lux_lib::package::PackageName;
use lux_lib::package::PackageReq;
use lux_lib::progress::MultiProgress;
use lux_lib::rockspec::lua_dependency;
use lux_lib::tree::RockMatches;
use lux_lib::workspace::Workspace;

#[derive(Args)]
pub struct ChangePin {
    /// Installed package or dependency to pin.
    /// If pinning a dependency in a project, this should
    /// be the package name.
    package_req: Vec<PackageReq>,

    /// Pin a development dependency.
    /// Also called `dev`.
    #[arg(short, long, alias = "dev", visible_short_aliases = ['d', 'b'])]
    build: Option<Vec<PackageName>>,

    /// Pin a test dependency.
    #[arg(short, long)]
    test: Option<Vec<PackageName>>,

    /// Project to modify.
    #[arg(short, long, visible_short_alias = 'p')]
    package: Option<PackageName>,
}

pub async fn set_pinned_state(data: ChangePin, config: Config, pin: PinnedState) -> Result<()> {
    match Workspace::current()? {
        Some(mut workspace) => {
            if data.package.is_none() && workspace.members().len() > 1 {
                return Err(eyre!(
                    "the project to modify must be specified with `--package`"
                ));
            }
            let progress = MultiProgress::new_arc(&config);

            if data
                .package_req
                .iter()
                .any(|pkg| !pkg.version_req().is_any())
            {
                return Err(eyre!(
                    "Cannot pin project dependencies using version constraints."
                ));
            }
            let packages = data
                .package_req
                .iter()
                .map(|pkg| pkg.name())
                .cloned()
                .collect_vec();
            if !packages.is_empty() {
                for project in workspace.try_members_mut(&data.package)? {
                    project
                        .set_pinned_state(
                            lua_dependency::LuaDependencyType::Regular(packages.iter().collect()),
                            pin,
                        )
                        .await?;
                }
                operations::Sync::new(&workspace, &config)
                    .progress(progress.clone())
                    .sync_dependencies()
                    .await
                    .wrap_err("syncing dependencies with the project lockfile failed.")?;
            }
            let build_packages = data.build.unwrap_or_default();
            if !build_packages.is_empty() {
                for project in workspace.try_members_mut(&data.package)? {
                    project
                        .set_pinned_state(
                            lua_dependency::LuaDependencyType::Build(
                                build_packages.iter().collect(),
                            ),
                            pin,
                        )
                        .await?;
                }
                operations::Sync::new(&workspace, &config)
                    .progress(progress.clone())
                    .sync_build_dependencies()
                    .await
                    .wrap_err("syncing build dependencies with the project lockfile failed.")?;
            }
            let test_packages = data.test.unwrap_or_default();
            if !test_packages.is_empty() {
                for project in workspace.try_members_mut(&data.package)? {
                    project
                        .set_pinned_state(
                            lua_dependency::LuaDependencyType::Test(test_packages.iter().collect()),
                            pin,
                        )
                        .await?;
                }
                operations::Sync::new(&workspace, &config)
                    .progress(progress.clone())
                    .sync_test_dependencies()
                    .await
                    .wrap_err("syncing test dependencies with the project lockfile failed.")?;
            }
        }
        None => {
            let tree = config.user_tree(LuaVersion::from(&config)?.clone())?;

            for package in &data.package_req {
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
