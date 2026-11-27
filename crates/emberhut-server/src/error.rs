//! Errors as the API answers them: a status, and a JSON body whose `error` is a sentence the
//! app can show as it is.
//!
//! ```json
//! { "error": "not found" }
//! { "error": "Sign in first.", "signIn": true }
//! ```
//!
//! The bodies are emberhut-api's [`ErrorBody`]. A refusal from the sign-in routes (a pairing code
//! or a refresh token not taken) is their [`AuthReply`] instead, with `ok` false and the status
//! repeated:
//!
//! ```json
//! { "ok": false, "status": 401, "error": "That code has expired. Make a new one." }
//! ```
//!
//! Server faults (5xx) are logged with their whole chain of causes and answered with a fixed
//! sentence, so nothing about the server's insides reaches a client.

use std::borrow::Cow;
use std::fmt;

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use emberhut_api::auth::AuthReply;
use emberhut_api::error::ErrorBody;
use emberhut_core::pairing::PairingRefusal;
use jiff::SignedDuration;

pub use crate::extract::Invalid;
use crate::services::agent_provider::ProviderError;
use crate::services::auth::AuthError;
use crate::services::backup::BackupError;
use crate::services::mac::MacError;
use crate::services::pairing::PairingError;
use crate::services::state::StateError;
use crate::services::talk::TalkError;
use crate::services::uploads::UploadError;

/// The sentence for every server fault.
const FAULT: &str = "Something went wrong on the server.";

/// What a failed request answers.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApiError {
    /// The request needs a signed-in device and has none (401, with `signIn: true`).
    #[error("sign-in required")]
    SignInRequired,
    /// No route has this path (404).
    #[error("no such route")]
    NotFound,
    /// The request took longer than the server allows (503).
    #[error("request timed out")]
    Timeout,
    /// The server is shutting down, and takes no new work (503).
    #[error("the server is shutting down")]
    ShuttingDown,
    /// Something unexpected went wrong in the server, such as a panicking handler (500). The
    /// cause is logged where it happens.
    #[error("unexpected server fault")]
    Internal,
    /// A session could not be renewed or opened.
    #[error(transparent)]
    Auth(#[from] AuthError),
    /// A pairing code was not taken or could not be made.
    #[error(transparent)]
    Pairing(#[from] PairingError),
    /// A request that only a signed-in device may make came without a session (401, with
    /// `ok: false`), in either auth mode.
    #[error("a signed-in device is required")]
    DeviceRequired,
    /// The request's body is not one the route takes (400, or 413 when it is too large).
    #[error("invalid request")]
    Invalid(#[source] Invalid),
    /// A sign-in route's request is not one it takes (400 or 413, in that route's
    /// `{ok, status, error}` shape).
    #[error("invalid sign-in request")]
    InvalidSignIn(#[source] Invalid),
    /// A file was not stored (400, `{ ok: false, error }`), or a stored one could not be
    /// served (500).
    #[error(transparent)]
    Upload(#[from] UploadError),
    /// The state could not be built.
    #[error(transparent)]
    State(#[from] StateError),
    /// A message could not be said.
    #[error(transparent)]
    Talk(#[from] TalkError),
    /// An answer could not be written as JSON.
    #[error("could not write the answer")]
    Encode(#[source] serde_json::Error),
    /// The Mac helper did not do what was asked (400, with its reason).
    #[error(transparent)]
    Mac(#[from] MacError),
    /// A backup could not start (500).
    #[error(transparent)]
    Backup(#[from] BackupError),
    /// The agent's model provider was not taken: not what it takes (400), its key refused
    /// (422), or not checked because it didn't answer (502); or it could not be kept (500).
    #[error(transparent)]
    Provider(#[from] ProviderError),
}

impl ApiError {
    /// The HTTP status this error answers with.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            Self::SignInRequired
            | Self::DeviceRequired
            | Self::Auth(AuthError::RefreshRejected) => StatusCode::UNAUTHORIZED,
            Self::Invalid(Invalid::TooLarge) | Self::InvalidSignIn(Invalid::TooLarge) => {
                StatusCode::PAYLOAD_TOO_LARGE
            }
            Self::Upload(UploadError::Database(_)) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Upload(_)
            | Self::Invalid(_)
            | Self::InvalidSignIn(_)
            | Self::Mac(_)
            | Self::Talk(TalkError::Blank)
            | Self::Provider(ProviderError::Malformed(_)) => StatusCode::BAD_REQUEST,
            Self::Provider(ProviderError::Refused(_)) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Provider(ProviderError::Unreachable { .. }) => StatusCode::BAD_GATEWAY,
            Self::Talk(TalkError::Busy) => StatusCode::TOO_MANY_REQUESTS,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Timeout
            | Self::ShuttingDown
            | Self::Talk(TalkError::ShuttingDown)
            | Self::Backup(BackupError::Busy) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Pairing(PairingError::Refused(refusal)) => match refusal {
                PairingRefusal::TooManyWrongCodes { .. } => StatusCode::TOO_MANY_REQUESTS,
                PairingRefusal::Missing => StatusCode::BAD_REQUEST,
                _ => StatusCode::UNAUTHORIZED,
            },
            Self::Internal
            | Self::Auth(_)
            | Self::Pairing(_)
            | Self::State(_)
            | Self::Talk(_)
            | Self::Encode(_)
            | Self::Backup(_)
            | Self::Provider(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Whether this is a sign-in route's refusal, answered in that route's own shape.
    fn is_sign_in_refusal(&self) -> bool {
        matches!(
            self,
            Self::Auth(AuthError::RefreshRejected)
                | Self::Pairing(PairingError::Refused(_))
                | Self::InvalidSignIn(_)
        )
    }

    /// The sentence the app shows.
    fn message(&self) -> Cow<'static, str> {
        Cow::Borrowed(match self {
            Self::SignInRequired => return ErrorBody::sign_in_required().error.into(),
            Self::DeviceRequired => "Only a signed-in device can make a pairing code.",
            Self::Invalid(invalid) | Self::InvalidSignIn(invalid) => {
                return invalid.reason().into();
            }
            Self::Upload(UploadError::TooLarge) => "file is larger than 500 MB",
            Self::Upload(UploadError::Interrupted(_)) => {
                "The file did not arrive whole. Try again."
            }
            Self::Upload(UploadError::Database(_)) => FAULT,
            Self::Upload(_) => "The file could not be stored.",
            Self::Talk(TalkError::Blank) => "Say something.",
            Self::Talk(TalkError::Busy) => {
                "Too many messages are waiting for an answer in this room. Try again in a moment."
            }
            Self::ShuttingDown | Self::Talk(TalkError::ShuttingDown) => {
                "The server is restarting. Try again in a moment."
            }
            Self::NotFound => "not found",
            Self::Timeout => "The server took too long to answer. Try again.",
            Self::Auth(AuthError::RefreshRejected) => "Sign in again.",
            Self::Pairing(PairingError::Refused(refusal)) => return refusal_message(*refusal),
            Self::Mac(error) => return error.to_string().into(),
            Self::Backup(BackupError::Busy) => {
                "A backup is being made already. Try again when it is done."
            }
            Self::Provider(ProviderError::Malformed(why) | ProviderError::Refused(why)) => {
                return why.clone().into();
            }
            Self::Provider(ProviderError::Unreachable { provider, .. }) => {
                return format!(
                    "{provider} couldn't be reached to check it, so nothing was saved. Check \
                     the address, or try again in a moment."
                )
                .into();
            }
            Self::Internal
            | Self::Auth(_)
            | Self::Pairing(_)
            | Self::State(_)
            | Self::Talk(_)
            | Self::Encode(_)
            | Self::Backup(_)
            | Self::Provider(_) => FAULT,
        })
    }

    /// How long a client should wait before trying again, for a `Retry-After` header.
    fn retry_after(&self) -> Option<SignedDuration> {
        match self {
            Self::Pairing(PairingError::Refused(PairingRefusal::TooManyWrongCodes {
                retry_after,
            })) => Some(*retry_after),
            _ => None,
        }
    }
}

fn refusal_message(refusal: PairingRefusal) -> Cow<'static, str> {
    Cow::Borrowed(match refusal {
        PairingRefusal::TooManyWrongCodes { retry_after } => {
            let minutes = whole_minutes(retry_after);
            let unit = if minutes == 1 { "minute" } else { "minutes" };
            return Cow::Owned(format!(
                "Too many wrong codes. Try again in {minutes} {unit}."
            ));
        }
        PairingRefusal::Missing => "Type the pairing code.",
        PairingRefusal::Used => "That code was already used. Make a new one.",
        PairingRefusal::Expired => "That code has expired. Make a new one.",
        // `Unknown`, and any refusal added later: the code did not work.
        _ => "That code isn't right. Check it, or make a new one.",
    })
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        if status.is_server_error()
            || matches!(
                self,
                Self::Upload(UploadError::Storage(_) | UploadError::Random(_))
            )
        {
            tracing::error!(error = %CauseChain(&self), "request failed");
        }
        let error = self.message().into_owned();
        let mut response = if self.is_sign_in_refusal() {
            (status, Json(AuthReply::refused(status.as_u16(), error))).into_response()
        } else {
            let body = ErrorBody {
                // A refused upload says `ok: false` too.
                ok: (matches!(self, Self::DeviceRequired)
                    || matches!(self, Self::Upload(_)) && !status.is_server_error())
                .then_some(false),
                error,
                sign_in: matches!(self, Self::SignInRequired),
            };
            (status, Json(body)).into_response()
        };
        if let Some(wait) = self.retry_after() {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(whole_seconds(wait)));
        }
        response
    }
}

/// `wait` in seconds, rounded up, at least one.
fn whole_seconds(wait: SignedDuration) -> i64 {
    let seconds = wait.as_secs() + i64::from(wait.subsec_nanos() > 0);
    seconds.max(1)
}

/// `wait` in minutes, rounded up, at least one.
fn whole_minutes(wait: SignedDuration) -> i64 {
    (whole_seconds(wait) + 59) / 60
}

/// An error and its sources, joined with ": ", for a log line (`%CauseChain(&error)`).
pub(crate) struct CauseChain<'a>(pub(crate) &'a dyn std::error::Error);

impl fmt::Display for CauseChain<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)?;
        let mut source = self.0.source();
        while let Some(cause) = source {
            write!(f, ": {cause}")?;
            source = cause.source();
        }
        Ok(())
    }
}

/// `error` and its causes, joined with ": ": [`CauseChain`] as text.
pub(crate) fn chain(error: &dyn std::error::Error) -> String {
    CauseChain(error).to_string()
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use emberhut_core::RandomError;
    use http_body_util::BodyExt as _;
    use serde_json::{Value, json};

    use super::*;

    async fn answer(error: ApiError) -> (StatusCode, Option<HeaderValue>, Value) {
        let response = error.into_response();
        let status = response.status();
        let retry_after = response.headers().get(header::RETRY_AFTER).cloned();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, retry_after, serde_json::from_slice(&bytes).unwrap())
    }

    fn refused(refusal: PairingRefusal) -> ApiError {
        PairingError::Refused(refusal).into()
    }

    #[tokio::test]
    async fn sign_in_required_says_so() {
        let (status, _, body) = answer(ApiError::SignInRequired).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body, json!({ "error": "Sign in first.", "signIn": true }));
    }

    #[tokio::test]
    async fn only_a_device_may_make_a_code() {
        let (status, _, body) = answer(ApiError::DeviceRequired).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            body,
            json!({ "ok": false, "error": "Only a signed-in device can make a pairing code." })
        );
    }

    fn shape_error() -> Invalid {
        Invalid::Shape(serde_json::from_str::<u8>("\"x\"").unwrap_err())
    }

    #[tokio::test]
    async fn invalid_requests_say_why_and_keep_their_cause() {
        let error = ApiError::Invalid(shape_error());
        assert!(error.source().is_some(), "the cause is in the chain");
        let (status, _, body) = answer(error).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .starts_with("Not a request this route takes")
        );

        let (status, _, body) = answer(ApiError::InvalidSignIn(shape_error())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["ok"], json!(false));
        assert_eq!(body["status"], json!(400));
    }

    #[tokio::test]
    async fn a_body_over_the_limit_is_413() {
        let (status, _, body) = answer(ApiError::Invalid(Invalid::TooLarge)).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(body, json!({ "error": "The request is too large." }));
        let (status, _, body) = answer(ApiError::InvalidSignIn(Invalid::TooLarge)).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            body,
            json!({ "ok": false, "status": 413, "error": "The request is too large." })
        );
    }

    #[tokio::test]
    async fn talk_refusals_answer_with_the_error_body() {
        for (error, status) in [
            (TalkError::Blank, StatusCode::BAD_REQUEST),
            (TalkError::Busy, StatusCode::TOO_MANY_REQUESTS),
            (TalkError::ShuttingDown, StatusCode::SERVICE_UNAVAILABLE),
        ] {
            let error = ApiError::Talk(error);
            let (got, _, body) = answer(error).await;
            assert_eq!(got, status);
            assert!(body["error"].as_str().is_some_and(|e| !e.is_empty()));
            assert_eq!(body.as_object().unwrap().len(), 1, "only `error`");
        }
    }

    #[tokio::test]
    async fn shutting_down_is_503_with_its_sentence() {
        let (status, _, body) = answer(ApiError::ShuttingDown).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            body,
            json!({ "error": "The server is restarting. Try again in a moment." })
        );
    }

    #[tokio::test]
    async fn a_provider_not_taken_says_why_in_words() {
        for (error, status, said) in [
            (
                ProviderError::Malformed("Anthropic needs an API key.".to_owned()),
                StatusCode::BAD_REQUEST,
                "Anthropic needs an API key.",
            ),
            (
                ProviderError::Refused("OpenAI didn't accept that key.".to_owned()),
                StatusCode::UNPROCESSABLE_ENTITY,
                "OpenAI didn't accept that key.",
            ),
            (
                ProviderError::Unreachable {
                    provider: "Ollama",
                    why: "connection refused at 127.0.0.1:11434".to_owned(),
                },
                StatusCode::BAD_GATEWAY,
                "Ollama couldn't be reached to check it, so nothing was saved. Check the \
                 address, or try again in a moment.",
            ),
        ] {
            let (got, _, body) = answer(error.into()).await;
            assert_eq!(got, status);
            assert_eq!(body, json!({ "error": said }), "the cause stays in the log");
        }
        let storage = ProviderError::Storage(std::io::Error::other("disk full"));
        let (got, _, body) = answer(storage.into()).await;
        assert_eq!(got, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, json!({ "error": FAULT }));
    }

    #[tokio::test]
    async fn not_found_has_only_an_error() {
        let (status, _, body) = answer(ApiError::NotFound).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, json!({ "error": "not found" }));
    }

    #[tokio::test]
    async fn pairing_refusals_answer_with_their_sentence() {
        let cases = [
            (PairingRefusal::Missing, 400, "Type the pairing code."),
            (
                PairingRefusal::Unknown,
                401,
                "That code isn't right. Check it, or make a new one.",
            ),
            (
                PairingRefusal::Used,
                401,
                "That code was already used. Make a new one.",
            ),
            (
                PairingRefusal::Expired,
                401,
                "That code has expired. Make a new one.",
            ),
        ];
        for (refusal, status, sentence) in cases {
            let (got, _, body) = answer(refused(refusal)).await;
            assert_eq!(got.as_u16(), status, "{refusal:?}");
            assert_eq!(
                body,
                json!({ "ok": false, "status": status, "error": sentence })
            );
        }
    }

    #[tokio::test]
    async fn too_many_wrong_codes_says_how_long_to_wait() {
        let error = refused(PairingRefusal::TooManyWrongCodes {
            retry_after: SignedDuration::from_secs(9 * 60 + 1),
        });
        let (status, retry_after, body) = answer(error).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(retry_after, Some(HeaderValue::from(541)));
        assert_eq!(
            body,
            json!({
                "ok": false,
                "status": 429,
                "error": "Too many wrong codes. Try again in 10 minutes."
            })
        );

        let error = refused(PairingRefusal::TooManyWrongCodes {
            retry_after: SignedDuration::from_millis(1),
        });
        let (_, retry_after, body) = answer(error).await;
        assert_eq!(retry_after, Some(HeaderValue::from(1)));
        assert_eq!(
            body["error"],
            json!("Too many wrong codes. Try again in 1 minute.")
        );
    }

    #[tokio::test]
    async fn refused_refresh_asks_to_sign_in_again() {
        let (status, _, body) = answer(AuthError::RefreshRejected.into()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            body,
            json!({ "ok": false, "status": 401, "error": "Sign in again." })
        );
    }

    #[tokio::test]
    async fn server_faults_do_not_leak_their_cause() {
        let cause = std::io::Error::other("entropy pool at /dev/secret is empty");
        let fault = AuthError::Random(RandomError::new(cause));
        let (status, _, body) = answer(fault.into()).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, json!({ "error": FAULT }));

        let (status, _, body) = answer(ApiError::Timeout).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            body,
            json!({ "error": "The server took too long to answer. Try again." })
        );
    }

    #[test]
    fn cause_chain_names_every_source() {
        let cause = std::io::Error::other("no entropy");
        let error = ApiError::from(AuthError::Random(RandomError::new(cause)));
        assert_eq!(
            CauseChain(&error).to_string(),
            "could not make tokens: no random bytes could be had: no entropy"
        );
    }
}
