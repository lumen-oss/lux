use clap::Args;
use eyre::{eyre, OptionExt, Result};
use itertools::Itertools;
use lux_lib::{
    config::Config, package::PackageName, progress::MultiProgress, rockspec::lua_dependency,
    workspace::Workspace,
};

use crate::workspace::{
    sync_build_dependencies_if_locked, sync_dependencies_if_locked,
    sync_test_dependencies_if_locked,
};

#[derive(Args)]
pub struct Remove {
    /// Package or list of packages to remove from the dependencies.
    depencencies: Vec<PackageName>,

    /// Remove a development dependency.
    /// Also called `dev`.
    #[arg(short, long, alias = "dev", visible_short_aliases = ['d', 'b'])]
    build: Option<Vec<PackageName>>,

    /// Remove a test dependency.
    #[arg(short, long)]
    test: Option<Vec<PackageName>>,

    /// Package to remove from.
    #[arg(short, long, visible_short_alias = 'p')]
    package: Option<PackageName>,
}

pub async fn remove(data: Remove, config: Config) -> Result<()> {
    let mut workspace = Workspace::current()?.ok_or_eyre("No project found")?;
    let progress = MultiProgress::new_arc(&config);

    if data.package.is_none() && workspace.members().len() > 1 {
        return Err(eyre!(
            "the project to remove from must be specified with `--package`"
        ));
    }

    if !data.depencencies.is_empty() {
        for project in workspace.try_members_mut(&data.package)? {
            project
                .remove(lua_dependency::DependencyType::Regular(
                    data.depencencies.iter().collect_vec(),
                ))
                .await?;
        }
        sync_dependencies_if_locked(&workspace, progress.clone(), &config).await?;
    }

    let build_packages = data.build.unwrap_or_default();
    if !build_packages.is_empty() {
        for project in workspace.try_members_mut(&data.package)? {
            project
                .remove(lua_dependency::DependencyType::Build(
                    build_packages.iter().collect_vec(),
                ))
                .await?;
        }
        sync_build_dependencies_if_locked(&workspace, progress.clone(), &config).await?;
    }

    let test_packages = data.test.unwrap_or_default();
    if !test_packages.is_empty() {
        for project in workspace.try_members_mut(&data.package)? {
            project
                .remove(lua_dependency::DependencyType::Test(
                    test_packages.iter().collect_vec(),
                ))
                .await?;
        }
        sync_test_dependencies_if_locked(&workspace, progress.clone(), &config).await?;
    }

    Ok(())
}
