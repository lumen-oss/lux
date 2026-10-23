use std::path::{Path, PathBuf};

use clap::Args;
use lux_lib::{
    config::Config, lua_rockspec::RemoteLuaRockspec, package::PackageName, package_db::PackageDB,
    upload::ProjectUpload, upload::RockspecUpload, workspace::Workspace,
};

use miette::{miette, IntoDiagnostic, Result};

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
    pub(crate) package: Option<PackageName>,

    /// Path to a `.rockspec` file to upload.{n}
    /// If not set, uploads the current project or workspace.
    #[arg(value_name = "PATH", conflicts_with = "package", value_parser = ensure_rockspec)]
    pub(crate) path: Option<PathBuf>,

    /// Code to use for two-factor authentication (2FA).{n}
    /// It is recommended to enable 2FA for luarocks uploads (see https://luarocks.org/settings/two-factor-auth).{n}
    /// Lux can also generate a TOTP code if you expose your luarocks.org 2FA secret via the 'LUAROCKS_2FA_SECRET' environment variable.{n}
    #[arg(short, long, visible_short_alias = 'c')]
    tfa_code: Option<String>,
}

pub async fn upload(data: Upload, config: Config) -> Result<()> {
    if let Some(path) = data.path.as_deref() {
        return upload_rockspec(path, &data, &config).await;
    }

    let workspace = Workspace::current_or_err()?;

    let package_db = PackageDB::from_config(&config).await?;
    let tfa_code = tfa_code_from_args_or_secret(&data, &config)?;
    if let Some(package) = data.package {
        let project = workspace.select_member(&package)?;
        let project_upload = ProjectUpload::new()
            .project(project)
            .config(&config)
            .maybe_tfa_code(tfa_code)
            .package_db(&package_db);

        #[cfg(feature = "gpgme")]
        let project_upload = project_upload.sign_protocol(data.sign_protocol.clone());

        project_upload.upload_to_luarocks().await?;
    } else {
        for project in workspace.members() {
            let project_upload = ProjectUpload::new()
                .project(project)
                .config(&config)
                .maybe_tfa_code(tfa_code.clone())
                .package_db(&package_db);

            #[cfg(feature = "gpgme")]
            let project_upload = project_upload.sign_protocol(data.sign_protocol.clone());

            project_upload.upload_to_luarocks().await?;
        }
    }

    Ok(())
}

async fn upload_rockspec(path: &Path, data: &Upload, config: &Config) -> Result<()> {
    let content = tokio::fs::read_to_string(path).await.into_diagnostic()?;
    let rockspec = RemoteLuaRockspec::new(&content)?;
    let tfa_code = tfa_code_from_args_or_secret(data, config)?;

    let upload = RockspecUpload::new()
        .rockspec(&rockspec)
        .config(config)
        .maybe_tfa_code(tfa_code);

    #[cfg(feature = "gpgme")]
    let upload = upload.sign_protocol(data.sign_protocol.clone());

    upload.upload_to_luarocks().await?;
    Ok(())
}

fn tfa_code_from_args_or_secret(data: &Upload, config: &Config) -> Result<Option<String>> {
    match &data.tfa_code {
        Some(code) => Ok(Some(code.to_owned())),
        None => match std::env::var("LUAROCKS_2FA_SECRET") {
            Ok(secret) => {
                let secret = base32::decode(base32::Alphabet::Crockford, &secret)
                    .ok_or(totp_rs::SecretParseError::ParseBase32)
                    .into_diagnostic()?;
                let totp = totp_rs::Builder::new()
                    .with_algorithm(totp_rs::Algorithm::SHA1)
                    .with_digits(6)
                    .with_skew(1)
                    .with_step_duration(30)
                    .with_secret(secret)
                    .build()
                    .into_diagnostic()?;
                Ok(Some(totp.generate_current().to_string()))
            }
            Err(_) => {
                if config.no_prompt() || config.no_tfa() {
                    Ok(None)
                } else {
                    let code = inquire::Password::new("Enter your 2FA code:")
                        .without_confirmation()
                        .prompt()
                        .into_diagnostic()?;
                    Ok(Some(code))
                }
            }
        },
    }
}

fn ensure_rockspec(s: &str) -> Result<PathBuf, miette::Report> {
    let path = PathBuf::from(s);

    if path.extension().and_then(|ext| ext.to_str()) != Some("rockspec") {
        return Err(miette!(
            help = "pass a path to a '.rockspec' file",
            "'{}' is not a rockspec",
            path.display()
        ));
    }

    if !path.is_file() {
        return Err(miette!("rockspec '{}' does not exist", path.display()));
    }

    Ok(path)
}
