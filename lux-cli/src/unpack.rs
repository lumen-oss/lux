use std::{fs::File, io::Cursor, path::PathBuf};

use clap::Args;
use lux_lib::{config::Config, operations, package::PackageReq};

use miette::{IntoDiagnostic, Result};

#[derive(Args)]
pub struct Unpack {
    /// A path to a .src.rock file. Usually obtained via `lux download`.
    path: PathBuf,
    /// Where to unpack the rock.
    destination: Option<PathBuf>,
}

#[derive(Args)]
pub struct UnpackRemote {
    pub package_req: PackageReq,
    /// The directory to unpack to
    pub destination: Option<PathBuf>,
}

pub async fn unpack(data: Unpack, _config: Config) -> Result<()> {
    let destination = data.destination.unwrap_or_else(|| {
        PathBuf::from(data.path.to_string_lossy().trim_end_matches(".src.rock"))
    });
    let src_file = File::open(data.path).into_diagnostic()?;

    let unpack_path = lux_lib::operations::unpack_src_rock(src_file, destination).await?;
    tracing::info!("unpacked rock to: {}", unpack_path.display());

    Ok(())
}

pub async fn unpack_remote(data: UnpackRemote, config: Config) -> Result<()> {
    let package_req = data.package_req;
    let rock = operations::Download::new(&package_req, &config)
        .search_and_download_src_rock()
        .await?;
    let cursor = Cursor::new(rock.bytes);

    let destination = data
        .destination
        .unwrap_or_else(|| PathBuf::from(format!("{}-{}", &rock.name, &rock.version)));

    let unpack_path = lux_lib::operations::unpack_src_rock(cursor, destination).await?;
    tracing::info!("unpacked rock to: {}", unpack_path.display());

    Ok(())
}
