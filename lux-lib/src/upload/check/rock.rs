use std::collections::HashMap;

use miette::Diagnostic;
use reqwest::Client;
use thiserror::Error;
use url::Url;

use crate::package;
use crate::upload::{url_for_method, ApiKey};

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("parse error while checking rock status on server")]
    #[diagnostic(help("check your server configuration"))]
    ParseError(#[from] url::ParseError),
    #[error("HTTP request error while checking rock status on server")]
    #[diagnostic(help(
        r#"check your network connection and server configuration.
if the issue persists, the server may be temporarily unavailable."#
    ))]
    Request(#[from] crate::reqwest::Error),
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Self::Request(err.into())
    }
}

pub(crate) async fn exists(
    client: &Client,
    api_key: &ApiKey,
    name: &package::Name,
    version: &package::Version,
    server: &Url,
) -> Result<bool, Error> {
    let server_response_raw_json = client
        .get(unsafe { url_for_method(server, api_key, "check_rockspec")? })
        .query(&(
            ("package", name.to_string()),
            ("version", version.to_string()),
        ))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(exists_in_response(&server_response_raw_json))
}

fn exists_in_response(response: &str) -> bool {
    serde_json::from_str::<HashMap<String, serde_json::Value>>(response).is_ok_and(|response| {
        let present = |key| response.get(key).is_some_and(|value| !value.is_null());
        present("module") && present("version")
    })
}

#[cfg(test)]
mod test {
    #[test]
    fn test_exists_in_response() {
        for (response, expected) in [
            ("{}", false),
            (r#"{"module":null,"version":null}"#, false),
            (r#"{"module":{},"version":null}"#, false),
            (r#"{"module":null,"version":{}}"#, false),
            (r#"{"module":{},"version":{}}"#, true),
        ] {
            assert_eq!(super::exists_in_response(response), expected);
        }
    }
}
