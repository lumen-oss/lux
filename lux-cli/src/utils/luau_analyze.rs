use std::path::PathBuf;

use lux_lib::{config::Config, lua_installation::LuaInstallation, lua_version::LuaVersion};
use miette::{miette, IntoDiagnostic, Result};
use tokio::process::Command;

#[cfg(windows)]
const ANALYZE_BIN: &str = "luau-analyze.exe";
#[cfg(not(windows))]
const ANALYZE_BIN: &str = "luau-analyze";

/// Runs `luau-analyze` on the given paths, exiting with the analyzer's
/// status code on failure.
///
/// Prefers `luau-analyze` on the PATH, falling back to the binary shipped
/// with the lux-installed luau toolchain (installing it if necessary).
pub(crate) async fn run(
    config: &Config,
    paths: Vec<String>,
    extra_args: Vec<String>,
) -> Result<()> {
    let luau_analyze = if config.variables().contains_key("LUAU") {
        installed_luau_analyze(config).await?
    } else {
        match which::which(ANALYZE_BIN) {
            Ok(path) => path,
            Err(_) => installed_luau_analyze(config).await?,
        }
    };
    let status = Command::new(luau_analyze)
        .args(paths)
        .args(extra_args)
        .status()
        .await
        .into_diagnostic()?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

async fn installed_luau_analyze(config: &Config) -> Result<PathBuf> {
    let lua = LuaInstallation::new(&LuaVersion::Luau, config)
        .await
        .map_err(|err| miette!("{err}"))?;
    let analyze_bin = lua
        .bin()
        .as_ref()
        .and_then(|luau_bin| luau_bin.parent())
        .map(|bin_dir| bin_dir.join(ANALYZE_BIN))
        .filter(|analyze_bin| analyze_bin.is_file());
    analyze_bin.ok_or_else(|| {
        miette!("`{ANALYZE_BIN}` not found on the PATH or alongside the luau runtime")
    })
}
