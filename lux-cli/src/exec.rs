use std::env;

use clap::Args;
use eyre::Result;
use lux_lib::{
    config::Config, lua_version::LuaVersion, operations, path::Paths, workspace::Workspace,
};

#[derive(Args)]
pub struct Exec {
    /// The command to run.
    command: String,

    /// Arguments to pass to the program.
    args: Option<Vec<String>>,

    /// Do not add `require('lux').loader()` to `LUA_INIT`.
    /// If a rock has conflicting transitive dependencies,
    /// disabling the Lux loader may result in the wrong modules being loaded.
    #[clap(default_value_t = false)]
    #[arg(long)]
    no_loader: bool,
}

pub async fn exec(run: Exec, config: Config) -> Result<()> {
    let workspace = Workspace::current()?;
    let tree = match &workspace {
        Some(project) => project.tree(&config)?,
        None => {
            let lua_version = LuaVersion::from(&config)?.clone();
            config.user_tree(lua_version)?
        }
    };

    let paths = Paths::new(&tree)?;
    unsafe {
        // safe as long as this is single-threaded
        env::set_var("PATH", paths.path_prepended().joined());
    }
    operations::Exec::new(&run.command, workspace.as_ref(), &config)
        .args(run.args.unwrap_or_default())
        .disable_loader(run.no_loader)
        .exec()
        .await?;
    Ok(())
}
