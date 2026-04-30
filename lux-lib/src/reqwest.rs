use reqwest::Client;

use crate::config::Config;

/// Returns a pre-configured HTTPS-only client.
pub(crate) fn new_https_client(config: &Config) -> Result<Client, reqwest::Error> {
    Client::builder()
        .https_only(true)
        .user_agent(config.user_agent())
        .build()
}

/// Returns a pre-configured HTTP client.
/// Used in tests and to fetch sources, which may be HTTP URLs.
pub(crate) fn new_http_client(config: &Config) -> Result<Client, reqwest::Error> {
    Client::builder().user_agent(config.user_agent()).build()
}
