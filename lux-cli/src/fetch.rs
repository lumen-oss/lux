use std::path::PathBuf;

use lux_lib::{config::Config, operations::Download, rockspec::Rockspec};

use miette::Result;

use crate::unpack::UnpackRemote;

pub async fn fetch_remote(data: UnpackRemote, config: Config) -> Result<()> {
    let package_req = data.package_req;

    let rockspec = Download::new(&package_req, &config)
        .download_rockspec()
        .await?
        .rockspec;

    let destination = data.destination.unwrap_or_else(|| {
        PathBuf::from(format!("{}-{}", &rockspec.package(), &rockspec.version()))
    });
    lux_lib::operations::FetchSrc::new(destination.clone().as_path(), &rockspec, &config)
        .fetch()
        .await?;

    Ok(())
}
