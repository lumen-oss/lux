use clap::Args;
use eyre::Result;
use lux_lib::config::Config;

#[derive(Args)]
pub struct Tarball {}

pub async fn tarball(_data: Tarball, _config: Config) -> Result<()> {
    unimplemented!("lx dist tarball");
}
