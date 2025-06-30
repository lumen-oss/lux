use std::{path::PathBuf, str::FromStr};

use crate::build;
use clap::Args;
use eyre::{eyre, Result};
use lux_lib::{
    build::{Build, BuildBehaviour},
    config::{Config, LuaVersion},
    lua_rockspec::RemoteLuaRockspec,
    operations::{self, Install, PackageInstallSpec},
    package::PackageReq,
    progress::MultiProgress,
    project::Project,
    rockspec::Rockspec as _,
    tree,
};
use tempdir::TempDir;

#[derive(Debug, Clone)]
pub enum PackageOrRockspec {
    Package(PackageReq),
    RockSpec(PathBuf),
}

impl FromStr for PackageOrRockspec {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let path = PathBuf::from(s);
        if path.is_file() {
            Ok(Self::RockSpec(path))
        } else {
            let pkg = PackageReq::from_str(s).map_err(|err| {
                eyre!(
                    "No file {0} found and cannot parse package query: {1}",
                    s,
                    err
                )
            })?;
            Ok(Self::Package(pkg))
        }
    }
}

#[derive(Args)]
pub struct Pack {
    /// Path to a RockSpec or a package query for a package to pack.{n}
    /// Prioritises installed rocks and will install a rock to a temporary{n}
    /// directory if none is found.{n}
    /// In case of multiple matches, the latest version will be packed.{n}
    ///{n}
    /// Examples:{n}
    ///     - "pkg"{n}
    ///     - "pkg@1.0.0"{n}
    ///     - "pkg>=1.0.0"{n}
    ///     - "/path/to/foo-1.0.0-1.rockspec"{n}
    ///{n}
    /// If not set, lux will build the current project and attempt to pack it.{n}
    /// To be able to pack a project, lux must be able to generate a release or dev{n}
    /// Lua rockspec.{n}
    #[clap(value_parser)]
    package_or_rockspec: Option<PackageOrRockspec>,
}

pub async fn pack(args: Pack, config: Config) -> Result<()> {
    let lua_version = LuaVersion::from(&config)?.clone();
    let dest_dir = std::env::current_dir()?;
    let progress = MultiProgress::new_arc();
    let result: Result<PathBuf> = match args.package_or_rockspec {
        Some(PackageOrRockspec::Package(package_req)) => {
            let user_tree = config.user_tree(lua_version.clone())?;
            match user_tree.match_rocks(&package_req)? {
                lux_lib::tree::RockMatches::NotFound(_) => {
                    let temp_dir = TempDir::new("lux-pack")?.into_path();
                    let temp_config = config.with_tree(temp_dir);
                    let tree = temp_config.user_tree(lua_version.clone())?;
                    let packages = Install::new(&temp_config)
                        .package(
                            PackageInstallSpec::new(package_req, tree::EntryType::Entrypoint)
                                .build_behaviour(BuildBehaviour::Force)
                                .build(),
                        )
                        .tree(tree.clone())
                        .progress(progress)
                        .install()
                        .await?;
                    let package = packages.first().unwrap();
                    let rock_path = operations::Pack::new(dest_dir, tree, package.clone())
                        .pack()
                        .await?;
                    Ok(rock_path)
                }
                lux_lib::tree::RockMatches::Single(local_package_id) => {
                    let lockfile = user_tree.lockfile()?;
                    let package = lockfile.get(&local_package_id).unwrap();
                    let rock_path = operations::Pack::new(dest_dir, user_tree, package.clone())
                        .pack()
                        .await?;
                    Ok(rock_path)
                }
                lux_lib::tree::RockMatches::Many(vec) => {
                    let local_package_id = vec.first().unwrap();
                    let lockfile = user_tree.lockfile()?;
                    let package = lockfile.get(local_package_id).unwrap();
                    let rock_path = operations::Pack::new(dest_dir, user_tree, package.clone())
                        .pack()
                        .await?;
                    Ok(rock_path)
                }
            }
        }
        Some(PackageOrRockspec::RockSpec(rockspec_path)) => {
            let content = std::fs::read_to_string(&rockspec_path)?;
            let rockspec = match rockspec_path
                .extension()
                .map(|ext| ext.to_string_lossy().to_string())
                .unwrap_or("".into())
                .as_str()
            {
                ".rockspec" => Ok(RemoteLuaRockspec::new(&content)?),
                _ => Err(eyre!(
                    "expected a path to a .rockspec or a package requirement."
                )),
            }?;
            let temp_dir = TempDir::new("lux-pack")?.into_path();
            let bar = progress.map(|p| p.new_bar());
            let config = config.with_tree(temp_dir);
            let tree = config.user_tree(lua_version)?;
            let package = Build::new(&rockspec, &tree, tree::EntryType::Entrypoint, &config, &bar)
                .build()
                .await?;
            let rock_path = operations::Pack::new(dest_dir, tree, package)
                .pack()
                .await?;
            Ok(rock_path)
        }
        None => {
            let project = Project::current_or_err()?;
            // luarocks expects a `<package>-<version>.rockspec` in the package root,
            // so we add a guard that it can be created here.
            project
                .toml()
                .into_remote()?
                .to_lua_remote_rockspec_string()?;
            let package = build::build(build::Build::default(), config.clone())
                .await?
                .expect("exptected a `LocalPackage`");
            let tree = project.tree(&config)?;
            let rock_path = operations::Pack::new(dest_dir, tree, package)
                .pack()
                .await?;
            Ok(rock_path)
        }
    };
    print!("packed rock created at {}", result?.display());
    Ok(())
}
