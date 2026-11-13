use miette::Diagnostic;
use serde_enum_str::Serialize_enum_str;
use thiserror::Error;
use url::Url;

use crate::config::Config;

#[cfg(feature = "gpgme")]
use std::io::Read;

#[cfg(feature = "gpgme")]
use gpgme::{Data, ExportMode};
#[cfg(feature = "gpgme")]
use strum_macros::EnumIter;

#[derive(Serialize_enum_str, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "clap", clap(rename_all = "lowercase"))]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
#[cfg(not(feature = "gpgme"))]
pub enum Protocol {
    #[default]
    None,
}

#[derive(Serialize_enum_str, Clone, PartialEq, Eq, EnumIter)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "clap", clap(rename_all = "lowercase"))]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
#[cfg(feature = "gpgme")]
pub enum Protocol {
    None,
    Assuan,
    CMS,
    G13,
    GPGConf,
    #[default]
    OpenPGP,
    Spawn,
    UIServer,
}

#[cfg(feature = "gpgme")]
impl From<Protocol> for gpgme::Protocol {
    fn from(val: Protocol) -> Self {
        match val {
            Protocol::OpenPGP => gpgme::Protocol::OpenPgp,
            Protocol::CMS => gpgme::Protocol::Cms,
            Protocol::GPGConf => gpgme::Protocol::GpgConf,
            Protocol::Assuan => gpgme::Protocol::Assuan,
            Protocol::G13 => gpgme::Protocol::G13,
            Protocol::UIServer => gpgme::Protocol::UiServer,
            Protocol::Spawn => gpgme::Protocol::Spawn,
            Protocol::None => {
                unreachable!("cannot convert Protocol::None to a gpgme::Protocol")
            }
        }
    }
}

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[cfg(feature = "gpgme")]
    #[error("GPG signing failed")]
    #[diagnostic(help(
        r#"ensure that a GPG agent is running and that a valid GPG signing key is registered.
          if you'd like to skip the signing step, supply `--sign-protocol=none`
        "#
    ))]
    Signing(#[from] gpgme::Error),
    #[cfg(feature = "gpgme")]
    #[error("could not determine the signing key fingerprint")]
    MissingSignerFingerprint,
    #[cfg(feature = "gpgme")]
    #[error("could not export the signing public key")]
    #[diagnostic(help("ensure the signing key is exportable; smartcard keys may not be"))]
    MissingPublicKey,
    #[error("unable to read signing data")]
    #[diagnostic(help("check that the GPG agent is available"))]
    Read(#[source] std::io::Error),
}

pub(crate) struct Signature {
    pub(crate) signature: String,
    pub(crate) public_key: String,
}

#[cfg(feature = "gpgme")]
fn detached(ctx: &mut gpgme::Context, content: &[u8]) -> Result<(String, String), Error> {
    let mut signature = Data::new()?;
    let fingerprint = {
        let result = ctx.sign_detached(content, &mut signature)?;
        result
            .new_signatures()
            .next()
            .and_then(|signature| signature.fingerprint().ok())
            .map(str::to_owned)
    }
    .ok_or(Error::MissingSignerFingerprint)?;

    let mut signature_str = String::new();
    signature
        .read_to_string(&mut signature_str)
        .map_err(Error::Read)?;
    Ok((signature_str, fingerprint))
}

#[cfg(feature = "gpgme")]
fn export_public_key(ctx: &mut gpgme::Context, fingerprint: &str) -> Result<String, Error> {
    let mut public_key = Vec::new();
    ctx.export([fingerprint], ExportMode::empty(), &mut public_key)?;
    if public_key.is_empty() {
        return Err(Error::MissingPublicKey);
    }
    Ok(String::from_utf8_lossy(&public_key).into_owned())
}

#[cfg(feature = "gpgme")]
pub(crate) fn file(content: &[u8], protocol: Protocol) -> Result<Option<Signature>, Error> {
    if let Protocol::None = protocol {
        return Ok(None);
    }

    let mut ctx = gpgme::Context::from_protocol(protocol.into())?;
    ctx.set_armor(true);
    let (signature, fingerprint) = detached(&mut ctx, content)?;
    let public_key = export_public_key(&mut ctx, &fingerprint)?;
    Ok(Some(Signature {
        signature,
        public_key,
    }))
}

#[cfg(not(feature = "gpgme"))]
pub(crate) fn file(_content: &[u8], _protocol: Protocol) -> Result<Option<Signature>, Error> {
    Ok(None)
}

pub mod verify;
use verify::Error as VerifyError;
pub(crate) use verify::Verify;

pub(crate) async fn verify(config: &Config, url: &Url, bytes: &[u8]) -> Result<(), VerifyError> {
    Verify::new(config).verify(url, bytes).await
}
