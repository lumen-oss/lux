pub(crate) mod package;

use self::package::WALLY_VERSION;

use url::Url;

/// Adds the `Wally-Version` header required by wally registries for content requests.
pub(crate) fn apply_content_headers(
    request: reqwest::RequestBuilder,
    url: &Url,
) -> reqwest::RequestBuilder {
    if url.path().starts_with("/v1/package-contents/") {
        request.header("Wally-Version", WALLY_VERSION)
    } else {
        request
    }
}
