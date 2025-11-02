use clap::Args;
use eyre::{eyre, Result, WrapErr};
use lux_lib::{
    config::Config, lua_installation::LuaInstallation, path::Paths, progress::MultiProgress,
};
use which::which;

use std::{env, path::PathBuf};
use tokio::process::Command;

use super::utils::project::current_project_or_user_tree;

#[derive(Args)]
pub struct Shell {
    /// Add test dependencies to the shell's paths,{n}
    /// in addition to the regular dependencies.
    #[arg(long)]
    test: bool,

    /// Add *only* build dependencies to the shell's paths.
    #[arg(long, conflicts_with = "test")]
    build: bool,

    /// Disable the Lux loader.{n}
    /// If a rock has conflicting transitive dependencies,{n}
    /// disabling the Lux loader may result in the wrong modules being loaded.{n}
    #[arg(long)]
    no_loader: bool,
}

pub async fn shell(data: Shell, config: Config) -> Result<()> {
    if env::var("LUX_SHELL").is_ok_and(|lx_shell_var| lx_shell_var == "1") {
        return Err(eyre!("Already in a Lux shell."));
    }

    let tree = current_project_or_user_tree(&config).unwrap();

    let path = if data.build {
        let build_tree_path = tree.build_tree(&config)?;
        Paths::new(&build_tree_path)?
    } else {
        let mut path = Paths::new(&tree)?;
        if data.test {
            let test_tree_path = tree.test_tree(&config)?;
            let test_path = Paths::new(&test_tree_path)?;
            path.prepend(&test_path);
        }
        path
    };

    let shell: PathBuf = match env::var("SHELL") {
        Ok(val) => PathBuf::from(val),
        Err(_) => {
            #[cfg(target_os = "linux")]
            let fallback = which("bash").wrap_err("Cannot find `bash` on your system!")?;

            #[cfg(target_os = "windows")]
            let fallback = which("cmd.exe").wrap_err("Cannot find `cmd.exe` on your system!")?;

            #[cfg(target_os = "macos")]
            let fallback = which("zsh").wrap_err("Cannot find `zsh` on your system!")?;

            fallback
        }
    };

    let lua_path = path.package_path_prepended();
    let lua_cpath = path.package_cpath_prepended();

    let lua_init = if data.no_loader {
        None
    } else if tree.version().lux_lib_dir().is_none() {
        eprintln!(
            "⚠️ WARNING: lux-lua library not found.
    Cannot use the `lux.loader`.
    To suppress this warning, set the `--no-loader` option.
                    "
        );
        None
    } else {
        Some(path.init())
    };

    let lua_version = tree.version();

    let mut bin_path = path.path_prepended();

    let progress = MultiProgress::new_arc(&config);
    let bar = progress.map(|progress| progress.new_bar());
    let lua = LuaInstallation::new(lua_version, &config, &bar).await?;
    bar.map(|bar| bar.finish_and_clear());
    if let Some(lua_bin_path) = lua.bin().as_ref().and_then(|lua_bin| lua_bin.parent()) {
        bin_path.add_path(lua_bin_path.to_path_buf());
    }

    let _ = Command::new(&shell)
        .env("PATH", bin_path.joined())
        .env("LUA_PATH", lua_path.joined())
        .env("LUA_CPATH", lua_cpath.joined())
        .env("LUA_INIT", lua_init.unwrap_or_default())
        .env("LUX_SHELL", "1")
        .spawn()?
        .wait()
        .await?;

    Ok(())
}
