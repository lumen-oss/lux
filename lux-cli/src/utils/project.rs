use std::{str::FromStr, sync::Arc};

use eyre::{Context, Result};
use lux_lib::{
    config::{Config, LuaVersion},
    git::shorthand::RemoteGitUrlShorthand,
    operations::Sync,
    package::PackageReq,
    progress::{MultiProgress, Progress},
    project::Project,
    tree::Tree,
};

/// Used for parsing alternatives between a git URL shorthand and a package requirement.
// The `FromStr` instance tries to parse a git URL shorthand first (expecting a git host prefix),
// and then a package requirement.
#[derive(Debug, Clone)]
pub enum PackageReqOrGitShorthand {
    PackageReq(PackageReq),
    GitShorthand(RemoteGitUrlShorthand),
}

impl FromStr for PackageReqOrGitShorthand {
    type Err = eyre::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match RemoteGitUrlShorthand::parse_with_prefix(s) {
            Ok(shorthand) => Ok(Self::GitShorthand(shorthand)),
            Err(_) => Ok(Self::PackageReq(PackageReq::parse(s)?)),
        }
    }
}

/// Get the current project's tree, or fall back to
/// the user tree if not in a project
pub fn current_project_or_user_tree(config: &Config) -> Result<Tree> {
    let project = Project::current()?;
    Ok(match &project {
        Some(project) => project.tree(config)?,
        None => {
            let lua_version = LuaVersion::from(config)?.clone();
            config.user_tree(lua_version)?
        }
    })
}

pub async fn sync_dependencies_if_locked(
    project: &Project,
    progress: Arc<Progress<MultiProgress>>,
    config: &Config,
) -> Result<()> {
    // NOTE: We only update the lockfile if one exists.
    // Otherwise, the next `lx build` will remove the packages.
    Sync::new(project, config)
        .progress(progress)
        .sync_dependencies()
        .await
        .wrap_err("syncing dependencies with the project lockfile failed.")?;
    Ok(())
}

pub async fn sync_build_dependencies_if_locked(
    project: &Project,
    progress: Arc<Progress<MultiProgress>>,
    config: &Config,
) -> Result<()> {
    Sync::new(project, config)
        .progress(progress.clone())
        .sync_build_dependencies()
        .await
        .wrap_err("syncing build dependencies with the project lockfile failed.")?;
    Ok(())
}

pub async fn sync_test_dependencies_if_locked(
    project: &Project,
    progress: Arc<Progress<MultiProgress>>,
    config: &Config,
) -> Result<()> {
    Sync::new(project, config)
        .progress(progress.clone())
        .sync_test_dependencies()
        .await
        .wrap_err("syncing test dependencies with the project lockfile failed.")?;
    Ok(())
}
