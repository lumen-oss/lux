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

    /// Code to use for two-factor authentication (2FA).{n}
    /// It is recommended to enable 2FA for luarocks uploads (see https://luarocks.org/settings/two-factor-auth).{n}
    /// Lux can also generate a TOTP code if you expose your luarocks.org 2FA secret via the 'LUAROCKS_2FA_SECRET' environment variable.{n}
    #[arg(short, long, visible_short_alias = 'c')]
    tfa_code: Option<String>,
}

#[cfg(feature = "gpgme")]
pub async fn upload(data: Upload, config: Config) -> Result<()> {
    let workspace = Workspace::current_or_err()?;

    let progress = MultiProgress::new(&config);
    let bar = progress.map(MultiProgress::new_bar);
    let package_db = RemotePackageDB::from_config(&config, &bar).await?;
    let tfa_code = tfa_code_from_args_or_secret(&data)?;
    if let Some(package) = data.package {
        let project = workspace.select_member(&package)?;
        ProjectUpload::new()
            .project(project)
            .config(&config)
            .sign_protocol(data.sign_protocol.clone())
            .maybe_tfa_code(tfa_code)
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
                .maybe_tfa_code(tfa_code.clone())
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
    let tfa_code = tfa_code_from_args_or_secret(&data)?;
    if let Some(package) = data.package {
        let project = workspace.select_member(&package)?;
        ProjectUpload::new()
            .project(project)
            .config(&config)
            .maybe_tfa_code(tfa_code)
            .progress(&bar)
            .package_db(&package_db)
            .upload_to_luarocks()
            .await?;
    } else {
        for project in workspace.members() {
            ProjectUpload::new()
                .project(project)
                .config(&config)
                .maybe_tfa_code(tfa_code.clone())
                .progress(&bar)
                .package_db(&package_db)
                .upload_to_luarocks()
                .await?;
        }
    }

    Ok(())
}

fn tfa_code_from_args_or_secret(data: &Upload) -> Result<Option<String>> {
    match &data.tfa_code {
        Some(code) => Ok(Some(code.to_owned())),
        None => match std::env::var("LUAROCKS_2FA_SECRET") {
            Ok(secret) => {
                let secret = base32::decode(base32::Alphabet::Crockford, &secret)
                    .ok_or(totp_rs::SecretParseError::ParseBase32)?;
                let totp = totp_rs::TOTP::new(totp_rs::Algorithm::SHA1, 6, 1, 30, secret)?;
                Ok(Some(totp.generate_current()?))
            }
            Err(_) => Ok(None),
        },
    }
}
