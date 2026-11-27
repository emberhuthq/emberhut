//! Android pushes through Firebase Cloud Messaging (the FCM HTTP v1 API).
//!
//! FCM takes an OAuth access token, which Google's token endpoint gives for a short JWT
//! signed (RS256) with the Firebase project's service account key. The token is kept until
//! five minutes before it expires, and dropped when FCM refuses it.
//!
//! Setup: the service account's key file (Firebase console, Project settings, Service
//! accounts, "Generate new private key") in the server's secrets directory ([`ACCOUNT_FILE`]).
//! The file is read off the async workers and kept for a minute, so one put there later is used
//! without a restart.

use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Bytes;
use axum::http::{Method, Request, StatusCode, header};
use emberhut_api::push::ANDROID_CHANNEL;
use emberhut_core::Clock;
use emberhut_core::push::{Alert, fcm_token_gone};
use futures_util::future::BoxFuture;
use jiff::{SignedDuration, Timestamp};
use ring::rand::SystemRandom;
use ring::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use serde::Deserialize;
use serde_json::{Value, json};

use super::http::{HttpClient, HttpError};
use super::keys::{KeyError, KeyFile, base64url, pkcs8_der, signing_input};
use super::{Delivery, FcmSender};
use crate::error::chain;
use crate::http::Protocol;

/// The service account's file name in the secrets directory.
pub const ACCOUNT_FILE: &str = "fcm-service-account.json";

/// The scope FCM's access tokens need.
pub const MESSAGING_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";

/// The grant type of a JWT traded for an access token (RFC 7523).
const JWT_BEARER_GRANT: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";

/// How long the signed assertion is valid: the longest Google takes.
const ASSERTION_LIFETIME: SignedDuration = SignedDuration::from_hours(1);

/// An access token's lifetime when Google's answer doesn't give one.
const DEFAULT_TOKEN_LIFETIME: SignedDuration = SignedDuration::from_hours(1);

/// How long before its expiry an access token is renewed.
const RENEW_BEFORE_EXPIRY: SignedDuration = SignedDuration::from_mins(5);

/// The longest refusal text kept for a log line.
const LOGGED_REPLY_LIMIT: usize = 300;

/// Where the sender gets tokens and sends messages: Google's, or a stand-in's in tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FcmEndpoints {
    /// The OAuth token endpoint, which is also the assertion's audience.
    pub token_url: String,
    /// FCM's base URL; messages go to `<base>/v1/projects/<project>/messages:send`.
    pub send_base: String,
}

impl Default for FcmEndpoints {
    /// Google's.
    fn default() -> Self {
        Self {
            token_url: "https://oauth2.googleapis.com/token".to_owned(),
            send_base: "https://fcm.googleapis.com".to_owned(),
        }
    }
}

impl FcmEndpoints {
    /// Where messages for `project` go.
    #[must_use]
    pub fn send_url(&self, project: &str) -> String {
        format!("{}/v1/projects/{project}/messages:send", self.send_base)
    }
}

/// The parts of a Google service account key file that FCM needs.
#[derive(Clone, Deserialize, PartialEq, Eq)]
pub struct ServiceAccount {
    /// The Firebase project.
    pub project_id: String,
    /// The account's address: the assertion's issuer.
    pub client_email: String,
    /// The key's id, named in the assertion's header.
    #[serde(default)]
    pub private_key_id: String,
    /// The account's RSA key, PEM (PKCS #8).
    pub private_key: String,
}

impl fmt::Debug for ServiceAccount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceAccount")
            .field("project_id", &self.project_id)
            .field("client_email", &self.client_email)
            .field("private_key", &"..")
            .finish_non_exhaustive()
    }
}

impl ServiceAccount {
    /// The account in a key file's text, when it has what FCM needs (a project, an address
    /// and a key, none of them empty).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let account: Self = serde_json::from_str(text).ok()?;
        let complete = !account.project_id.is_empty()
            && !account.client_email.is_empty()
            && !account.private_key.is_empty();
        complete.then_some(account)
    }
}

/// An access token, whose account it is for, and when to get a new one.
#[derive(Clone, Debug)]
struct KeptToken {
    value: String,
    client_email: String,
    renew_at: Timestamp,
}

/// Why no access token was had.
#[derive(Debug, thiserror::Error)]
enum TokenError {
    #[error("no assertion")]
    Assertion(#[source] KeyError),
    #[error("not a request")]
    Request(#[source] axum::http::Error),
    #[error("the token endpoint was not reached")]
    Http(#[source] HttpError),
    #[error("Google's token endpoint answered {0}")]
    Refused(String),
}

/// Why the FCM sender could not be made.
#[derive(Debug, thiserror::Error)]
#[error("the FCM client could not be made")]
pub struct FcmSetupError(#[source] HttpError);

/// The real FCM sender.
#[derive(Debug)]
pub struct HttpFcm {
    account_file: KeyFile,
    endpoints: FcmEndpoints,
    client: HttpClient,
    clock: Arc<dyn Clock>,
    token: Mutex<Option<KeptToken>>,
}

impl HttpFcm {
    /// A sender with the service account in `account_file`, using `endpoints` (Google's, by
    /// [`FcmEndpoints::default`]), telling the time with `clock`.
    ///
    /// # Errors
    ///
    /// [`FcmSetupError`] when its HTTPS client cannot be made.
    pub fn new(
        account_file: PathBuf,
        endpoints: FcmEndpoints,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, FcmSetupError> {
        Ok(Self {
            account_file: KeyFile::new(account_file),
            endpoints,
            client: HttpClient::new(Protocol::Any).map_err(FcmSetupError)?,
            clock,
            token: Mutex::new(None),
        })
    }

    async fn account(&self) -> Option<ServiceAccount> {
        ServiceAccount::parse(&self.account_file.text().await.ok()?)
    }

    fn kept(&self) -> std::sync::MutexGuard<'_, Option<KeptToken>> {
        self.token.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// An access token for `account`: the one kept, or a new one when it is about to expire.
    async fn access_token(&self, account: &ServiceAccount) -> Result<String, TokenError> {
        let now = self.clock.now();
        if let Some(kept) = self.kept().as_ref()
            && kept.client_email == account.client_email
            && now < kept.renew_at
        {
            return Ok(kept.value.clone());
        }
        let assertion =
            assertion(account, &self.endpoints.token_url, now).map_err(TokenError::Assertion)?;
        let form = form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", JWT_BEARER_GRANT)
            .append_pair("assertion", &assertion)
            .finish();
        let request = Request::builder()
            .method(Method::POST)
            .uri(&self.endpoints.token_url)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Bytes::from(form))
            .map_err(TokenError::Request)?;
        let answer = self.client.send(request).await.map_err(TokenError::Http)?;
        let granted: Option<Granted> = answer
            .status
            .is_success()
            .then(|| serde_json::from_str(&answer.text).ok())
            .flatten();
        let Some(granted) = granted.filter(|granted| !granted.access_token.is_empty()) else {
            // Only a refusal's text is logged: a success carries the token.
            let said = if answer.status.is_success() {
                answer.status.to_string()
            } else {
                format!("{}: {}", answer.status, cut(&answer.text))
            };
            return Err(TokenError::Refused(said));
        };
        let lifetime = granted
            .expires_in
            .filter(|seconds| *seconds > 0)
            .map_or(DEFAULT_TOKEN_LIFETIME, SignedDuration::from_secs);
        *self.kept() = Some(KeptToken {
            value: granted.access_token.clone(),
            client_email: account.client_email.clone(),
            renew_at: now + lifetime - RENEW_BEFORE_EXPIRY,
        });
        Ok(granted.access_token)
    }

    async fn deliver(&self, token: &str, alert: &Alert) -> Delivery {
        let Some(account) = self.account().await else {
            return Delivery::Failed(format!("no usable {ACCOUNT_FILE}"));
        };
        let bearer = match self.access_token(&account).await {
            Ok(bearer) => bearer,
            Err(error) => {
                if matches!(error, TokenError::Assertion(_)) {
                    // The key was refused: read the file again next time.
                    self.account_file.forget();
                }
                return Delivery::Failed(chain(&error));
            }
        };
        let request = Request::builder()
            .method(Method::POST)
            .uri(self.endpoints.send_url(&account.project_id))
            .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Bytes::from(message(token, alert).to_string()));
        let request = match request {
            Ok(request) => request,
            Err(error) => return Delivery::Failed(format!("not a request: {error}")),
        };
        match self.client.send(request).await {
            Ok(answer) if answer.status.is_success() => Delivery::Delivered,
            Ok(answer) if fcm_token_gone(&answer.text) => Delivery::Gone,
            Ok(answer) => {
                if answer.status == StatusCode::UNAUTHORIZED {
                    // The next push gets a new one.
                    *self.kept() = None;
                }
                Delivery::Failed(format!(
                    "FCM answered {}: {}",
                    answer.status,
                    cut(&answer.text)
                ))
            }
            Err(error) => Delivery::Failed(chain(&error)),
        }
    }
}

impl FcmSender for HttpFcm {
    fn ready(&self) -> BoxFuture<'_, bool> {
        Box::pin(async { self.account().await.is_some() })
    }

    fn send<'a>(&'a self, token: &'a str, alert: &'a Alert) -> BoxFuture<'a, Delivery> {
        Box::pin(self.deliver(token, alert))
    }
}

/// What Google's token endpoint answers.
#[derive(Deserialize)]
struct Granted {
    #[serde(default)]
    access_token: String,
    expires_in: Option<i64>,
}

/// The JWT Google's token endpoint trades for an access token: `{alg: RS256, typ: JWT, kid}`
/// and `{iss, scope, aud, iat, exp}`, signed with the service account's key.
///
/// # Errors
///
/// [`KeyError`] when the key is not an RSA PKCS #8 key, or signing fails.
pub fn assertion(
    account: &ServiceAccount,
    audience: &str,
    issued_at: Timestamp,
) -> Result<String, KeyError> {
    let key = RsaKeyPair::from_pkcs8(&pkcs8_der(&account.private_key)?)
        .map_err(|rejected| KeyError::Rejected(rejected.to_string()))?;
    let signed = signing_input(
        &json!({ "alg": "RS256", "typ": "JWT", "kid": account.private_key_id }),
        &json!({
            "iss": account.client_email,
            "scope": MESSAGING_SCOPE,
            "aud": audience,
            "iat": issued_at.as_second(),
            "exp": (issued_at + ASSERTION_LIFETIME).as_second(),
        }),
    );
    let mut signature = vec![0; key.public().modulus_len()];
    key.sign(
        &RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        signed.as_bytes(),
        &mut signature,
    )
    .map_err(|_| KeyError::Sign)?;
    Ok(format!("{signed}.{}", base64url(&signature)))
}

/// FCM's message for one phone. The notification shows by itself while the app is closed;
/// `data` carries what a tap opens (`link`), the room's id when there is one, and the room's
/// group (`thread`). FCM's data values are strings only.
#[must_use]
pub fn message(token: &str, alert: &Alert) -> Value {
    let mut data = json!({ "link": alert.link(), "thread": alert.thread() });
    if let Some(room) = alert.room() {
        data["room"] = json!(room.to_string());
    }
    json!({
        "message": {
            "token": token,
            "notification": { "title": alert.title(), "body": alert.body() },
            "data": data,
            "android": {
                "priority": "high",
                "notification": { "channel_id": ANDROID_CHANNEL, "sound": "default" },
            },
        },
    })
}

/// The start of `text`, for a log line.
fn cut(text: &str) -> &str {
    match text.char_indices().nth(LOGGED_REPLY_LIMIT) {
        Some((at, _)) => &text[..at],
        None => text,
    }
}
