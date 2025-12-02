use clap::Args;
use eyre::{OptionExt, Result};
use lux_lib::{
    config::Config, progress::MultiProgress, project::Project, remote_package_db::RemotePackageDB,
    upload::ProjectUpload,
};

#[cfg(feature = "gpgme")]
use lux_lib::upload::SignatureProtocol;

#[derive(Args)]
pub struct Upload {
    /// The protocol to use when signing upload artefacts
    #[cfg(feature = "gpgme")]
    #[arg(long, default_value_t)]
    sign_protocol: SignatureProtocol,
}

#[cfg(feature = "gpgme")]
pub async fn upload(data: Upload, config: Config) -> Result<()> {
    let project = Project::current()?.ok_or_eyre("No project found")?;

    let progress = MultiProgress::new(&config);
    let bar = progress.map(MultiProgress::new_bar);
    let package_db = RemotePackageDB::from_config(&config, &bar).await?;
    ProjectUpload::new()
        .project(project)
        .config(&config)
        .sign_protocol(data.sign_protocol)
        .progress(&bar)
        .package_db(&package_db)
        .upload_to_luarocks()
        .await?;

    Ok(())
}

#[cfg(not(feature = "gpgme"))]
pub async fn upload(_data: Upload, config: Config) -> Result<()> {
    let project = Project::current()?.ok_or_eyre("No project found")?;
    let progress = MultiProgress::new(&config);
    let bar = progress.map(MultiProgress::new_bar);
    let package_db = RemotePackageDB::from_config(&config, &bar).await?;

    ProjectUpload::new()
        .project(project)
        .config(&config)
        .progress(&bar)
        .package_db(&package_db)
        .upload_to_luarocks()
        .await?;

    Ok(())
}
