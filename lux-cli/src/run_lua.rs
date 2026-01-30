use std::path::PathBuf;

use path_slash::PathExt;
use tokio::process::Command;

use clap::Args;
use eyre::{eyre, Result};
use itertools::Itertools;
use lux_lib::{
    config::{Config, LuaVersion},
    lua_installation::{LuaBinary, LuaInstallation},
    operations,
    progress::MultiProgress,
    project::Project,
    rockspec::LuaVersionCompatibility,
};

use crate::build::{self, Build};

#[derive(Args, Default)]
#[clap(disable_help_flag = true)]
pub struct RunLua {
    /// Arguments to pass to Lua. See `lua -h`.
    args: Option<Vec<String>>,

    /// Path to the Lua interpreter to use.
    #[arg(long)]
    lua: Option<String>,

    /// Add test dependencies to the environment.
    #[arg(long)]
    test: bool,

    /// Add build dependencies to the environment.
    #[arg(long)]
    build: bool,

    /// Disable the Lux loader.
    /// If a rock has conflicting transitive dependencies,
    /// disabling the Lux loader may result in the wrong modules being loaded.
    #[clap(default_value_t = false)]
    #[arg(long)]
    no_loader: bool,

    #[clap(flatten)]
    build_args: Build,

    /// Print help
    #[arg(long)]
    help: bool,
}

pub async fn run_lua(run_lua: RunLua, config: Config) -> Result<()> {
    let project = Project::current()?;
    let (lua_version, root, tree, mut welcome_message) = match &project {
        Some(project) => (
            project.toml().lua_version_matches(&config)?,
            project.root().to_path_buf(),
            project.tree(&config)?,
            format!(
                "Welcome to the lux Lua repl for {}.",
                project.toml().package()
            ),
        ),
        None => {
            let version = LuaVersion::from(&config)?.clone();
            (
                version.clone(),
                std::env::current_dir()?,
                config.user_tree(version)?,
                "Welcome to the lux Lua repl.".into(),
            )
        }
    };

    welcome_message = format!(
        r#"{welcome_message}
Run `lx lua --help` for options.
To exit type 'exit()' or <C-d>.
"#,
    );

    let lua_cmd = match run_lua.lua.map(LuaBinary::Custom) {
        Some(lua_cmd) => lua_cmd,
        None => {
            let progress = MultiProgress::new_arc(&config);
            let bar = progress.map(|progress| progress.new_bar());
            let lua_cmd = LuaInstallation::new(&lua_version, &config, &bar)
                .await?
                .bin()
                .as_ref()
                .map(|lua_bin| LuaBinary::Custom(lua_bin.to_slash_lossy().to_string()))
                .unwrap_or_else(|| LuaBinary::new(lua_version, &config));
            bar.map(|bar| bar.finish_and_clear());
            lua_cmd
        }
    };

    if run_lua.help {
        return print_lua_help(&lua_cmd).await;
    }

    if project.is_some() {
        build::build(run_lua.build_args, config.clone()).await?;
    }

    let args = &run_lua.args.unwrap_or_default();

    operations::RunLua::new()
        .root(&root)
        .tree(&tree)
        .config(&config)
        .lua_cmd(lua_cmd)
        .args(args)
        .prepend_test_paths(run_lua.test)
        .prepend_build_paths(run_lua.build)
        .disable_loader(run_lua.no_loader)
        .lua_init("exit = os.exit".to_string())
        .welcome_message(welcome_message)
        .run_lua()
        .await?;

    Ok(())
}

async fn print_lua_help(lua_cmd: &LuaBinary) -> Result<()> {
    let lua_cmd_path: PathBuf = lua_cmd.clone().try_into()?;
    let output = match Command::new(lua_cmd_path.to_string_lossy().to_string())
        // HACK: This fails with exit 1, because lua doesn't actually have a help flag (╯°□°)╯︵ ┻━┻
        .arg("-h")
        .output()
        .await
    {
        Ok(output) => Ok(output),
        Err(err) => Err(eyre!("Failed to run {}: {}", lua_cmd, err)),
    }?;
    let lua_help = String::from_utf8_lossy(&output.stderr)
        .lines()
        .skip(2)
        .map(|line| format!("  {line}"))
        .collect_vec()
        .join("\n");
    print!(
        "
Usage: lx lua -- [LUA_OPTIONS] [SCRIPT [ARGS]]...

Arguments:
  [LUA_OPTIONS]...
{lua_help}

Options:
  --lua       Path to the Lua interpreter to use
  -h, --help  Print help

Build options (if running a repl for a project):
  --test      Prepend test dependencies to the LUA_PATH and LUA_CPATH
  --build     Prepend build dependencies to the LUA_PATH and LUA_CPATH
  --no-lock   Ignore the project's lockfile and don't create one
  --only-deps Build only the dependencies
",
    );
    Ok(())
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use lux_lib::config::ConfigBuilder;
    use serial_test::serial;

    use super::*;

    #[serial]
    #[tokio::test]
    async fn test_run_lua() {
        let args = RunLua {
            args: Some(vec!["-v".into()]),
            ..RunLua::default()
        };
        let temp: PathBuf = assert_fs::TempDir::new().unwrap().path().into();
        let data_dir: PathBuf = assert_fs::TempDir::new().unwrap().path().into();
        let cwd = &std::env::current_dir().unwrap();
        tokio::fs::create_dir_all(&temp).await.unwrap();
        std::env::set_current_dir(&temp).unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .user_tree(Some(temp.clone()))
            .data_dir(Some(data_dir))
            .build()
            .unwrap();
        run_lua(args, config).await.unwrap();
        std::env::set_current_dir(cwd).unwrap();
    }
}
