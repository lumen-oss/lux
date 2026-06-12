use clap::Args;
use eyre::Result;
use lux_lib::config::Config;

#[derive(Args)]
pub struct Bin {}

pub async fn bin(_data: Bin, _config: Config) -> Result<()> {
    unimplemented!("lx dist bin");
}
