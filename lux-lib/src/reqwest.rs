use std::sync::OnceLock;

use reqwest::Client;

use crate::config::Config;

static HTTPS_CLIENT: OnceLock<Client> = OnceLock::new();
static HTTP_CLIENT: OnceLock<Client> = OnceLock::new();

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
