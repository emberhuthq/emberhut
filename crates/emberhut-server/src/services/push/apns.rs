//! iPhone pushes through Apple's push service (APNs), over HTTP/2.
//!
//! Each request carries a provider token: a JWT signed with ES256 by the key Apple gives the
//! developer team (a `.p8` file), naming the key's id and the team. Apple takes a token for an
//! hour, so one is made every 50 minutes. The alert goes to `/3/device/<token>` on the sandbox
//! or the production host, as the phone registered, for the app's bundle id (the topic).
//!
//! Setup: the key's file in the server's secrets directory ([`KEY_FILE`]), and the key id, the
//! team id and the app's bundle id in the configuration.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Bytes;
use axum::http::{Method, Request, header};
use emberhut_core::Clock;
use emberhut_core::push::{Alert, ApnsDevice, ApnsEnv, apns_token_gone};
use futures_util::future::BoxFuture;
use jiff::{SignedDuration, Timestamp};
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair};
use serde_json::{Value, json};

use super::http::{HttpClient, HttpError};
use super::keys::{KeyError, KeyFile, base64url, pkcs8_der, signing_input};
use super::{ApnsSender, Delivery};
use crate::error::chain;
use crate::http::Protocol;

/// The key's file name in the secrets directory.
pub const KEY_FILE: &str = "apns-key.p8";

/// Apple's service for development builds.
pub const SANDBOX_HOST: &str = "https://api.sandbox.push.apple.com";

/// Apple's service for release builds.
pub const PRODUCTION_HOST: &str = "https://api.push.apple.com";

/// How long a provider token is used before a new one is made (Apple takes one for an hour).
pub const TOKEN_RENEWAL: SignedDuration = SignedDuration::from_mins(50);

/// What the APNs sender needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApnsConfig {
    /// The `.p8` key's file.
    pub key_file: PathBuf,
    /// The key's id (ten characters, from Apple's developer site).
    pub key_id: String,
    /// The developer team's id.
    pub team_id: String,
    /// The iPhone app's bundle id: the topic of every alert.
    pub topic: String,
}

/// Why the APNs sender could not be made.
#[derive(Debug, thiserror::Error)]
#[error("the APNs client could not be made")]
pub struct ApnsSetupError(#[source] HttpError);

/// The real APNs sender.
#[derive(Debug)]
pub struct HttpApns {
    config: ApnsConfig,
    key: KeyFile,
    client: HttpClient,
    clock: Arc<dyn Clock>,
    token: Mutex<Option<(String, Timestamp)>>,
}

impl HttpApns {
    /// A sender with `config`, telling the time (for its tokens) with `clock`.
    ///
    /// # Errors
    ///
    /// [`ApnsSetupError`] when its HTTPS client cannot be made.
    pub fn new(config: ApnsConfig, clock: Arc<dyn Clock>) -> Result<Self, ApnsSetupError> {
        Ok(Self {
            key: KeyFile::new(config.key_file.clone()),
            config,
            client: HttpClient::new(Protocol::Http2Only).map_err(ApnsSetupError)?,
            clock,
            token: Mutex::new(None),
        })
    }

    /// The provider token: the one made within [`TOKEN_RENEWAL`], or a new one from the key's
    /// file ([`KeyFile`]: a key put there later is used without a restart).
    async fn provider_token(&self) -> Result<String, KeyError> {
        let now = self.clock.now();
        if let Some((token, made)) = self.kept_token().as_ref()
            && now - *made < TOKEN_RENEWAL
        {
            return Ok(token.clone());
        }
        let key = self.key.text().await?;
        let token = provider_token(&key, &self.config.key_id, &self.config.team_id, now)
            .inspect_err(|_| self.key.forget())?;
        *self.kept_token() = Some((token.clone(), now));
        Ok(token)
    }

    fn kept_token(&self) -> std::sync::MutexGuard<'_, Option<(String, Timestamp)>> {
        self.token.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn deliver(&self, device: &ApnsDevice, alert: &Alert) -> Delivery {
        let token = match self.provider_token().await {
            Ok(token) => token,
            Err(error) => return Delivery::Failed(format!("no provider token: {error}")),
        };
        let request = match request(device, &self.config.topic, &token, &payload(alert)) {
            Ok(request) => request,
            Err(error) => return Delivery::Failed(format!("not a request: {error}")),
        };
        match self.client.send(request).await {
            Ok(answer) if answer.status.is_success() => Delivery::Delivered,
            Ok(answer) if apns_token_gone(answer.status.as_u16(), &answer.text) => Delivery::Gone,
            Ok(answer) => {
                if answer.text.contains("ExpiredProviderToken") {
                    // The next push makes a new one.
                    *self.kept_token() = None;
                }
                Delivery::Failed(format!("APNs answered {}: {}", answer.status, answer.text))
            }
            Err(error) => Delivery::Failed(chain(&error)),
        }
    }
}

impl ApnsSender for HttpApns {
    fn ready(&self) -> BoxFuture<'_, bool> {
        Box::pin(async { self.key.text().await.is_ok() })
    }

    fn send<'a>(&'a self, device: &'a ApnsDevice, alert: &'a Alert) -> BoxFuture<'a, Delivery> {
        Box::pin(self.deliver(device, alert))
    }
}

/// A provider token: `{alg: ES256, kid}` and `{iss: team, iat}`, signed with the `.p8` key
/// in `key_pem`.
///
/// # Errors
///
/// [`KeyError`] when the key is not a P-256 PKCS #8 key, or signing fails.
pub fn provider_token(
    key_pem: &str,
    key_id: &str,
    team_id: &str,
    issued_at: Timestamp,
) -> Result<String, KeyError> {
    let random = SystemRandom::new();
    let key = EcdsaKeyPair::from_pkcs8(
        &ECDSA_P256_SHA256_FIXED_SIGNING,
        &pkcs8_der(key_pem)?,
        &random,
    )
    .map_err(|rejected| KeyError::Rejected(rejected.to_string()))?;
    let signed = signing_input(
        &json!({ "alg": "ES256", "kid": key_id }),
        &json!({ "iss": team_id, "iat": issued_at.as_second() }),
    );
    let signature = key
        .sign(&random, signed.as_bytes())
        .map_err(|_| KeyError::Sign)?;
    Ok(format!("{signed}.{}", base64url(signature.as_ref())))
}

/// What Apple is sent for `alert`: the alert with the default sound, grouped by its thread,
/// and what a tap opens (`link`, and `room` when there is one).
#[must_use]
pub fn payload(alert: &Alert) -> Value {
    let mut payload = json!({
        "aps": {
            "alert": { "title": alert.title(), "body": alert.body() },
            "sound": "default",
            "thread-id": alert.thread(),
        },
        "link": alert.link(),
    });
    if let Some(room) = alert.room() {
        payload["room"] = json!(room.get());
    }
    payload
}

/// The host for a phone's service.
#[must_use]
pub fn host(env: ApnsEnv) -> &'static str {
    match env {
        ApnsEnv::Sandbox => SANDBOX_HOST,
        ApnsEnv::Production => PRODUCTION_HOST,
    }
}

/// The request that pushes `payload` to `device`, for the app `topic`, with the provider
/// token `token`.
///
/// # Errors
///
/// [`axum::http::Error`] when a part can't go in a request (a token or topic with a line break).
pub fn request(
    device: &ApnsDevice,
    topic: &str,
    token: &str,
    payload: &Value,
) -> Result<Request<Bytes>, axum::http::Error> {
    Request::builder()
        .method(Method::POST)
        .uri(format!("{}/3/device/{}", host(device.env), device.token))
        .version(axum::http::Version::HTTP_2)
        .header(header::AUTHORIZATION, format!("bearer {token}"))
        .header("apns-topic", topic)
        .header("apns-push-type", "alert")
        .header("apns-priority", "10")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Bytes::from(payload.to_string()))
}
