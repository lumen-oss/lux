use clap::Args;
use eyre::Result;
use lux_lib::{
    config::Config,
    lockfile::LocalPackage,
    operations::{self},
    project::Project,
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

/// Returns `Some` if the `only_deps` arg is set to `false`.
pub async fn build(data: Build, config: Config) -> Result<Option<LocalPackage>> {
    let project = Project::current_or_err()?;
    let result = operations::BuildProject::new(&project, &config)
        .no_lock(data.no_lock)
        .only_deps(data.only_deps)
        .build()
        .await?;
    Ok(result)
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
    use serial_test::serial;

    #[serial]
    #[tokio::test]
    async fn test_build_project_from_vendored() {
        let cwd = &std::env::current_dir().unwrap();
        let project_dir = TempDir::new().unwrap();
        std::env::set_current_dir(&project_dir).unwrap();
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
        let toml_content = r#"
        package = "test_rock"
        version = "scm-1"

        lua = ">= 5.1"

        [dependencies]
        foo = ">= 1.0.0"
        bar = ">= 1.0.0"
        baz = "== 2.0.0"
        "#;
        let toml = project_dir.child("lux.toml");
        toml.write_str(toml_content).unwrap();
        let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));
        let config = ConfigBuilder::new()
            .unwrap()
            .vendor_dir(Some(vendor_dir.to_path_buf()))
            .lua_version(lua_version)
            .build()
            .unwrap();
        build(Build::default(), config).await.unwrap();
        std::env::set_current_dir(cwd).unwrap();
    }
}
