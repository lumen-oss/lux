use miette::Diagnostic;
use thiserror::Error;
use url::Url;

use crate::config::{self, Config};

#[cfg(feature = "gpgme")]
use crate::reqwest::RequestBuilderExt;

pub(crate) struct Verify<'a> {
    config: &'a Config,
}

impl<'a> Verify<'a> {
    pub(crate) fn new(config: &'a Config) -> Self {
        Self { config }
    }

    pub(crate) async fn verify(&self, url: &Url, bytes: &[u8]) -> Result<(), Error> {
        let policy = self.config.verify();
        if policy == config::verify::Policy::Off {
            return Ok(());
        }
        match self.verify_inner(url, bytes).await {
            Ok(()) => Ok(()),
            Err(error) if policy == config::verify::Policy::Warn && !error.is_hard_failure() => {
                tracing::warn!("signature verification failed for {url}: {error}");
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    async fn verify_inner(&self, url: &Url, bytes: &[u8]) -> Result<(), Error> {
        #[cfg(not(feature = "gpgme"))]
        {
            let _ = (url, bytes);
            Err(Error::FeatureDisabled)
        }
        #[cfg(feature = "gpgme")]
        {
            let signature = self.fetch_signature(url).await?;
            verify_detached(&signature, bytes)
        }
    }

    #[cfg(feature = "gpgme")]
    async fn fetch_signature(&self, url: &Url) -> Result<Vec<u8>, Error> {
        let signature_url = signature_url(url);
        let response = self
            .config
            .http_client()?
            .get(signature_url.clone())
            .apply_access_token(self.config, &signature_url)
            .send()
            .await
            .map_err(|source| Error::SignatureFetch(signature_url.clone(), source))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::MissingSignature(signature_url));
        }
        Ok(response
            .error_for_status()
            .map_err(|source| Error::SignatureFetch(signature_url.clone(), source))?
            .bytes()
            .await
            .map_err(|source| Error::SignatureFetch(signature_url, source))?
            .to_vec())
    }
}

#[cfg(feature = "gpgme")]
fn signature_url(url: &Url) -> Url {
    let mut signature_url = url.clone();
    signature_url.set_path(&format!("{}.asc", url.path()));
    signature_url
}

#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("failed to create the HTTP client")]
    #[diagnostic(help("check your network configuration"))]
    Client(#[from] reqwest::Error),
    #[error("failed to fetch the signature for {0}")]
    #[diagnostic(help("the artifact may be unsigned or its signature may be unavailable"))]
    SignatureFetch(Url, #[source] reqwest::Error),
    #[error("the signature for {0} could not be found")]
    #[diagnostic(help("the artifact is not signed"))]
    MissingSignature(Url),
    #[error("signature verification is not supported in this build")]
    #[diagnostic(help("rebuild Lux with the `gpgme` feature"))]
    FeatureDisabled,
    #[cfg(feature = "gpgme")]
    #[error("GPG verification failed")]
    #[diagnostic(help(
        "ensure your GPG keyring is available and contains the signer's public key"
    ))]
    Gpg(#[from] gpgme::Error),
    #[error("the signature file contains no signatures")]
    #[diagnostic(help("the signature file is invalid"))]
    NoSignatures,
    #[error("the signer's public key could not be found")]
    #[diagnostic(help("import the signer's public key into your GPG keyring"))]
    MissingKey,
    #[error("the signature is invalid")]
    #[diagnostic(help("the artifact or its signature may have been tampered with"))]
    BadSignature,
    #[error("the signing key has been revoked")]
    Revoked,
    #[error("the signing key or signature has expired")]
    Expired,
}

impl Error {
    pub(crate) fn is_hard_failure(&self) -> bool {
        matches!(self, Self::Revoked | Self::Expired)
    }
}

#[cfg(feature = "gpgme")]
fn verify_detached(signature: &[u8], bytes: &[u8]) -> Result<(), Error> {
    let mut context = gpgme::Context::from_protocol(gpgme::Protocol::OpenPgp)?;
    verify_detached_with(&mut context, signature, bytes)
}

#[cfg(feature = "gpgme")]
fn verify_detached_with(
    context: &mut gpgme::Context,
    signature: &[u8],
    bytes: &[u8],
) -> Result<(), Error> {
    let result = context.verify_detached(signature, bytes)?;
    let mut found = false;
    for signature in result.signatures() {
        found = true;
        check_signature(&signature)?;
    }
    if !found {
        return Err(Error::NoSignatures);
    }
    Ok(())
}

#[cfg(feature = "gpgme")]
fn check_signature(signature: &gpgme::results::Signature<'_>) -> Result<(), Error> {
    use gpgme::SignatureSummary;

    let summary = signature.summary();
    if summary.contains(SignatureSummary::KEY_REVOKED) {
        return Err(Error::Revoked);
    }
    if summary.contains(SignatureSummary::KEY_EXPIRED)
        || summary.contains(SignatureSummary::SIG_EXPIRED)
    {
        return Err(Error::Expired);
    }
    if summary.contains(SignatureSummary::KEY_MISSING) {
        return Err(Error::MissingKey);
    }
    if let Err(error) = signature.status() {
        if error == gpgme::Error::NO_PUBKEY || error == gpgme::Error::NO_KEY {
            return Err(Error::MissingKey);
        }
        if error == gpgme::Error::BAD_SIGNATURE {
            return Err(Error::BadSignature);
        }
        if error == gpgme::Error::KEY_EXPIRED || error == gpgme::Error::SIG_EXPIRED {
            return Err(Error::Expired);
        }
        if error == gpgme::Error::CERT_REVOKED {
            return Err(Error::Revoked);
        }
        return Err(Error::Gpg(error));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::lua;
    use crate::operations::download;
    use crate::package;
    use httptest::matchers::request;
    use httptest::responders::status_code;
    use httptest::{Expectation, Server};

    fn config(server: &Server, policy: config::verify::Policy) -> (tempfile::TempDir, Config) {
        let dir = tempfile::TempDir::new().unwrap();
        let config = config::Builder::default()
            .server(Some(Url::parse(&server.url_str("/")).unwrap()))
            .data_dir(Some(dir.path().to_path_buf()))
            .cache_dir(Some(dir.path().to_path_buf()))
            .lua_version(Some(lua::Version::Lua51))
            .verify(Some(policy))
            .build()
            .unwrap();
        (dir, config)
    }

    #[tokio::test]
    async fn off_does_not_fetch_signature() {
        let server = Server::run();
        server.expect(
            Expectation::matching(request::path("/test.rockspec.asc"))
                .times(0)
                .respond_with(status_code(200)),
        );
        let (_dir, config) = config(&server, config::verify::Policy::Off);
        let url = Url::parse(&server.url_str("/test.rockspec")).unwrap();

        Verify::new(&config).verify(&url, b"content").await.unwrap();
    }

    #[cfg(feature = "gpgme")]
    #[tokio::test]
    async fn require_errors_on_missing_signature() {
        let server = Server::run();
        server.expect(
            Expectation::matching(request::path("/test.rockspec.asc"))
                .respond_with(status_code(404)),
        );
        let (_dir, config) = config(&server, config::verify::Policy::Require);
        let url = Url::parse(&server.url_str("/test.rockspec")).unwrap();

        let error = Verify::new(&config)
            .verify(&url, b"content")
            .await
            .unwrap_err();
        assert!(matches!(error, Error::MissingSignature(_)));
    }

    #[cfg(feature = "gpgme")]
    #[tokio::test]
    async fn warn_allows_missing_signature() {
        let server = Server::run();
        server.expect(
            Expectation::matching(request::path("/test.rockspec.asc"))
                .respond_with(status_code(404)),
        );
        let (_dir, config) = config(&server, config::verify::Policy::Warn);
        let url = Url::parse(&server.url_str("/test.rockspec")).unwrap();

        Verify::new(&config).verify(&url, b"content").await.unwrap();
    }

    #[cfg(not(feature = "gpgme"))]
    #[tokio::test]
    async fn require_errors_without_gpgme() {
        let server = Server::run();
        let (_dir, config) = config(&server, config::verify::Policy::Require);
        let url = Url::parse(&server.url_str("/test.rockspec")).unwrap();

        let error = Verify::new(&config)
            .verify(&url, b"content")
            .await
            .unwrap_err();
        assert!(matches!(error, Error::FeatureDisabled));
    }

    #[cfg(feature = "gpgme")]
    #[test]
    fn signature_url_is_appended_to_the_path() {
        let url = Url::parse("https://example.com/foo-1.0-1.src.rock?token=abc").unwrap();
        assert_eq!(
            signature_url(&url).as_str(),
            "https://example.com/foo-1.0-1.src.rock.asc?token=abc"
        );
    }

    #[cfg(feature = "gpgme")]
    #[tokio::test]
    async fn src_rock_verification_is_wired_into_download() {
        let server = Server::run();
        server.expect(
            Expectation::matching(request::path("/foo-1.0.0-1.src.rock"))
                .respond_with(status_code(200).body("not a real rock")),
        );
        server.expect(
            Expectation::matching(request::path("/foo-1.0.0-1.src.rock.asc"))
                .respond_with(status_code(404)),
        );
        let (_dir, config) = config(&server, config::verify::Policy::Require);
        let package = package::Spec::new("foo".into(), "1.0.0-1".parse().unwrap());
        let server_url = Url::parse(&server.url_str("/")).unwrap();

        let error = download::src_rock(&package, &server_url, &config)
            .await
            .err()
            .expect("expected verification to fail");
        assert!(matches!(error, download::SrcRockError::Verify(_)));
    }

    #[tokio::test]
    async fn binary_rock_fails_when_verification_is_required() {
        let server = Server::run();
        let (_dir, config) = config(&server, config::verify::Policy::Require);
        let package = package::Spec::new("foo".into(), "1.0.0-1".parse().unwrap());
        let server_url = Url::parse(&server.url_str("/")).unwrap();

        let error = download::binary_rock(&package, &server_url, &config)
            .await
            .err()
            .expect("expected binary rock verification to fail");
        assert!(matches!(error, download::SrcRockError::UnverifiedBinary(_)));
    }

    #[cfg(feature = "gpgme")]
    #[test]
    fn verifies_openpgp_signature() {
        let home = tempfile::TempDir::new().unwrap();
        let mut context = gpgme::Context::from_protocol(gpgme::Protocol::OpenPgp).unwrap();
        context
            .set_engine_home_dir(home.path().to_str().unwrap())
            .unwrap();
        let fixtures =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/signatures");
        let import = context
            .import(std::fs::read_to_string(fixtures.join("test-key.asc")).unwrap())
            .unwrap();
        assert!(import.imported() > 0 || import.unchanged() > 0);

        let payload = std::fs::read(fixtures.join("test.rockspec")).unwrap();
        let signature = std::fs::read(fixtures.join("test.rockspec.asc")).unwrap();
        verify_detached_with(&mut context, &signature, &payload).unwrap();

        let error = verify_detached_with(&mut context, &signature, b"tampered").unwrap_err();
        assert!(matches!(error, Error::BadSignature | Error::Gpg(_)));
    }
}
