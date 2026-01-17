use eyre::eyre;
use std::{path::PathBuf, sync::Arc};

use clap::Args;
use eyre::Result;
use lux_lib::{
    build::{self, BuildBehaviour},
    config::Config,
    lockfile::{OptState, PinnedState},
    lua_installation::LuaInstallation,
    lua_rockspec::{BuildBackendSpec, RemoteLuaRockspec},
    luarocks::luarocks_installation::LuaRocksInstallation,
    operations::{Install, PackageInstallSpec},
    progress::MultiProgress,
    rockspec::{LuaVersionCompatibility, Rockspec},
    tree,
};

#[derive(Args, Default)]
pub struct InstallRockspec {
    /// The path to the RockSpec file to install
    rockspec_path: PathBuf,

    /// Whether to pin the installed package and dependencies.
    #[arg(long)]
    pin: bool,
}

/// Install a rockspec into the user tree.
pub async fn install_rockspec(data: InstallRockspec, config: Config) -> Result<()> {
    let pin = PinnedState::from(data.pin);
    let path = data.rockspec_path;

    if path
        .extension()
        .map(|ext| ext != "rockspec")
        .unwrap_or(true)
    {
        return Err(eyre!("Provided path is not a valid rockspec!"));
    }

    let progress_arc = MultiProgress::new_arc(&config);
    let progress = Arc::clone(&progress_arc);

    let content = std::fs::read_to_string(path)?;
    let rockspec = RemoteLuaRockspec::new(&content)?;
    let lua_version = rockspec.lua_version_matches(&config)?;
    let lua = LuaInstallation::new(
        &lua_version,
        &config,
        &progress.map(|progress| progress.new_bar()),
    )
    .await?;
    let tree = config.user_tree(lua_version)?;

    // Ensure all dependencies and build dependencies are installed first

    let build_dependencies = rockspec.build_dependencies().current_platform();

    let build_dependencies_to_install = build_dependencies
        .iter()
        .filter(|dep| {
            // Exclude luarocks build backends that we have implemented in lux
            !matches!(
                dep.name().to_string().as_str(),
                "luarocks-build-rust-mlua" | "luarocks-build-treesitter-parser"
            )
        })
        .filter(|dep| {
            tree.match_rocks(dep.package_req())
                .is_ok_and(|rock_match| rock_match.is_found())
        })
        .map(|dep| {
            PackageInstallSpec::new(dep.package_req().clone(), tree::EntryType::Entrypoint)
                .build_behaviour(BuildBehaviour::NoForce)
                .pin(pin)
                .opt(OptState::Required)
                .maybe_source(dep.source().clone())
                .build()
        })
        .collect();

    Install::new(&config)
        .packages(build_dependencies_to_install)
        .tree(tree.build_tree(&config)?)
        .progress(progress_arc.clone())
        .install()
        .await?;

    let dependencies = rockspec.dependencies().current_platform();

    let mut dependencies_to_install = Vec::new();
    for dep in dependencies {
        let rock_match = tree.match_rocks(dep.package_req())?;
        if !rock_match.is_found() {
            let dep =
                PackageInstallSpec::new(dep.package_req().clone(), tree::EntryType::DependencyOnly)
                    .build_behaviour(BuildBehaviour::NoForce)
                    .pin(pin)
                    .opt(OptState::Required)
                    .maybe_source(dep.source().clone())
                    .build();
            dependencies_to_install.push(dep);
        }
    }

    Install::new(&config)
        .packages(dependencies_to_install)
        .tree(tree.clone())
        .progress(progress_arc.clone())
        .install()
        .await?;

    if let Some(BuildBackendSpec::LuaRock(_)) = &rockspec.build().current_platform().build_backend {
        let build_tree = tree.build_tree(&config)?;
        let luarocks = LuaRocksInstallation::new(&config, build_tree)?;
        let bar = progress.map(|p| p.new_bar());
        luarocks.ensure_installed(&lua, &bar).await?;
    }

    build::Build::new()
        .rockspec(&rockspec)
        .tree(&tree)
        .lua(&lua)
        .entry_type(tree::EntryType::Entrypoint)
        .config(&config)
        .progress(&progress.map(|p| p.new_bar()))
        .pin(pin)
        .behaviour(BuildBehaviour::Force)
        .build()
        .await?;

    Ok(())
}
