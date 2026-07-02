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

    /// Code to use for two-factor authentication
    #[arg(short, long, visible_short_alias = 'c')]
    tfa_code: Option<String>,
}

#[cfg(feature = "gpgme")]
pub async fn upload(data: Upload, config: Config) -> Result<()> {
    let workspace = Workspace::current_or_err()?;

    let progress = MultiProgress::new(&config);
    let bar = progress.map(MultiProgress::new_bar);
    let package_db = RemotePackageDB::from_config(&config, &bar).await?;
    if let Some(package) = data.package {
        let project = workspace.select_member(&package)?;
        ProjectUpload::new()
            .project(project)
            .config(&config)
            .sign_protocol(data.sign_protocol.clone())
            .maybe_tfa_code(data.tfa_code)
            .progress(&bar)
            .package_db(&package_db)
            .upload_to_luarocks()
            .await?;
    } else {
        for project in workspace.members() {
            ProjectUpload::new()
                .project(project)
                .config(&config)
                .sign_protocol(data.sign_protocol.clone())
                .maybe_tfa_code(data.tfa_code.clone())
                .progress(&bar)
                .package_db(&package_db)
                .upload_to_luarocks()
                .await?;
        }
    }

    Ok(())
}

#[cfg(not(feature = "gpgme"))]
pub async fn upload(data: Upload, config: Config) -> Result<()> {
    let workspace = Workspace::current_or_err()?;
    let progress = MultiProgress::new(&config);
    let bar = progress.map(MultiProgress::new_bar);
    let package_db = RemotePackageDB::from_config(&config, &bar).await?;

    if let Some(package) = data.package {
        let project = workspace.select_member(&package)?;
        ProjectUpload::new()
            .project(project)
            .config(&config)
            .maybe_tfa_code(data.tfa_code)
            .progress(&bar)
            .package_db(&package_db)
            .upload_to_luarocks()
            .await?;
    } else {
        for project in workspace.members() {
            ProjectUpload::new()
                .project(project)
                .config(&config)
                .maybe_tfa_code(data.tfa_code.clone())
                .progress(&bar)
                .package_db(&package_db)
                .upload_to_luarocks()
                .await?;
        }
    }

    Ok(())
}
