use crate::build::backend::{BuildBackend, BuildInfo, RunBuildArgs};
use crate::build::utils::{self, InstallBinaryError};
use crate::config::{build, Config};
use crate::fs;
use crate::lua_rockspec::RustBinaryBuildSpec;
use crate::tree::InstallTree;
use miette::Diagnostic;
use path_slash::PathBufExt;
use std::io;
use std::path::{Path, PathBuf};
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
    #[error("failed to run `cargo`")]
    #[diagnostic(help("ensure cargo is installed"))]
    RustBuild { source: io::Error },
    #[error("`cargo metadata` failed.\nstatus: {status}\nstdout: {stdout}\nstderr: {stderr}")]
    CargoMetadata {
        status: ExitStatus,
        stdout: String,
        stderr: String,
    },
    #[error("failed to parse `cargo metadata` output")]
    CargoMetadataParse(#[from] serde_json::Error),
    #[error("could not locate cargo package '{package}' in the source")]
    CargoPackageNotFound { package: String },
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
        let cargo_vendor_dir = config
            .vendor_dir()
            .map(|vendor_dir| vendor_dir.join("cargo"))
            .filter(|dir| dir.is_dir());

        let crate_dir = if let Some(package) = &self.package {
            find_cargo_package_dir(config, build_dir, package)
                .await?
                .to_slash_lossy()
                .to_string()
        } else {
            format!("{}", build_dir.display())
        };

        install_args.push("--path");
        install_args.push(&crate_dir);

        if let Some(cargo_vendor_dir) = &cargo_vendor_dir {
            install_args.push("--offline");
            utils::prepare_cargo_vendor_config(config, build_dir, cargo_vendor_dir).await?;
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

async fn find_cargo_package_dir(
    config: &Config,
    source_root: &Path,
    package: &str,
) -> Result<PathBuf, RustBinaryError> {
    let output = config
        .wrapped_command("cargo", ["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(source_root)
        .output()
        .await
        .map_err(|source| RustBinaryError::RustBuild { source })?;
    if !output.status.success() {
        return Err(RustBinaryError::CargoMetadata {
            status: output.status,
            stdout: String::from_utf8_lossy(&output.stdout).into(),
            stderr: String::from_utf8_lossy(&output.stderr).into(),
        });
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    metadata
        .get("packages")
        .and_then(|packages| packages.as_array())
        .and_then(|packages| {
            packages
                .iter()
                .find(|pkg| pkg.get("name").and_then(|name| name.as_str()) == Some(package))
        })
        .and_then(|pkg| pkg.get("manifest_path").and_then(|path| path.as_str()))
        .and_then(|path| Path::new(path).parent())
        .map(Path::to_path_buf)
        .ok_or_else(|| RustBinaryError::CargoPackageNotFound {
            package: package.to_string(),
        })
}
