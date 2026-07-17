use std::path::PathBuf;

use lux_lib::{config::Config, operations::Download, progress::MultiProgress, rockspec::Rockspec};
use miette::Result;

use crate::unpack::UnpackRemote;

pub async fn fetch_remote(data: UnpackRemote, config: Config) -> Result<()> {
    let package_req = data.package_req;
    let progress = MultiProgress::new(&config);
    let bar = progress.map(MultiProgress::new_bar);

    let rockspec = Download::new(&package_req, &config, &bar)
        .download_rockspec()
        .await?
        .rockspec;

    let destination = data.path.unwrap_or_else(|| {
        PathBuf::from(format!("{}-{}", &rockspec.package(), &rockspec.version()))
    });
    lux_lib::operations::FetchSrc::new(destination.clone().as_path(), &rockspec, &config, &bar)
        .fetch()
        .await?;

    let rock_source = rockspec.source().current_platform();
    let build_dir = rock_source
        .unpack_dir
        .as_ref()
        .map(|path| destination.join(path))
        .unwrap_or_else(|| destination);

    bar.map(|b| {
        b.finish_with_message(
            format!(
                "
You may now enter the following directory:
{}
and type `lux build` to build.
    ",
                build_dir.as_path().display()
            ),
            lux_lib::logging::LogLevel::Info,
        )
    });

    Ok(())
}
