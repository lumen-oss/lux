use clap::Args;
use eyre::Result;
use lux_lib::{
    config::Config, package::PackageName, progress::MultiProgress,
    remote_package_db::RemotePackageDB, upload::ProjectUpload, workspace::Workspace,
};

#[cfg(feature = "gpgme")]
use lux_lib::upload::SignatureProtocol;

#[derive(Args)]
pub struct Upload {
    /// The protocol to use when signing upload artefacts
    #[cfg(feature = "gpgme")]
    #[arg(long, default_value_t)]
    sign_protocol: SignatureProtocol,

    /// Package to upload.
    #[arg(short, long, visible_short_alias = 'p')]
    package: Option<PackageName>,
}

#[cfg(feature = "gpgme")]
pub async fn upload(data: Upload, config: Config) -> Result<()> {
    let workspace = Workspace::current_or_err()?;

    let progress = MultiProgress::new(&config);
    let bar = progress.map(MultiProgress::new_bar);
    let package_db = RemotePackageDB::from_config(&config, &bar).await?;
    for project in workspace.try_members(&data.package)? {
        ProjectUpload::new()
            .project(project)
            .config(&config)
            .sign_protocol(data.sign_protocol.clone())
            .progress(&bar)
            .package_db(&package_db)
            .upload_to_luarocks()
            .await?;
    }

    Ok(())
}

#[cfg(not(feature = "gpgme"))]
pub async fn upload(data: Upload, config: Config) -> Result<()> {
    let workspace = Workspace::current_or_err()?;
    let progress = MultiProgress::new(&config);
    let bar = progress.map(MultiProgress::new_bar);
    let package_db = RemotePackageDB::from_config(&config, &bar).await?;

    for project in workspace.try_members(&data.package)? {
        ProjectUpload::new()
            .project(project)
            .config(&config)
            .progress(&bar)
            .package_db(&package_db)
            .upload_to_luarocks()
            .await?;
    }

    Ok(())
}
