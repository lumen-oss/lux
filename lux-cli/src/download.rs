use clap::Args;
use lux_lib::{config::Config, operations, package::PackageReq};

use miette::Result;

#[derive(Args)]
pub struct Download {
    package_req: PackageReq,
}

pub async fn download(dl_data: Download, config: Config) -> Result<()> {
    let _rock = operations::Download::new(&dl_data.package_req, &config)
        .download_src_rock_to_file(None)
        .await?;
    Ok(())
}
