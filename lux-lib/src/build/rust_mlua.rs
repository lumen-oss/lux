use super::utils::c_dylib_extension;
use crate::build::backend::{BuildBackend, BuildInfo, RunBuildArgs};
use crate::build::utils;
use crate::fs;
use crate::lua_version::{LuaVersion, LuaVersionUnset};
use crate::tree::InstallTree;
use crate::{lua_rockspec::RustMluaBuildSpec, tree::RockLayout};
use itertools::Itertools;
use miette::Diagnostic;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use thiserror::Error;
use tokio::process::Command;
use tracing::{info_span, Instrument};

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum RustError {
    #[error("`cargo build` failed.\nstatus: {status}\nstdout: {stdout}\nstderr: {stderr}")]
    CargoBuild {
        status: ExitStatus,
        stdout: String,
        stderr: String,
    },
    #[error("failed to run `cargo build`")]
    RustBuild { source: io::Error },
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaVersionUnset(#[from] LuaVersionUnset),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::FsError),
}

impl BuildBackend for RustMluaBuildSpec {
    type Err = RustError;

    #[tracing::instrument(name = "rust_mlua::run", skip_all, level = "debug")]
    async fn run<T>(self, args: RunBuildArgs<'_, T>) -> Result<BuildInfo, Self::Err>
    where
        T: InstallTree,
    {
        let output_paths = args.output_paths;
        let config = args.config;
        let build_dir = args.build_dir;
        let lua_version = LuaVersion::from(config)?;
        let lua_feature = match lua_version {
            LuaVersion::Lua51 => "lua51",
            LuaVersion::Lua52 => "lua52",
            LuaVersion::Lua53 => "lua53",
            LuaVersion::Lua54 => "lua54",
            LuaVersion::Lua55 => "lua55",
            LuaVersion::LuaJIT => "luajit",
            LuaVersion::LuaJIT52 => "luajit",
        };
        let features = self
            .features
            .into_iter()
            .chain(std::iter::once(lua_feature.into()))
            .join(",");
        let target_dir_arg = format!("--target-dir={}", self.target_path.display());
        let mut build_args = vec!["build", "--release", &target_dir_arg];
        if !self.default_features {
            build_args.push("--no-default-features");
        }
        build_args.push("--features");
        build_args.push(&features);
        build_args.extend(self.cargo_extra_args.iter().map(|arg| arg.as_str()));
        {
            let span = info_span!("Compiling rust-mlua module");
            match Command::new("cargo")
                .current_dir(build_dir)
                .args(build_args)
                .output()
                .instrument(span)
                .await
            {
                Ok(output) if output.status.success() => utils::trace_command_output(&output),
                Ok(output) => {
                    return Err(RustError::CargoBuild {
                        status: output.status,
                        stdout: String::from_utf8_lossy(&output.stdout).into(),
                        stderr: String::from_utf8_lossy(&output.stderr).into(),
                    });
                }
                Err(source) => return Err(RustError::RustBuild { source }),
            }
        }
        fs::tokio::create_dir_all(&output_paths.lib).await?;
        if let Err(err) =
            install_rust_libs(self.modules, &self.target_path, build_dir, output_paths).await
        {
            cleanup(output_paths).await;
            return Err(err.into());
        }
        fs::tokio::create_dir_all(&output_paths.src).await?;
        if let Err(err) = install_lua_libs(self.include, build_dir, output_paths).await {
            cleanup(output_paths).await;
            return Err(err.into());
        }
        Ok(BuildInfo::default())
    }
}

#[tracing::instrument(level = "trace")]
async fn install_rust_libs(
    modules: HashMap<String, PathBuf>,
    target_path: &Path,
    build_dir: &Path,
    output_paths: &RockLayout,
) -> Result<(), fs::FsError> {
    for (module, rust_lib) in modules {
        let src = build_dir.join(target_path).join("release").join(rust_lib);
        let mut dst: PathBuf = output_paths.lib.join(module);
        dst.set_extension(c_dylib_extension());
        fs::tokio::copy(&src, &dst).await?;
    }
    Ok(())
}

#[tracing::instrument(level = "trace")]
async fn install_lua_libs(
    include: HashMap<PathBuf, PathBuf>,
    build_dir: &Path,
    output_paths: &RockLayout,
) -> Result<(), fs::FsError> {
    for (from, to) in include {
        let src = build_dir.join(from);
        let dst = output_paths.src.join(to);
        fs::tokio::copy(&src, &dst).await?;
    }
    Ok(())
}

#[tracing::instrument(level = "trace")]
async fn cleanup(output_paths: &RockLayout) -> () {
    let root_dir = &output_paths.rock_path;

    match fs::tokio::remove_dir_all(root_dir).await {
        Ok(_) => (),
        Err(err) => tracing::warn!("failed to clean up {}: {}", root_dir.display(), err),
    };
}
