use super::utils::c_dylib_extension;
use crate::build::backend::{BuildBackend, BuildInfo, RunBuildArgs};
use crate::config::LuaVersionUnset;
use crate::progress::{Progress, ProgressBar};
use crate::{config::LuaVersion, lua_rockspec::RustMluaBuildSpec, tree::RockLayout};
use itertools::Itertools;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::{fs, io};
use thiserror::Error;
use tokio::process::Command;

#[derive(Error, Debug)]
pub enum RustError {
    #[error("`cargo build` failed.\nstatus: {status}\nstdout: {stdout}\nstderr: {stderr}")]
    CargoBuild {
        status: ExitStatus,
        stdout: String,
        stderr: String,
    },
    #[error("failed to run `cargo build`: {0}")]
    RustBuild(io::Error),
    #[error("unable to create directory {0}:\n{1}")]
    CreateDir(String, io::Error),
    #[error(transparent)]
    InstallRustLib(#[from] InstallRustLibError),
    #[error(transparent)]
    InstallLuaLib(#[from] InstallLuaLibError),
    #[error(transparent)]
    LuaVersionUnset(#[from] LuaVersionUnset),
}

#[derive(Error, Debug)]
#[error("error installing rust library {lib}:\n{cause}")]
pub struct InstallRustLibError {
    lib: String,
    cause: io::Error,
}

#[derive(Error, Debug)]
#[error("error installing Lua library {lib}:\n{cause}")]
pub struct InstallLuaLibError {
    lib: String,
    cause: io::Error,
}

impl BuildBackend for RustMluaBuildSpec {
    type Err = RustError;

    async fn run(self, args: RunBuildArgs<'_>) -> Result<BuildInfo, Self::Err> {
        let output_paths = args.output_paths;
        let config = args.config;
        let build_dir = args.build_dir;
        let progress = args.progress;
        let lua_version = LuaVersion::from(config)?;
        let lua_feature = match lua_version {
            LuaVersion::Lua51 => "lua51",
            LuaVersion::Lua52 => "lua52",
            LuaVersion::Lua53 => "lua53",
            LuaVersion::Lua54 => "lua54",
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
        progress.map(|bar| bar.set_message("🦀 Building (rust-mlua)..."));
        match Command::new("cargo")
            .current_dir(build_dir)
            .args(build_args)
            .output()
            .await
        {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                return Err(RustError::CargoBuild {
                    status: output.status,
                    stdout: String::from_utf8_lossy(&output.stdout).into(),
                    stderr: String::from_utf8_lossy(&output.stderr).into(),
                });
            }
            Err(err) => return Err(RustError::RustBuild(err)),
        }
        fs::create_dir_all(&output_paths.lib).map_err(|err| {
            RustError::CreateDir(output_paths.lib.to_string_lossy().to_string(), err)
        })?;
        if let Err(err) =
            install_rust_libs(self.modules, &self.target_path, build_dir, output_paths)
        {
            cleanup(output_paths, progress).await;
            return Err(err.into());
        }
        fs::create_dir_all(&output_paths.src).map_err(|err| {
            RustError::CreateDir(output_paths.src.to_string_lossy().to_string(), err)
        })?;
        if let Err(err) = install_lua_libs(self.include, build_dir, output_paths) {
            cleanup(output_paths, progress).await;
            return Err(err.into());
        }
        Ok(BuildInfo::default())
    }
}

fn install_rust_libs(
    modules: HashMap<String, PathBuf>,
    target_path: &Path,
    build_dir: &Path,
    output_paths: &RockLayout,
) -> Result<(), InstallRustLibError> {
    for (module, rust_lib) in modules {
        let src = build_dir.join(target_path).join("release").join(rust_lib);
        let mut dst: PathBuf = output_paths.lib.join(module);
        dst.set_extension(c_dylib_extension());
        fs::copy(&src, &dst).map_err(|err| InstallRustLibError {
            lib: src.to_string_lossy().to_string(),
            cause: err,
        })?;
    }
    Ok(())
}

fn install_lua_libs(
    include: HashMap<PathBuf, PathBuf>,
    build_dir: &Path,
    output_paths: &RockLayout,
) -> Result<(), InstallLuaLibError> {
    for (from, to) in include {
        let src = build_dir.join(from);
        let dst = output_paths.src.join(to);
        fs::copy(&src, &dst).map_err(|err| InstallLuaLibError {
            lib: src.to_string_lossy().to_string(),
            cause: err,
        })?;
    }
    Ok(())
}

async fn cleanup(output_paths: &RockLayout, progress: &Progress<ProgressBar>) -> () {
    let root_dir = &output_paths.rock_path;

    progress.map(|p| p.set_message(format!("🗑️ Cleaning up {}", root_dir.display())));

    match tokio::fs::remove_dir_all(root_dir).await {
        Ok(_) => (),
        Err(err) => {
            progress
                .map(|p| p.println(format!("Error cleaning up {}: {}", root_dir.display(), err)));
        }
    };
}
