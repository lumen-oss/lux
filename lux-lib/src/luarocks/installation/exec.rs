use std::path::Path;
use std::process::ExitStatus;

use miette::Diagnostic;
use path_slash::{PathBufExt, PathExt};
use thiserror::Error;

use super::{Installation, LUAROCKS_EXE};
use crate::{
    build, fs, lua,
    path::{self, Paths},
    tree::InstallTree,
    variables,
};

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaVersion(#[from] lua::version::Error),
    #[error(
        "could not substitute '$(LUA_LIBDIR)' and '$(LUA_INCDIR)' variables in the luarocks config template"
    )]
    #[diagnostic(forward(0))]
    VariableSubstitutionInConfig(#[from] variables::Error),
    #[error("failed to run luarocks")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error("error setting up luarocks paths")]
    #[diagnostic(forward(0))]
    Paths(#[from] path::Error),
    #[error("luarocks binary not found at '{0}'")]
    #[diagnostic(help(
        "Lux successfully installed luarocks, but couldn't find the binary to execute"
    ))]
    LuarocksBinNotFound(std::path::PathBuf),
    #[error(
        r#"executing luarocks compatibility layer failed.
status: {status}
stdout:
{stdout}
stderr:
{stderr}
"#
    )]
    #[diagnostic(help("see the build output for details"))]
    CommandFailure {
        status: ExitStatus,
        stdout: String,
        stderr: String,
    },
    #[error("luau does not support LuaRocks build backends")]
    LuauUnsupported,
}

impl Installation {
    #[tracing::instrument(level = "trace", skip(self))]
    pub async fn make(
        self,
        rockspec_path: &Path,
        build_dir: &Path,
        dest_dir: &Path,
        lua: &lua::Installation,
    ) -> Result<(), Error> {
        fs::sync::create_dir_all(dest_dir)?;
        let dest_dir_str = dest_dir.to_slash_lossy().to_string();
        let rockspec_path_str = rockspec_path.to_slash_lossy().to_string();
        let args = vec![
            "make",
            "--deps-mode",
            "none",
            "--tree",
            &dest_dir_str,
            &rockspec_path_str,
        ];
        self.exec(args, build_dir, lua).await
    }

    async fn exec(self, args: Vec<&str>, cwd: &Path, lua: &lua::Installation) -> Result<(), Error> {
        let luarocks_paths = Paths::new(&self.tree)?;
        // Ensure a pure environment so we can do parallel builds
        let temp_dir = fs::tempfile::tempdir()?;
        let lua_version_str = match lua.version {
            lua::Version::Lua51 | lua::Version::LuaJIT => "5.1",
            lua::Version::Lua52 | lua::Version::LuaJIT52 => "5.2",
            lua::Version::Lua53 => "5.3",
            lua::Version::Lua54 => "5.4",
            lua::Version::Lua55 => "5.5",
            lua::Version::Luau => return Err(Error::LuauUnsupported),
        };
        let luarocks_config_content = format!(
            r#"
lua_version = "{0}"
variables = {{
    LUA_LIBDIR = "$(LUA_LIBDIR)",
    LUA_INCDIR = "$(LUA_INCDIR)",
    LUA_VERSION = "{1}",
    MAKE = "{2}",
}}
"#,
            lua_version_str,
            lua::Version::from(&self.config)?,
            self.config.make_cmd()
        );
        let luarocks_config_content =
            variables::substitute(&[lua, &self.config], &luarocks_config_content)?;
        let luarocks_config = temp_dir.path().join("luarocks-config.lua");
        fs::sync::write(luarocks_config.clone(), luarocks_config_content)?;
        let luarocks_bin = self.tree.bin().join(LUAROCKS_EXE);
        if !luarocks_bin.is_file() {
            return Err(Error::LuarocksBinNotFound(luarocks_bin));
        }
        let output = self
            .config
            .wrapped_command(luarocks_bin, args)
            .current_dir(cwd)
            .env("PATH", luarocks_paths.path_prepended().joined())
            .env("LUA_PATH", luarocks_paths.package_path().joined())
            .env("LUA_CPATH", luarocks_paths.package_cpath().joined())
            .env("HOME", temp_dir.path().to_slash_lossy().to_string())
            .env(
                "LUAROCKS_CONFIG",
                luarocks_config.to_slash_lossy().to_string(),
            )
            .output()
            .await?;
        if output.status.success() {
            build::command::trace_output(&output);
            Ok(())
        } else {
            Err(Error::CommandFailure {
                status: output.status,
                stdout: String::from_utf8_lossy(&output.stdout).into(),
                stderr: String::from_utf8_lossy(&output.stderr).into(),
            })
        }
    }
}
