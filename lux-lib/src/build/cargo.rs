use std::{fs::OpenOptions, io::Write, path::Path};

use crate::{config::Config, fs};

/// Points cargo at the vendored sources under [`Config::vendor_dir`]/cargo by
/// writing a `.cargo/config.toml` in the build directory, if such a directory exists.
#[tracing::instrument(level = "trace")]
pub(crate) async fn prepare_vendor_config(
    config: &Config,
    build_dir: &Path,
    cargo_vendor_dir: &Path,
) -> Result<(), fs::Error> {
    let cargo_config_dir = build_dir.join(".cargo");
    fs::tokio::create_dir_all(&cargo_config_dir).await?;
    let cargo_config_path = cargo_config_dir.join("config.toml");
    if cargo_config_path.exists() {
        fs::make_writable(&cargo_config_path).await?;
    }
    let mut target_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&cargo_config_path)
        .map_err(|source| fs::Error::FileOpen {
            path: cargo_config_path.to_path_buf(),
            source,
        })?;
    write!(
        &mut target_file,
        "[source.crates-io]\nreplace-with = \"vendored-sources\"\n\n[source.vendored-sources]\ndirectory = \"{}\"\n",
        cargo_vendor_dir.display()
    )
    .map_err(|source| fs::Error::Write {
        path: cargo_config_path.to_path_buf(),
        source,
    })?;
    Ok(())
}
