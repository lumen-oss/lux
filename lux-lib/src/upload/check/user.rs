use miette::Diagnostic;
use reqwest::{Client, StatusCode};
use thiserror::Error;
use url::Url;

use crate::upload::{url_for_method, ApiKey};

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("error parsing user check URL")]
    #[diagnostic(help("check your server configuration"))]
    Parse(#[from] url::ParseError),
    #[error(transparent)]
    #[diagnostic(help(
        r#"check your network connection and server configuration.
if the issue persists, the server may be temporarily unavailable."#
    ))]
    Request(#[from] crate::reqwest::Error),
    #[diagnostic(help(
        "ensure that the API key provided via the $LUX_API_KEY environemnt variable is correct"
    ))]
    #[error("invalid API key provided")]
    UserNotFound,
    #[error("server '{0}' responded with error status: '{1}'")]
    #[diagnostic(help("the server may be temporarily unavailable"))]
    Server(Url, StatusCode),
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Self::Request(err.into())
    }
}

#[tracing::instrument(level = "trace", skip(client, api_key))]
pub(crate) async fn ensure(
    client: &Client,
    api_key: &ApiKey,
    server_url: &Url,
) -> Result<(), Error> {
    let response = client
        .get(unsafe { url_for_method(server_url, api_key, "status")? })
        .send()
        .await?;
    let status = response.status();
    if status.is_client_error() {
        Err(Error::UserNotFound)
    } else if status.is_server_error() {
        Err(Error::Server(server_url.clone(), status))
    } else {
        Ok(())
    }
}
