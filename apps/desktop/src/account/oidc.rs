//! The account's provider as an OAuth 2.0 and OIDC provider: its discovery document,
//! and the token endpoint's requests and answers (a code exchanged, a device code polled, a
//! refresh token turned over). The app is a public client: no secret, PKCE for codes.

use std::fmt;
use std::time::Duration;

use serde::Deserialize;

use super::http::{Request, Response};
use super::switch::is_loopback_host;

/// The scopes asked for: the account's id, and a refresh token.
pub(crate) const SCOPES: &str = "openid offline_access";
/// The one redirect the OAuth application has: a universal link only this app receives.
pub(crate) const CALLBACK: &str = "https://emberhut.com/oauth/callback";
/// The device grant's `grant_type` (RFC 8628).
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// Where the issuer's discovery document is.
#[must_use]
pub(crate) fn discovery_url(issuer: &str) -> String {
    format!("{issuer}/.well-known/openid-configuration")
}

/// The provider's endpoints, from its discovery document.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct Discovery {
    /// The issuer it says it is.
    pub(crate) issuer: String,
    /// Where a browser signs in (the web authentication session).
    pub(crate) authorization_endpoint: String,
    /// Where codes and refresh tokens are traded for tokens.
    pub(crate) token_endpoint: String,
    /// Where the device grant starts, when the provider has it.
    #[serde(default)]
    pub(crate) device_authorization_endpoint: Option<String>,
    /// Where an access token says whose it is.
    #[serde(default)]
    pub(crate) userinfo_endpoint: Option<String>,
}

/// Why a discovery document isn't used.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DiscoveryError {
    /// It doesn't read as one.
    #[error("the account's provider sent a discovery document this app can't read")]
    Unreadable,
    /// It names another issuer than the one built in.
    #[error("the account's provider says it is another issuer")]
    OtherIssuer,
    /// An endpoint isn't `https://` (or `http://` to this computer).
    #[error("the account's provider named an endpoint that isn't https")]
    Insecure,
}

impl Discovery {
    /// The document in `body`, checked against `issuer`.
    ///
    /// # Errors
    ///
    /// [`DiscoveryError`] when it doesn't read, is another issuer's, or names a plain-HTTP
    /// endpoint off this computer.
    pub(crate) fn read(body: &[u8], issuer: &str) -> Result<Self, DiscoveryError> {
        let found: Self = serde_json::from_slice(body).map_err(|_| DiscoveryError::Unreadable)?;
        if found.issuer.trim_end_matches('/') != issuer.trim_end_matches('/') {
            return Err(DiscoveryError::OtherIssuer);
        }
        let endpoints = [
            Some(&found.authorization_endpoint),
            Some(&found.token_endpoint),
            found.device_authorization_endpoint.as_ref(),
            found.userinfo_endpoint.as_ref(),
        ];
        if endpoints.into_iter().flatten().all(|url| is_secure(url)) {
            Ok(found)
        } else {
            Err(DiscoveryError::Insecure)
        }
    }

    /// Trades a code from the web authentication session (and its PKCE verifier) for tokens.
    #[must_use]
    pub(crate) fn code_request(&self, client_id: &str, code: &str, verifier: &str) -> Request {
        Request::form(
            &self.token_endpoint,
            vec![
                ("grant_type", "authorization_code".to_owned()),
                ("client_id", client_id.to_owned()),
                ("code", code.to_owned()),
                ("code_verifier", verifier.to_owned()),
                ("redirect_uri", CALLBACK.to_owned()),
            ],
        )
    }

    /// Asks once whether the device code was approved.
    #[must_use]
    pub(crate) fn device_poll_request(&self, client_id: &str, device_code: &str) -> Request {
        Request::form(
            &self.token_endpoint,
            vec![
                ("grant_type", DEVICE_GRANT.to_owned()),
                ("client_id", client_id.to_owned()),
                ("device_code", device_code.to_owned()),
            ],
        )
    }

    /// Turns a refresh token over for a new access token.
    #[must_use]
    pub(crate) fn refresh_request(&self, client_id: &str, refresh: &str) -> Request {
        Request::form(
            &self.token_endpoint,
            vec![
                ("grant_type", "refresh_token".to_owned()),
                ("client_id", client_id.to_owned()),
                ("refresh_token", refresh.to_owned()),
            ],
        )
    }
}

/// Whether `url` is `https://`, or `http://` to this computer.
fn is_secure(url: &str) -> bool {
    url.starts_with("https://") || url.strip_prefix("http://").is_some_and(is_loopback_host)
}

/// The token endpoint's answer when it grants tokens.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct TokenAnswer {
    /// The access token.
    pub(crate) access_token: String,
    /// Its lifetime in seconds, when said.
    #[serde(default)]
    pub(crate) expires_in: Option<u64>,
    /// The refresh token, when given.
    #[serde(default)]
    pub(crate) refresh_token: Option<String>,
    /// The ID token, when given (with `openid`).
    #[serde(default)]
    pub(crate) id_token: Option<String>,
}

// Its `Debug` shows which tokens came, never the tokens.
impl fmt::Debug for TokenAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenAnswer")
            .field("expires_in", &self.expires_in)
            .field("refresh_token", &self.refresh_token.is_some())
            .field("id_token", &self.id_token.is_some())
            .finish_non_exhaustive()
    }
}

/// What the token endpoint answered.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TokenReply {
    /// Tokens.
    Granted(TokenAnswer),
    /// An OAuth error (RFC 6749, section 5.2; RFC 8628, section 3.5), by its `error`.
    Refused(String),
    /// 429 or a 5xx, whatever the body says: try again later.
    Unavailable {
        /// The status.
        status: u16,
        /// `Retry-After`, when given.
        retry_after: Option<Duration>,
    },
    /// Anything else: a status without an OAuth error, or a body that doesn't read.
    Failed(u16),
}

impl TokenReply {
    /// Reads the token endpoint's `response`, by its status first.
    #[must_use]
    pub(crate) fn read(response: &Response) -> Self {
        #[derive(Deserialize)]
        struct OAuthError {
            error: String,
        }
        if response.status == 429 || response.status >= 500 {
            return Self::Unavailable {
                status: response.status,
                retry_after: response.retry_after,
            };
        }
        if (200..300).contains(&response.status) {
            return serde_json::from_slice(&response.body)
                .map_or(Self::Failed(response.status), Self::Granted);
        }
        match serde_json::from_slice::<OAuthError>(&response.body) {
            Ok(refused) if (400..500).contains(&response.status) => Self::Refused(refused.error),
            _ => Self::Failed(response.status),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const ISSUER: &str = "https://accounts.example";

    fn document(issuer: &str, token: &str) -> Vec<u8> {
        json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{ISSUER}/oauth/authorize"),
            "token_endpoint": token,
            "device_authorization_endpoint": format!("{ISSUER}/oauth/device"),
            "jwks_uri": format!("{ISSUER}/.well-known/jwks.json"),
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn discovery_is_the_issuers_own() {
        let found = Discovery::read(
            &document(
                "https://accounts.example/",
                "https://accounts.example/oauth/token",
            ),
            ISSUER,
        )
        .unwrap();
        assert_eq!(found.token_endpoint, "https://accounts.example/oauth/token");
        assert_eq!(
            found.device_authorization_endpoint.as_deref(),
            Some("https://accounts.example/oauth/device")
        );
        assert_eq!(
            Discovery::read(&document("https://other.example", "https://x/t"), ISSUER),
            Err(DiscoveryError::OtherIssuer)
        );
        assert_eq!(
            Discovery::read(&document(ISSUER, "http://accounts.example/t"), ISSUER),
            Err(DiscoveryError::Insecure)
        );
        assert_eq!(
            Discovery::read(b"<html>", ISSUER),
            Err(DiscoveryError::Unreadable)
        );
        assert_eq!(
            discovery_url(ISSUER),
            "https://accounts.example/.well-known/openid-configuration"
        );
    }

    #[test]
    fn the_token_requests_are_a_public_clients() {
        let found = Discovery::read(
            &document(ISSUER, "https://accounts.example/oauth/token"),
            ISSUER,
        )
        .unwrap();
        let code = found.code_request("client", "the-code", "the-verifier");
        assert_eq!(code.field("grant_type"), Some("authorization_code"));
        assert_eq!(code.field("code_verifier"), Some("the-verifier"));
        assert_eq!(code.field("redirect_uri"), Some(CALLBACK));
        assert_eq!(code.field("client_secret"), None);
        let poll = found.device_poll_request("client", "device-code");
        assert_eq!(poll.field("grant_type"), Some(DEVICE_GRANT));
        assert_eq!(poll.field("device_code"), Some("device-code"));
        let refresh = found.refresh_request("client", "r1");
        assert_eq!(refresh.field("grant_type"), Some("refresh_token"));
        assert_eq!(refresh.field("refresh_token"), Some("r1"));
    }

    #[test]
    fn the_token_endpoints_answers_read() {
        let answer = |status, body: serde_json::Value| Response {
            status,
            retry_after: None,
            body: body.to_string().into_bytes(),
        };
        assert!(matches!(
            TokenReply::read(&answer(
                200,
                json!({"access_token": "a", "token_type": "Bearer"})
            )),
            TokenReply::Granted(TokenAnswer {
                expires_in: None,
                ..
            })
        ));
        assert_eq!(
            TokenReply::read(&answer(400, json!({"error": "slow_down"}))),
            TokenReply::Refused("slow_down".to_owned())
        );
        assert_eq!(
            TokenReply::read(&answer(503, json!({"error": "temporarily_unavailable"}))),
            TokenReply::Unavailable {
                status: 503,
                retry_after: None
            },
            "by the status first, whatever the body"
        );
        assert_eq!(
            TokenReply::read(&answer(429, json!({"error": "invalid_grant"}))),
            TokenReply::Unavailable {
                status: 429,
                retry_after: None
            }
        );
        assert_eq!(
            TokenReply::read(&answer(200, json!({"no": "token"}))),
            TokenReply::Failed(200)
        );
    }
}
