use miette::miette;
use std::path::PathBuf;

use clap::Args;
use lux_lib::{
    config::Config,
    lockfile::PinnedState,
    lua_rockspec::RemoteLuaRockspec,
    operations::{self},
    rockspec::LuaVersionCompatibility,
};

use miette::{IntoDiagnostic, Result};

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
        return Err(miette!("provided path is not a valid rockspec!"));
    }

    let content = std::fs::read_to_string(path).into_diagnostic()?;

    let rockspec = RemoteLuaRockspec::new(&content)?;

    let lua_version = rockspec.lua_version_matches(&config)?;
    let tree = config.user_tree(lua_version)?;

    operations::InstallRockspec::new()
        .rockspec(rockspec)
        .pin(pin)
        .config(&config)
        .tree(&tree)
        .install()
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
        config::ConfigBuilder, lua_installation::detect_installed_lua_version,
        lua_version::LuaVersion,
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
