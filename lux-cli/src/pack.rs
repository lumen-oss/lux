use std::path::{Path, PathBuf};

use crate::{args::PackageOrRockspec, build, workspace::exists_matching_workspace_member};
use clap::Args;
use itertools::Itertools;
use lux_lib::{
    build::{Build, BuildBehaviour},
    config::Config,
    lua_installation::LuaInstallation,
    lua_rockspec::RemoteLuaRockspec,
    lua_version::LuaVersion,
    operations::{self, Install, PackageInstallSpec},
    package::PackageName,
    rockspec::Rockspec as _,
    tree::{self, InstallTree},
    workspace::Workspace,
};
use miette::{miette, IntoDiagnostic, Result};
use path_slash::PathBufExt;
use tempfile::tempdir;


#[derive(Args)]
pub struct Pack {
    /// Path to a RockSpec or a package query for a package to pack.{n}
    /// Prioritises local projects if in a workspace, then installed rocks.{n}
    /// If there is no matching workspace member or installed rock,{n}
    /// a rock will be downloaded and installed to a temporary directory.{n}
    /// In case of multiple matches, the latest version will be packed.{n}
    ///{n}
    /// Examples:{n}
    ///     - "pkg"{n}
    ///     - "pkg@1.0.0"{n}
    ///     - "pkg>=1.0.0"{n}
    ///     - "/path/to/foo-1.0.0-1.rockspec"{n}
    ///{n}
    /// If not set, lux will attempt to pack either all workspace members{n}
    /// or the current project.{n}
    /// To pack a project, lux must be able to generate a release or dev RockSpec.{n}
    #[clap(value_parser)]
    package_or_rockspec: Option<PackageOrRockspec>,
}

async fn pack_workspace(
    member: Option<&PackageName>,
    dest_dir: &Path,
    config: &Config,
) -> Result<Vec<PathBuf>> {
    let workspace = Workspace::current_or_err()?;

    // luarocks expects a `<package>-<version>.rockspec` in the package root,
    // so we add a guard that it can be created here.
    let packages = match member {
        // Pack only the provided workspace member
        Some(package_name) => {
            let project = workspace.select_member(package_name)?;
            project
                .toml()
                .into_remote(None)?
                .to_lua_remote_rockspec_string()?;

            let mut build = build::Build::default();
            build.package = Some(package_name.clone());
            build::build(build, config.clone())
        }
        // Pack all workspace members
        None => {
            for project in workspace.members() {
                project
                    .toml()
                    .into_remote(None)?
                    .to_lua_remote_rockspec_string()?;
            }
            build::build(build::Build::default(), config.clone())
        }
    }
    .await?;

    if packages.is_empty() {
        return Err(miette!("build did not produce a package"));
    }

    let mut rock_paths = Vec::new();
    for package in packages {
        let tree = workspace.tree(config)?;
        let rock_path = operations::Pack::new(dest_dir.to_path_buf(), tree, package)
            .pack()
            .await?;
        rock_paths.push(rock_path);
    }

    Ok(rock_paths)
}

pub async fn pack(args: Pack, config: Config) -> Result<()> {
    let lua_version = LuaVersion::from(&config)?.clone();
    let dest_dir = std::env::current_dir().into_diagnostic()?;
    let rock_paths: Vec<PathBuf> = match args.package_or_rockspec {
        Some(PackageOrRockspec::Package(package_req))
            if exists_matching_workspace_member(&package_req)? =>
        {
            pack_workspace(Some(package_req.name()), &dest_dir, &config).await
        }
        Some(PackageOrRockspec::Package(package_req)) => {
            let user_tree = config.user_tree(lua_version.clone())?;
            match user_tree.match_rocks(&package_req)? {
                lux_lib::tree::RockMatches::NotFound(_) => {
                    let temp_dir = tempdir().into_diagnostic()?;
                    let temp_config = config.with_tree(temp_dir.path().to_path_buf());
                    let tree = temp_config.user_tree(lua_version.clone())?;
                    let packages = Install::new(&temp_config)
                        .package(
                            PackageInstallSpec::new(package_req, tree::EntryType::Entrypoint)
                                .build_behaviour(BuildBehaviour::Force)
                                .build(),
                        )
                        .tree(tree.clone())
                        .install()
                        .await?;
                    let package = packages
                        .first()
                        .ok_or_else(|| miette!("no packages installed"))?;
                    let rock_path = operations::Pack::new(dest_dir, tree, package.clone())
                        .pack()
                        .await?;
                    Ok(vec![rock_path])
                }
                lux_lib::tree::RockMatches::Single(local_package_id) => {
                    let lockfile = user_tree.lockfile()?;
                    let package = lockfile.get(&local_package_id).ok_or_else(|| {
                        miette!("package is installed, but was not found in the lockfile")
                    })?;
                    let rock_path = operations::Pack::new(dest_dir, user_tree, package.clone())
                        .pack()
                        .await?;
                    Ok(vec![rock_path])
                }
                lux_lib::tree::RockMatches::Many(vec) => {
                    let local_package_id = vec.first();
                    let lockfile = user_tree.lockfile()?;
                    let package = lockfile.get(local_package_id).ok_or_else(|| {
                        miette!(
                            "multiple package installations found, but not found in the lockfile"
                        )
                    })?;
                    let rock_path = operations::Pack::new(dest_dir, user_tree, package.clone())
                        .pack()
                        .await?;
                    Ok(vec![rock_path])
                }
            }
        }
        Some(PackageOrRockspec::RockSpec(rockspec_path)) => {
            let content = tokio::fs::read_to_string(&rockspec_path)
                .await
                .into_diagnostic()?;
            let rockspec = match rockspec_path
                .extension()
                .map(|ext| ext.to_string_lossy().to_string())
                .unwrap_or("".into())
                .as_str()
            {
                "rockspec" => Ok(RemoteLuaRockspec::new(&content)?),
                _ => Err(miette!(
                    "expected a path to a .rockspec or a package requirement."
                )),
            }?;
            let temp_dir = tempdir().into_diagnostic()?;
            let config = config.with_tree(temp_dir.path().to_path_buf());
            let lua = LuaInstallation::new(
                &lua_version,
                &config,
            )
            .await?;
            let tree = config.user_tree(lua_version)?;
            let package = Build::new()
                .rockspec(&rockspec)
                .lua(&lua)
                .tree(&tree)
                .entry_type(tree::EntryType::Entrypoint)
                .config(&config)
                .build()
                .await?;
            let rock_path = operations::Pack::new(dest_dir, tree, package)
                .pack()
                .await?;
            Ok(vec![rock_path])
        }
        None => pack_workspace(None, &dest_dir, &config).await,
    }?;

    if rock_paths.len() > 1 {
        let rock_paths = rock_paths
            .iter()
            .map(|path| path.to_slash_lossy().to_string())
            .join("\n");
        print!("packed rocks created at\n{}", rock_paths)
    } else {
        rock_paths
            .first()
            .iter()
            .for_each(|path| print!("packed rock created at {}", path.display()));
    }
    Ok(())
}
