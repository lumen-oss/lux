use miette::Diagnostic;
use reqwest::Client;
use serde::Deserialize;
use thiserror::Error;
use url::Url;

use crate::TOOL_VERSION;

#[derive(Deserialize, Debug)]
pub struct VersionCheckResponse {
    version: String,
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("error parsing tool check URL")]
    #[diagnostic(help("check your server configuration"))]
    ParseError(#[from] url::ParseError),
    #[error("error sending HTTP request")]
    #[diagnostic(help(
        r#"check your network connection and server configuration.
if the issue persists, the server may be temporarily unavailable."#
    ))]
    Request(#[from] crate::reqwest::Error),
    #[error(r#"Lux is out of date with {0}'s expected tool version.
    Lux is at version {TOOL_VERSION}, server is at {server_version}"#, server_version = _1.version)]
    #[diagnostic(help("ensure you have an up-to-date version of Lux installed"))]
    ToolOutdated(String, VersionCheckResponse),
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Self::Request(err.into())
    }
}

#[tracing::instrument(level = "trace", skip(client))]
pub(crate) async fn ensure_version(client: &Client, server_url: &Url) -> Result<(), Error> {
    let url = server_url.join("api/tool_version")?;
    let response: VersionCheckResponse = client
        .post(url)
        .json(&("current", TOOL_VERSION))
        .send()
        .await?
        .json()
        .await?;

    if response.version == TOOL_VERSION {
        Ok(())
    } else {
        Err(Error::ToolOutdated(server_url.to_string(), response))
    }
}
