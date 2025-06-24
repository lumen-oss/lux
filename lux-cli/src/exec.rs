use std::env;

use clap::Args;
use eyre::Result;
use lux_lib::{
    config::{Config, LuaVersion},
    operations::{self, install_command},
    path::Paths,
    project::Project,
};
use which::which;

use crate::build::Build;

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
    let project = Project::current()?;
    let tree = match &project {
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
    if which(&run.command).is_err() {
        match project {
            Some(_) => super::build::build(Build::default(), config.clone()).await?,
            None => install_command(&run.command, &config).await?,
        }
    };
    operations::Exec::new(&run.command, project.as_ref(), &config)
        .args(run.args.unwrap_or_default())
        .disable_loader(run.no_loader)
        .exec()
        .await?;
    Ok(())
}
