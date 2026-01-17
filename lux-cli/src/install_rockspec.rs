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

#[cfg(test)]
mod tests {

    use super::*;

    use assert_fs::{
        prelude::{FileWriteStr, PathChild, PathCreateDir},
        TempDir,
    };

    use lux_lib::{
        config::{ConfigBuilder, LuaVersion},
        lua_installation::detect_installed_lua_version,
    };

    #[tokio::test]
    async fn test_install_rockspec_from_vendored() {
        // This test runs without a network connection when run with Nix
        let vendor_dir = TempDir::new().unwrap();
        let foo_dir = vendor_dir.child("foo@1.0.0-1");
        foo_dir.create_dir_all().unwrap();
        let foo_rockspec = vendor_dir.child("foo-1.0.0-1.rockspec");
        foo_rockspec
            .write_str(
                r#"
                package = 'foo'
                version = '1.0.0-1'
                source = {
                    url = 'https://github.com/lumen-oss/luarocks-stub',
                }
            "#,
            )
            .unwrap();
        let bar_dir = vendor_dir.child("bar@2.0.0-2");
        bar_dir.create_dir_all().unwrap();
        let bar_rockspec = vendor_dir.child("bar-2.0.0-2.rockspec");
        bar_rockspec
            .write_str(
                r#"
                package = 'bar'
                version = '2.0.0-2'
                source = {
                    url = 'https://github.com/lumen-oss/luarocks-stub',
                }
            "#,
            )
            .unwrap();
        let baz_dir = vendor_dir.child("baz@2.0.0-1");
        baz_dir.create_dir_all().unwrap();
        let baz_rockspec = vendor_dir.child("baz-2.0.0-1.rockspec");
        baz_rockspec
            .write_str(
                r#"
                package = 'baz'
                version = '2.0.0-1'
                source = {
                    url = 'https://github.com/lumen-oss/luarocks-stub',
                }
            "#,
            )
            .unwrap();
        let test_rock_dir = vendor_dir.child("test_rock@scm-1");
        test_rock_dir.create_dir_all().unwrap();
        let rockspec_content = r#"
        package = 'test_rock'
        version = 'scm-1'
        source = {
            url = 'https://github.com/lumen-oss/luarocks-stub',
        }
        dependencies = {
            'foo >= 1.0.0',
            'bar',
            'baz == 2.0.0',
        }
        "#;
        let temp_dir = TempDir::new().unwrap();
        let rockspec = temp_dir.child("test_rock-scm-1.rockspec");
        rockspec.write_str(rockspec_content).unwrap();
        let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));
        let config = ConfigBuilder::new()
            .unwrap()
            .vendor_dir(Some(vendor_dir.to_path_buf()))
            .lua_version(lua_version)
            .user_tree(Some(temp_dir.to_path_buf()))
            .build()
            .unwrap();
        install_rockspec(
            InstallRockspec {
                rockspec_path: rockspec.to_path_buf(),
                pin: false,
            },
            config,
        )
        .await
        .unwrap()
    }
}
