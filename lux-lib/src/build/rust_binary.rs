use crate::build::backend::{BuildBackend, BuildInfo, RunBuildArgs};
use crate::build::utils::{self, InstallBinaryError};
use crate::config::build;
use crate::fs;
use crate::lua_rockspec::RustBinaryBuildSpec;
use crate::tree::InstallTree;
use miette::Diagnostic;
use std::io;
use std::process::ExitStatus;
use thiserror::Error;

use tracing::Instrument;

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum RustBinaryError {
    #[error("`cargo install` failed.\nstatus: {status}\nstdout: {stdout}\nstderr: {stderr}")]
    CargoInstall {
        status: ExitStatus,
        stdout: String,
        stderr: String,
    },
    #[error("failed to run `cargo install`")]
    RustBuild { source: io::Error },
    #[error("failed to install binary '{file_name}'")]
    InstallBinary {
        file_name: String,
        #[diagnostic_source]
        source: InstallBinaryError,
    },
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::FsError),
}

impl BuildBackend for RustBinaryBuildSpec {
    type Err = RustBinaryError;

    #[tracing::instrument(name = "rust_binary::run", skip_all, level = "debug")]
    async fn run<T>(self, args: RunBuildArgs<'_, T>) -> Result<BuildInfo, Self::Err>
    where
        T: InstallTree,
    {
        let config = args.config;
        let build_dir = args.build_dir;

        let mut install_args = vec!["install", "--root", ".", "--bins"];
        if config.build_profile() == build::Profile::Dev {
            install_args.push("--debug");
        }
        let features = self.features.join(",");
        if !features.is_empty() {
            install_args.push("--features");
            install_args.push(&features);
        }
        install_args.push(&self.binary);

        if let Some(cargo_vendor_dir) = config
            .vendor_dir()
            .map(|vendor_dir| vendor_dir.join("cargo"))
            .filter(|dir| dir.is_dir())
        {
            utils::prepare_cargo_vendor_config(config, build_dir, &cargo_vendor_dir).await?;
            install_args.push("--offline");
        }

        match config
            .wrapped_command("cargo", install_args)
            .current_dir(build_dir)
            .output()
            .instrument(tracing::info_span!(
                "Compiling rust binary",
                profile = config.build_profile().to_string()
            ))
            .await
        {
            Ok(output) if output.status.success() => utils::trace_command_output(&output),
            Ok(output) => {
                return Err(RustBinaryError::CargoInstall {
                    status: output.status,
                    stdout: String::from_utf8_lossy(&output.stdout).into(),
                    stderr: String::from_utf8_lossy(&output.stderr).into(),
                });
            }
            Err(source) => return Err(RustBinaryError::RustBuild { source }),
        }

        let mut binaries = Vec::new();
        for bin_script in utils::detect_binaries(build_dir) {
            if let Some(target) = bin_script.file_name() {
                let file_name = target.to_string_lossy().to_string();
                let installed_bin_script = utils::install_binary(
                    &bin_script,
                    &file_name,
                    args.tree,
                    args.lua,
                    args.deploy,
                    config,
                )
                .await
                .map_err(|err| RustBinaryError::InstallBinary {
                    file_name: file_name.to_string(),
                    source: err,
                })?;
                if let Some(bin_script_file_name) = installed_bin_script.file_name() {
                    binaries.push(bin_script_file_name.into());
                }
            }
        }

        Ok(BuildInfo { binaries })
    }
}
