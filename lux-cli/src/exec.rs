use std::env;

use clap::Args;
use lux_lib::{
    config::Config, lua_version::LuaVersion, operations, path::Paths, workspace::Workspace,
};
use miette::Result;

#[derive(Args)]
pub struct Exec {
    /// The command to run, followed by arguments to pass to it.
    #[arg(required = true, trailing_var_arg = true)]
    command: Vec<String>,

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
    let [command, args @ ..] = run.command.as_slice() else {
        unreachable!("command is required");
    };

    operations::Exec::new(command, workspace.as_ref(), &config)
        .args(args.to_vec())
        .disable_loader(run.no_loader)
        .exec()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{Cli, Commands};
    use clap::error::ErrorKind;
    use clap::Parser;

    #[test]
    fn forwards_arguments_after_command() {
        let cli = Cli::try_parse_from(["lx", "exec", "echo", "--help", "-n"]).unwrap();
        let Commands::Exec(exec) = cli.command else {
            unreachable!()
        };
        assert_eq!(exec.command, ["echo", "--help", "-n"]);
    }

    #[test]
    fn parses_exec_flags_before_command() {
        let cli = Cli::try_parse_from(["lx", "exec", "--no-loader", "echo", "-n", "hi"]).unwrap();
        let Commands::Exec(exec) = cli.command else {
            unreachable!()
        };
        assert!(exec.no_loader);
        assert_eq!(exec.command, ["echo", "-n", "hi"]);
    }

    #[test]
    fn exec_help_is_not_forwarded() {
        let err = match Cli::try_parse_from(["lx", "exec", "--help"]) {
            Err(err) => err,
            Ok(_) => unreachable!(),
        };
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }
}
