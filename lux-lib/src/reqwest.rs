use std::sync::OnceLock;

use miette::Diagnostic;
use reqwest::{header::AUTHORIZATION, Client, RequestBuilder};
use thiserror::Error;
use url::Url;

use crate::config::{access_tokens::Kind, Config};

const SOURCEHUT_HOST: &str = "git.sr.ht";

static HTTPS_CLIENT: OnceLock<Client> = OnceLock::new();
static HTTP_CLIENT: OnceLock<Client> = OnceLock::new();

/// An error that occurred while performing a web request.
#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum RequestError {
    #[error("request refused")]
    RequestRefused {
        #[help]
        help: String,
        source: reqwest::Error,
    },
    #[error("resource not found")]
    #[diagnostic(help("the resource could not be found. verify that it exists on the server."))]
    NotFound(#[source] reqwest::Error),
    #[error("request failed")]
    #[diagnostic(help("check your network connection."))]
    Other(#[source] reqwest::Error),
}

impl From<reqwest::Error> for RequestError {
    fn from(err: reqwest::Error) -> Self {
        if err.status() == Some(reqwest::StatusCode::NOT_FOUND) {
            Self::NotFound(err)
        } else if err.status().is_some_and(|status| status.is_client_error()) {
            let for_host_str = match (err.url(), err.url().and_then(Url::host_str)) {
                (_, Some(host)) => format!(" for {host}"),
                (Some(url), None) => format!(" for the host of {url}"),
                (None, None) => "".to_string(),
            };
            Self::RequestRefused {
                help: format!(
                    r#"this could be an access-token issue. consider adding an access token{for_host_str}
                        in the `[access_tokens]` section of your Lux config,
                        or via the `LUX_ACCESS_TOKENS` environment variable."#
                ),
                source: err,
            }
        } else {
            Self::Other(err)
        }
    }
}

/// Extension trait for [`reqwest::RequestBuilder`] that attaches
/// an authentication header for the URL's host,
/// if an access token is configured for it.
pub(crate) trait RequestBuilderExt: private::Sealed {
    fn apply_access_token(self, config: &Config, url: &Url) -> RequestBuilder;
}

impl RequestBuilderExt for reqwest::RequestBuilder {
    fn apply_access_token(self, config: &Config, url: &Url) -> RequestBuilder {
        if let Some(host) = url.host_str() {
            if let Some(token) = config.access_token(host) {
                let password = unsafe { token.password() };
                return match (token.kind(), host) {
                    (Kind::GitLabPAT, _) => self.header("PRIVATE-TOKEN", password),
                    (Kind::GitLabOAuth2, _) | (Kind::Plain, SOURCEHUT_HOST) => {
                        self.header(AUTHORIZATION, format!("Bearer {password}"))
                    }
                    (Kind::Plain, _) => self.header(AUTHORIZATION, format!("token {password}")),
                };
            }
        }
        self
    }
}

fn client(https_only: bool, config: &Config) -> Result<&Client, reqwest::Error> {
    let cache = if https_only {
        &HTTPS_CLIENT
    } else {
        &HTTP_CLIENT
    };
    if let Some(client) = cache.get() {
        Ok(client)
    } else {
        let mut builder = Client::builder();
        if https_only {
            builder = builder.https_only(true);
        }
        let client = builder.user_agent(config.user_agent()).build()?;
        // TODO: use get_or_try_init when available
        Ok(cache.get_or_init(|| client))
    }
}

/// Returns a pre-configured HTTPS-only client.
pub(crate) fn https_client(config: &Config) -> Result<&Client, reqwest::Error> {
    client(true, config)
}

/// Returns a pre-configured HTTP client.
/// Used in tests and to fetch sources, which may be HTTP URLs.
pub(crate) fn http_client(config: &Config) -> Result<&Client, reqwest::Error> {
    client(false, config)
}

mod private {
    pub trait Sealed {}
    impl Sealed for reqwest::RequestBuilder {}
}

#[cfg(test)]
mod tests {
    use httptest::{
        matchers::{contains, request},
        responders::status_code,
        Expectation, Server,
    };

    use super::*;

    fn config_with_token(host: &str, raw_token: &str) -> crate::config::Config {
        let config: crate::config::ConfigBuilder =
            toml::from_str(&format!("[access_tokens]\n\"{host}\" = {raw_token:?}\n")).unwrap();
        config.build().unwrap()
    }

    #[tokio::test]
    async fn test_plain_token_uses_token_header() {
        let server = Server::run();
        server.expect(
            Expectation::matching(request::headers(contains(("authorization", "token abc"))))
                .respond_with(status_code(200)),
        );
        let url = Url::parse(&server.url("/").to_string()).unwrap();
        let config = config_with_token(url.host_str().unwrap(), "abc");
        let response = Client::new()
            .get(url.clone())
            .apply_access_token(&config, &url)
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
    }

    #[test]
    fn test_sourcehut_plain_token_uses_bearer_header() {
        let url = Url::parse("https://git.sr.ht/~user/repo/archive/ref.tar.gz").unwrap();
        let config = config_with_token("git.sr.ht", "abc");
        let request = Client::new()
            .get(url.clone())
            .apply_access_token(&config, &url)
            .build()
            .unwrap();
        assert_eq!(
            request.headers().get("authorization").unwrap(),
            "Bearer abc"
        );
    }

    #[tokio::test]
    async fn test_oauth2_token_uses_bearer_header() {
        let server = Server::run();
        server.expect(
            Expectation::matching(request::headers(contains(("authorization", "Bearer abc"))))
                .respond_with(status_code(200)),
        );
        let url = Url::parse(&server.url("/").to_string()).unwrap();
        let config = config_with_token(url.host_str().unwrap(), "OAuth2:abc");
        let response = Client::new()
            .get(url.clone())
            .apply_access_token(&config, &url)
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
    }

    #[tokio::test]
    async fn test_pat_token_uses_private_token_header() {
        let server = Server::run();
        server.expect(
            Expectation::matching(request::headers(contains(("private-token", "abc"))))
                .respond_with(status_code(200)),
        );
        let url = Url::parse(&server.url("/").to_string()).unwrap();
        let config = config_with_token(url.host_str().unwrap(), "PAT:abc");
        let response = Client::new()
            .get(url.clone())
            .apply_access_token(&config, &url)
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
    }
}
