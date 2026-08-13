use std::collections::HashMap;
use std::fmt;

use serde::Deserialize;

/// The environment variable for setting access tokens without a config file.
pub(crate) const ACCESS_TOKENS_ENV: &str = "LUX_ACCESS_TOKENS";

/// The type of an access token.
/// Determines how the token is presented (e.g. which HTTP header to send it in).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A plain token, e.g. a Codeberg/Sourcehut/GitHub personal access token.
    Plain,
    /// A GitLab personal access token, configured as `PAT:<token>`.
    GitLabPAT,
    /// A GitLab OAuth2 token, configured as `OAuth2:<token>`.
    GitLabOAuth2,
}

/// An access token configured for a host, via the `[access_tokens]` section
/// of the config file, or the `LUX_ACCESS_TOKENS` environment variable.
#[derive(Clone, PartialEq, Eq)]
pub struct AccessToken {
    kind: Kind,
    token: String,
}

impl fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessToken")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl From<&str> for AccessToken {
    fn from(raw: &str) -> Self {
        let (kind, token) = if let Some(rest) = raw.strip_prefix("PAT:") {
            (Kind::GitLabPAT, rest)
        } else if let Some(rest) = raw.strip_prefix("OAuth2:") {
            (Kind::GitLabOAuth2, rest)
        } else {
            (Kind::Plain, raw)
        };
        Self {
            kind,
            token: token.into(),
        }
    }
}

impl<'de> Deserialize<'de> for AccessToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer).map(|raw| raw.as_str().into())
    }
}

impl AccessToken {
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Username to use for git-over-HTTPS / basic auth.
    pub fn username(&self) -> &str {
        match self.kind {
            Kind::Plain => "x-access-token",
            Kind::GitLabPAT | Kind::GitLabOAuth2 => "oauth2",
        }
    }

    /// Retrieves the underlying token as a, without any type prefix.
    ///
    /// # Safety
    ///
    /// The token is a secret. Ensure that you never pass this variable
    /// somewhere it may be displayed or logged.
    pub unsafe fn password(&self) -> &str {
        &self.token
    }
}

/// Parse the `LUX_ACCESS_TOKENS` environment variable: a whitespace-separated
/// list of `host=token` pairs.
/// Entries without an `=` are ignored.
pub(crate) fn env_access_tokens() -> HashMap<String, AccessToken> {
    let Ok(raw) = std::env::var(ACCESS_TOKENS_ENV) else {
        return HashMap::new();
    };
    let mut tokens = HashMap::new();
    for entry in raw.split_whitespace() {
        if let Some((host, token)) = entry.split_once('=') {
            tokens.insert(host.to_string(), AccessToken::from(token));
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_strips_known_prefixes() {
        let token = AccessToken::from("PAT:A123Bp_Cd..EfG");
        assert_eq!(token.kind(), Kind::GitLabPAT);
        assert_eq!(token.username(), "oauth2");
        assert_eq!(unsafe { token.password() }, "A123Bp_Cd..EfG");

        let token = AccessToken::from("OAuth2:1jklw3jk");
        assert_eq!(token.kind(), Kind::GitLabOAuth2);
        assert_eq!(token.username(), "oauth2");
        assert_eq!(unsafe { token.password() }, "1jklw3jk");

        let token = AccessToken::from("23ac...b289");
        assert_eq!(token.kind(), Kind::Plain);
        assert_eq!(token.username(), "x-access-token");
        assert_eq!(unsafe { token.password() }, "23ac...b289");
    }

    #[test]
    fn env_access_tokens_parses_pairs_and_ignores_bare_entries() {
        std::env::set_var(
            ACCESS_TOKENS_ENV,
            "github.com=23ac...b289 gitlab.com=OAuth2:1jklw3jk",
        );
        let tokens = env_access_tokens();
        assert_eq!(
            tokens.get("github.com").cloned(),
            Some(AccessToken::from("23ac...b289"))
        );
        assert_eq!(
            tokens.get("gitlab.com").cloned(),
            Some(AccessToken::from("OAuth2:1jklw3jk"))
        );
        std::env::remove_var(ACCESS_TOKENS_ENV);

        std::env::set_var(ACCESS_TOKENS_ENV, "gitlab.com=tok,en");
        let tokens = env_access_tokens();
        assert_eq!(
            tokens.get("gitlab.com").cloned(),
            Some(AccessToken::from("tok,en"))
        );
        std::env::remove_var(ACCESS_TOKENS_ENV);

        std::env::set_var(ACCESS_TOKENS_ENV, "23ac...b289");
        assert!(env_access_tokens().is_empty());
        std::env::remove_var(ACCESS_TOKENS_ENV);

        std::env::set_var(ACCESS_TOKENS_ENV, "");
        assert!(env_access_tokens().is_empty());
        std::env::remove_var(ACCESS_TOKENS_ENV);
    }
}
