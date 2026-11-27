//! Signing in to the emberhut.com account from the command line: the OAuth 2.0 device
//! authorization grant (RFC 8628), as a public client with no secret (ADR 0007, section 5).
//!
//! The issuer's OIDC discovery document names the endpoints. The person opens the
//! verification address on any device and enters the code shown; meanwhile the token endpoint
//! is polled at the `interval` the issuer gave (kept between 1 and 60 seconds), five seconds
//! slower after each `slow_down`, until `expires_in` has passed (at most 30 minutes). A poll
//! that fails on the way (the issuer unreachable, or a 5xx) is tried again at the next
//! interval, a few times in a row before the grant gives up. The access token is used once, for the link, and never kept.
//!
//! Every endpoint a token or a code goes to must be `https://`, or `http://` on loopback (a
//! stand-in on this computer, as tests use).

use std::fmt;
use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use url::Url;
use zeroize::Zeroizing;

use crate::http::read_body;

/// How long one request to the issuer may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// The most of an answer that is read.
const ANSWER_LIMIT: usize = 64 * 1024;
/// The wait between polls when the issuer names none (RFC 8628, section 3.2).
const DEFAULT_INTERVAL: Duration = Duration::from_secs(5);
/// How much longer to wait after each `slow_down` (RFC 8628, section 3.5).
const SLOW_DOWN_STEP: Duration = Duration::from_secs(5);
/// The shortest wait between polls, whatever the issuer says.
const MIN_INTERVAL: Duration = Duration::from_secs(1);
/// The longest wait between polls, whatever the issuer says or `slow_down` adds.
const MAX_INTERVAL: Duration = Duration::from_mins(1);
/// The longest a grant is waited for, whatever the issuer says.
const MAX_EXPIRES_IN: Duration = Duration::from_mins(30);
/// How many polls in a row may fail on the way before the grant gives up.
const TRANSIENT_FAILURES: u32 = 3;
/// The grant type of a device code at the token endpoint.
const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// What the token is asked for: the person's user id, from the user info endpoint.
const SCOPE: &str = "openid";

/// Why signing in to the account failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OauthError {
    /// The issuer couldn't be reached.
    #[error("cannot reach the account's sign-in at {url}")]
    Unreachable {
        /// Where.
        url: Url,
        /// Why not.
        #[source]
        source: reqwest::Error,
    },
    /// An answer that isn't what the protocol says.
    #[error("the account's sign-in at {url} answered {status}: {detail}")]
    Unexpected {
        /// Where.
        url: Url,
        /// Its status.
        status: StatusCode,
        /// What was wrong with it.
        detail: String,
    },
    /// An address a code or token would go to over plain HTTP to another computer.
    #[error("{0} is not https://, so no code or token is sent there")]
    Insecure(Url),
    /// The discovery document is another issuer's.
    #[error("the sign-in's discovery document is for {got}, not {expected}")]
    OtherIssuer {
        /// The issuer configured.
        expected: Url,
        /// The issuer the document names.
        got: String,
    },
    /// The issuer doesn't offer the device grant.
    #[error(
        "the account's sign-in doesn't offer the device authorization grant: link this \
         machine from a signed-in desktop on it instead"
    )]
    NoDeviceGrant,
    /// The person declined the sign-in.
    #[error("the sign-in was declined")]
    Denied,
    /// The code ran out before the sign-in finished.
    #[error("the code expired before the sign-in finished: run the command again")]
    Expired,
    /// The token endpoint refused the code for another reason.
    #[error("the account's sign-in refused the code: {0}")]
    Refused(String),
}

/// The endpoints the issuer's discovery document names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    /// Where a device grant starts.
    pub device_authorization: Url,
    /// Where the code is traded for a token.
    pub token: Url,
    /// Where the token names its user.
    pub userinfo: Url,
}

/// A device grant under way: what the person is shown, and the code that is polled with.
pub struct DeviceAuthorization {
    device_code: Zeroizing<String>,
    /// The code the person enters.
    pub user_code: String,
    /// Where the person enters it.
    pub verification_uri: Url,
    /// The same address with the code filled in, when the issuer gives one.
    pub verification_uri_complete: Option<Url>,
    /// How long the code works.
    pub expires_in: Duration,
    /// How long to wait between polls.
    pub interval: Duration,
}

impl fmt::Debug for DeviceAuthorization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceAuthorization")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri.as_str())
            .field("expires_in", &self.expires_in)
            .field("interval", &self.interval)
            .finish_non_exhaustive()
    }
}

/// An account access token, held only for the request it is for. Its `Debug` never shows it,
/// and its bytes are wiped when it is dropped.
pub struct AccountToken(Zeroizing<String>);

impl AccountToken {
    /// The token, for an `Authorization: Bearer` header. Never log it.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AccountToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AccountToken(<redacted>)")
    }
}

/// Whether a code or token may go to `url`: `https://`, or plain HTTP on loopback.
#[must_use]
pub fn is_secure(url: &Url) -> bool {
    match (url.scheme(), url.host()) {
        ("https", Some(_)) => true,
        ("http", Some(url::Host::Ipv4(ip))) => ip.is_loopback(),
        ("http", Some(url::Host::Ipv6(ip))) => ip.is_loopback(),
        ("http", Some(url::Host::Domain(name))) => name.eq_ignore_ascii_case("localhost"),
        _ => false,
    }
}

/// `url`, when a code or token may go there.
///
/// # Errors
///
/// [`OauthError::Insecure`] otherwise.
pub fn secure(url: Url) -> Result<Url, OauthError> {
    if is_secure(&url) {
        Ok(url)
    } else {
        Err(OauthError::Insecure(url))
    }
}

/// The OIDC discovery document's fields this needs.
#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    device_authorization_endpoint: Option<Url>,
    token_endpoint: Url,
    userinfo_endpoint: Option<Url>,
}

/// The device authorization answer (RFC 8628, section 3.2).
#[derive(Deserialize)]
struct AuthorizationAnswer {
    device_code: String,
    user_code: String,
    #[serde(alias = "verification_url")]
    verification_uri: Url,
    #[serde(default)]
    verification_uri_complete: Option<Url>,
    expires_in: u64,
    #[serde(default)]
    interval: Option<u64>,
}

/// The token endpoint's answer: a token, or an error (RFC 6749, section 5).
#[derive(Deserialize)]
struct TokenAnswer {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

/// The user info answer's one field this needs.
#[derive(Deserialize)]
struct UserInfo {
    sub: String,
}

/// What one poll of the token endpoint said.
#[derive(Debug, PartialEq, Eq)]
enum Poll {
    /// The person signed in.
    Token(String),
    /// Not yet.
    Pending,
    /// Not yet, and poll less often.
    SlowDown,
    /// The grant is over, without a token.
    Failed(Failed),
}

/// How a grant ended without a token.
#[derive(Debug, PartialEq, Eq)]
enum Failed {
    Denied,
    Expired,
    Other(String),
}

impl From<Failed> for OauthError {
    fn from(failed: Failed) -> Self {
        match failed {
            Failed::Denied => Self::Denied,
            Failed::Expired => Self::Expired,
            Failed::Other(error) => Self::Refused(error),
        }
    }
}

/// Reads a token endpoint answer.
fn read_poll(answer: TokenAnswer) -> Poll {
    if let Some(token) = answer.access_token.filter(|token| !token.is_empty()) {
        return Poll::Token(token);
    }
    match answer.error.as_deref() {
        Some("authorization_pending") => Poll::Pending,
        Some("slow_down") => Poll::SlowDown,
        Some("access_denied") => Poll::Failed(Failed::Denied),
        Some("expired_token") => Poll::Failed(Failed::Expired),
        Some(error) => Poll::Failed(Failed::Other(match answer.error_description {
            Some(description) => format!("{error} ({description})"),
            None => error.to_owned(),
        })),
        None => Poll::Failed(Failed::Other("an answer with no token".to_owned())),
    }
}

/// Talks to one issuer as one public client.
#[derive(Clone, Debug)]
pub struct DeviceGrant {
    http: Client,
    client_id: String,
}

impl DeviceGrant {
    /// The grant for the public client `client_id`, over `http`.
    #[must_use]
    pub fn new(http: Client, client_id: String) -> Self {
        Self { http, client_id }
    }

    /// The endpoints `issuer`'s discovery document names, checked: the document is the
    /// issuer's own, it offers the device grant, and every endpoint is secure.
    ///
    /// # Errors
    ///
    /// [`OauthError`] when the document can't be had, is another issuer's, lacks the device
    /// grant, or names an insecure endpoint.
    pub async fn discover(&self, issuer: &Url) -> Result<Endpoints, OauthError> {
        let issuer = secure(issuer.clone())?;
        let mut url = issuer.clone();
        let path = format!(
            "{}/.well-known/openid-configuration",
            issuer.path().trim_end_matches('/')
        );
        url.set_path(&path);
        let request = self.http.get(url.clone());
        let (status, document): (_, Discovery) = self.json(&url, request).await?;
        if !status.is_success() {
            return Err(unexpected(&url, status, "no discovery document"));
        }
        if document.issuer.trim_end_matches('/') != issuer.as_str().trim_end_matches('/') {
            return Err(OauthError::OtherIssuer {
                expected: issuer,
                got: document.issuer,
            });
        }
        let device_authorization = document
            .device_authorization_endpoint
            .ok_or(OauthError::NoDeviceGrant)?;
        let userinfo = document
            .userinfo_endpoint
            .ok_or_else(|| unexpected(&url, status, "no user info endpoint"))?;
        Ok(Endpoints {
            device_authorization: secure(device_authorization)?,
            token: secure(document.token_endpoint)?,
            userinfo: secure(userinfo)?,
        })
    }

    /// Starts a grant: the code to show the person.
    ///
    /// # Errors
    ///
    /// [`OauthError`] when the issuer can't be reached or refuses.
    pub async fn start(&self, endpoints: &Endpoints) -> Result<DeviceAuthorization, OauthError> {
        let url = &endpoints.device_authorization;
        let form = form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", &self.client_id)
            .append_pair("scope", SCOPE)
            .finish();
        let request = self
            .http
            .post(url.clone())
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form);
        let (status, answer): (_, AuthorizationAnswer) = self.json(url, request).await?;
        if !status.is_success() {
            return Err(unexpected(url, status, "the grant didn't start"));
        }
        Ok(DeviceAuthorization {
            device_code: Zeroizing::new(answer.device_code),
            user_code: answer.user_code,
            verification_uri: answer.verification_uri,
            verification_uri_complete: answer.verification_uri_complete,
            expires_in: Duration::from_secs(answer.expires_in).min(MAX_EXPIRES_IN),
            interval: answer
                .interval
                .map_or(DEFAULT_INTERVAL, Duration::from_secs)
                .clamp(MIN_INTERVAL, MAX_INTERVAL),
        })
    }

    /// Polls until the person has signed in, declined, or the code has run out.
    ///
    /// # Errors
    ///
    /// [`OauthError::Denied`], [`OauthError::Expired`] (also when `expires_in` passes with no
    /// answer), or another refusal or failure.
    pub async fn wait_for_token(
        &self,
        endpoints: &Endpoints,
        authorization: &DeviceAuthorization,
    ) -> Result<AccountToken, OauthError> {
        let start = tokio::time::Instant::now();
        let deadline = start
            .checked_add(authorization.expires_in.min(MAX_EXPIRES_IN))
            .ok_or(OauthError::Expired)?;
        let mut interval = authorization.interval.clamp(MIN_INTERVAL, MAX_INTERVAL);
        let mut failures = 0;
        loop {
            let next = tokio::time::Instant::now()
                .checked_add(interval)
                .filter(|next| *next <= deadline)
                .ok_or(OauthError::Expired)?;
            tokio::time::sleep_until(next).await;
            let poll = match self.poll(endpoints, authorization).await {
                Ok(poll) => poll,
                Err(error) if error.is_transient() && failures < TRANSIENT_FAILURES => {
                    failures += 1;
                    tracing::warn!(%error, failures, "a poll for the sign-in failed; trying again");
                    continue;
                }
                Err(error) => return Err(error),
            };
            failures = 0;
            match poll {
                Poll::Token(token) => return Ok(AccountToken(Zeroizing::new(token))),
                Poll::Pending => {}
                Poll::SlowDown => interval = slower(interval),
                Poll::Failed(failed) => return Err(failed.into()),
            }
        }
    }

    /// The user id the token was issued for.
    ///
    /// # Errors
    ///
    /// [`OauthError`] when the user info endpoint can't be reached or names no user.
    pub async fn user_id(
        &self,
        endpoints: &Endpoints,
        token: &AccountToken,
    ) -> Result<String, OauthError> {
        let url = &endpoints.userinfo;
        let request = self
            .http
            .get(url.clone())
            .bearer_auth(token.expose_secret());
        let (status, info): (_, UserInfo) = self.json(url, request).await?;
        if !status.is_success() {
            return Err(unexpected(url, status, "no user for the token"));
        }
        Ok(info.sub)
    }

    /// One poll of the token endpoint.
    async fn poll(
        &self,
        endpoints: &Endpoints,
        authorization: &DeviceAuthorization,
    ) -> Result<Poll, OauthError> {
        let url = &endpoints.token;
        let form = Zeroizing::new(
            form_urlencoded::Serializer::new(String::new())
                .append_pair("grant_type", DEVICE_CODE_GRANT)
                .append_pair("device_code", &authorization.device_code)
                .append_pair("client_id", &self.client_id)
                .finish(),
        );
        let request = self
            .http
            .post(url.clone())
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form.as_str().to_owned());
        let (status, answer): (_, TokenAnswer) = self.json(url, request).await?;
        if status.is_server_error() {
            return Err(unexpected(url, status, "the token endpoint failed"));
        }
        Ok(read_poll(answer))
    }

    /// Sends `request` to `url` and reads its JSON answer, whatever its status.
    async fn json<T: DeserializeOwned>(
        &self,
        url: &Url,
        request: reqwest::RequestBuilder,
    ) -> Result<(StatusCode, T), OauthError> {
        let unreachable = |source| OauthError::Unreachable {
            url: url.clone(),
            source,
        };
        let response = request
            .header("accept", "application/json")
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(unreachable)?;
        let status = response.status();
        let body = read_body(response, ANSWER_LIMIT)
            .await
            .map_err(unreachable)?;
        let answer = serde_json::from_slice(&body.bytes)
            .map_err(|error| unexpected(url, status, &format!("not the JSON expected: {error}")))?;
        Ok((status, answer))
    }
}

/// The wait after a `slow_down`: five seconds longer, at most [`MAX_INTERVAL`].
fn slower(interval: Duration) -> Duration {
    interval
        .checked_add(SLOW_DOWN_STEP)
        .map_or(MAX_INTERVAL, |slower| slower.min(MAX_INTERVAL))
}

impl OauthError {
    /// Whether trying again may work: the issuer couldn't be reached, or failed (5xx).
    fn is_transient(&self) -> bool {
        match self {
            Self::Unreachable { .. } => true,
            Self::Unexpected { status, .. } => status.is_server_error(),
            _ => false,
        }
    }
}

/// An answer that isn't what the protocol says.
fn unexpected(url: &Url, status: StatusCode, detail: &str) -> OauthError {
    OauthError::Unexpected {
        url: url.clone(),
        status,
        detail: detail.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(json: &str) -> Poll {
        read_poll(serde_json::from_str(json).unwrap())
    }

    #[test]
    fn token_answers_are_read_as_rfc_8628_says() {
        assert_eq!(
            answer(r#"{"access_token":"t","token_type":"Bearer"}"#),
            Poll::Token("t".to_owned())
        );
        assert_eq!(
            answer(r#"{"error":"authorization_pending"}"#),
            Poll::Pending
        );
        assert_eq!(answer(r#"{"error":"slow_down"}"#), Poll::SlowDown);
        assert_eq!(
            answer(r#"{"error":"access_denied"}"#),
            Poll::Failed(Failed::Denied)
        );
        assert_eq!(
            answer(r#"{"error":"expired_token"}"#),
            Poll::Failed(Failed::Expired)
        );
        assert_eq!(
            answer(r#"{"error":"invalid_client","error_description":"unknown client"}"#),
            Poll::Failed(Failed::Other("invalid_client (unknown client)".to_owned()))
        );
        assert!(matches!(answer("{}"), Poll::Failed(Failed::Other(_))));
    }

    #[test]
    fn slowing_down_adds_five_seconds_up_to_a_minute() {
        assert_eq!(slower(Duration::from_secs(5)), Duration::from_secs(10));
        assert_eq!(slower(Duration::from_secs(58)), MAX_INTERVAL);
        assert_eq!(slower(Duration::MAX), MAX_INTERVAL);
    }

    #[test]
    fn only_failures_on_the_way_are_tried_again() {
        let url: Url = "https://accounts.example.com/token".parse().unwrap();
        assert!(unexpected(&url, StatusCode::BAD_GATEWAY, "x").is_transient());
        assert!(!unexpected(&url, StatusCode::BAD_REQUEST, "x").is_transient());
        assert!(!OauthError::Denied.is_transient());
        assert!(!OauthError::Expired.is_transient());
    }

    #[test]
    fn codes_and_tokens_go_over_https_or_to_loopback_only() {
        for good in [
            "https://accounts.example.com",
            "http://127.0.0.1:9000/token",
            "http://[::1]:9000/token",
            "http://localhost:9000",
        ] {
            assert!(is_secure(&good.parse().unwrap()), "{good}");
        }
        for bad in [
            "http://accounts.example.com",
            "http://192.0.2.1/token",
            "ftp://x",
        ] {
            assert!(
                matches!(secure(bad.parse().unwrap()), Err(OauthError::Insecure(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn secrets_never_show_in_debug() {
        let authorization = DeviceAuthorization {
            device_code: Zeroizing::new("device-secret".to_owned()),
            user_code: "WDJB-MJHT".to_owned(),
            verification_uri: "https://accounts.example.com/device".parse().unwrap(),
            verification_uri_complete: None,
            expires_in: Duration::from_secs(600),
            interval: DEFAULT_INTERVAL,
        };
        let shown = format!("{authorization:?}");
        assert!(
            !shown.contains("device-secret") && shown.contains("WDJB-MJHT"),
            "{shown}"
        );
        let token = AccountToken(Zeroizing::new("token-secret".to_owned()));
        assert_eq!(format!("{token:?}"), "AccountToken(<redacted>)");
    }
}
