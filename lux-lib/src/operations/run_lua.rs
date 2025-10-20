//! Run the `lua` binary with some given arguments.
//!
//! The interfaces exposed here ensure that the correct version of Lua is being used.

use bon::Builder;

use crate::{
    config::Config,
    path::{BinPath, PackagePath},
};

use std::{
    io,
    path::{Path, PathBuf},
    process::Stdio,
};

use thiserror::Error;
use tokio::process::Command;

use crate::{
    lua_installation::{LuaBinary, LuaBinaryError},
    path::{Paths, PathsError},
    tree::Tree,
    tree::TreeError,
};

#[derive(Error, Debug)]
pub enum RunLuaError {
    #[error("error running lua: {0}")]
    LuaBinary(#[from] LuaBinaryError),
    #[error("failed to run {lua_cmd}: {source}")]
    LuaCommandFailed {
        lua_cmd: String,
        #[source]
        source: io::Error,
    },
    #[error("{lua_cmd} exited with non-zero exit code: {}", exit_code.map(|code| code.to_string()).unwrap_or("unknown".into()))]
    LuaCommandNonZeroExitCode {
        lua_cmd: String,
        exit_code: Option<i32>,
    },
    #[error(transparent)]
    Paths(#[from] PathsError),

    #[error(transparent)]
    Tree(#[from] TreeError),
}

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct RunLua<'a> {
    root: &'a Path,
    tree: &'a Tree,
    config: &'a Config,
    lua_cmd: LuaBinary,
    args: &'a Vec<String>,
    prepend_test_paths: Option<bool>,
    prepend_build_paths: Option<bool>,
    disable_loader: Option<bool>,
    lua_init: Option<String>,
    welcome_message: Option<String>,
}

impl<State> RunLuaBuilder<'_, State>
where
    State: run_lua_builder::State + run_lua_builder::IsComplete,
{
    pub async fn run_lua(self) -> Result<(), RunLuaError> {
        let args = self._build();
        let mut paths = Paths::new(args.tree)?;

        if args.prepend_test_paths.unwrap_or(false) {
            let test_tree_path = args.tree.test_tree(args.config)?;

            let test_path = Paths::new(&test_tree_path)?;

            paths.prepend(&test_path);
        }

        if args.prepend_build_paths.unwrap_or(false) {
            let build_tree_path = args.tree.build_tree(args.config)?;

            let build_path = Paths::new(&build_tree_path)?;

            paths.prepend(&build_path);
        }

        let lua_cmd: PathBuf = args.lua_cmd.try_into()?;

        let is_lux_lua_available = detect_lux_lua(&lua_cmd, &paths).await;

        let loader_init = if args.disable_loader.unwrap_or(false) {
            "".to_string()
        } else if !is_lux_lua_available && args.tree.version().lux_lib_dir().is_none() {
            eprintln!(
                "⚠️ WARNING: lux-lua library not found.
Cannot use the `lux.loader`.
To suppress this warning, set the `--no-loader` option.
                "
            );
            "".to_string()
        } else {
            paths.init()
        };
        let lua_init = format!(
            r#"print([==[{}]==])
{}
{}
        "#,
            args.welcome_message.unwrap_or_default(),
            args.lua_init.unwrap_or_default(),
            loader_init
        );

        let status = match Command::new(&lua_cmd)
            .current_dir(args.root)
            .args(args.args)
            .env("PATH", paths.path_prepended().joined())
            .env("LUA_PATH", paths.package_path().joined())
            .env("LUA_CPATH", paths.package_cpath().joined())
            .env("LUA_INIT", lua_init)
            .status()
            .await
        {
            Ok(status) => Ok(status),
            Err(err) => Err(RunLuaError::LuaCommandFailed {
                lua_cmd: lua_cmd.to_string_lossy().to_string(),
                source: err,
            }),
        }?;
        if status.success() {
            Ok(())
        } else {
            Err(RunLuaError::LuaCommandNonZeroExitCode {
                lua_cmd: lua_cmd.to_string_lossy().to_string(),
                exit_code: status.code(),
            })
        }
    }
}

/// Attempts to detect lux-lua by invoking a Lua command
/// in case it's a Lua wrapper, like the one created
/// in nixpkgs using `lua.withPackages (ps: [ps.lux-lua])`.
/// If the command fails for any reason (including not being able to find the 'lux' module),
/// this function evaluates to `false`.
async fn detect_lux_lua(lua_cmd: &Path, paths: &Paths) -> bool {
    detect_lua_module(
        lua_cmd,
        &paths.package_path_prepended(),
        &paths.package_cpath_prepended(),
        &paths.path_prepended(),
        "lux",
    )
    .await
}

async fn detect_lua_module(
    lua_cmd: &Path,
    lua_path: &PackagePath,
    lua_cpath: &PackagePath,
    path: &BinPath,
    module: &str,
) -> bool {
    Command::new(lua_cmd)
        .arg("-e")
        .arg(format!(
            "if pcall(require, '{}') then os.exit(0) else os.exit(1) end",
            module
        ))
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .env("LUA_PATH", lua_path.joined())
        .env("LUA_CPATH", lua_cpath.joined())
        .env("PATH", path.joined())
        .status()
        .await
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use super::*;
    use assert_fs::prelude::{PathChild, PathCreateDir};
    use assert_fs::TempDir;
    use path_slash::PathBufExt;
    use which::which;

    #[tokio::test]
    async fn test_detect_lua_module() {
        let temp_dir = TempDir::new().unwrap();
        let lua_dir = temp_dir.child("lua");
        lua_dir.create_dir_all().unwrap();
        let lux_file = lua_dir.child("lux.lua").to_path_buf();
        let lux_path_expr = lua_dir.child("?.lua").to_path_buf();
        tokio::fs::write(&lux_file, "return true").await.unwrap();
        let package_path =
            PackagePath::from_str(lux_path_expr.to_slash_lossy().to_string().as_str()).unwrap();
        let package_cpath = PackagePath::default();
        let path = BinPath::default();
        let lua_cmd = which("lua")
            .ok()
            .or(which("luajit").ok())
            .expect("lua not found");
        let result = detect_lua_module(&lua_cmd, &package_path, &package_cpath, &path, "lux").await;
        assert!(result, "detects module on the LUA_PATH");
        let result = detect_lua_module(
            &lua_cmd,
            &package_path,
            &package_cpath,
            &path,
            "lhflasdlkas",
        )
        .await;
        assert!(!result, "does not detect non-existing module");
    }
}
