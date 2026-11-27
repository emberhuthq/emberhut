//! What handlers take from a request: a validated body, the room of a query, and who is asking.

use std::net::SocketAddr;

use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, FailedToBufferBody};
use axum::extract::{ConnectInfo, FromRequest, FromRequestParts, Query, Request};
use axum::http::header;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use emberhut_core::{DeviceId, RoomId};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::app::AppState;
use crate::config::AuthMode;
use crate::error::ApiError;

/// A JSON body of type `T` that passed its validation rules (emberhut-api's `garde` rules).
///
/// An empty body reads as `{}`, as the contract has it. The body's content type is not
/// checked: clients send JSON without saying so.
#[derive(Debug)]
pub(crate) struct Valid<T>(pub(crate) T);

impl<T> Valid<T>
where
    T: DeserializeOwned + garde::Validate<Context = ()>,
{
    /// `value` read as `T` and checked against `T`'s rules.
    pub(crate) fn check(value: Value) -> Result<Self, Invalid> {
        let body: T = serde_json::from_value(value).map_err(Invalid::Shape)?;
        body.validate().map_err(Invalid::Rules)?;
        Ok(Self(body))
    }
}

/// A body read as JSON, not yet as any type: for a route that looks at one field before the
/// rest (pairing reads its code first).
#[derive(Debug)]
pub(crate) struct JsonBody(pub(crate) Value);

/// Why a request's body was not taken.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Invalid {
    /// The body is over the server's limit (axum's default, 2 MB).
    #[error("the request body is over the limit")]
    TooLarge,
    /// The body could not be read.
    #[error("the request body could not be read")]
    Unreadable(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The body is not JSON of the right shape.
    #[error("not a request this route takes")]
    Shape(#[source] serde_json::Error),
    /// The body broke a validation rule.
    #[error("the request breaks a rule")]
    Rules(#[source] garde::Report),
}

impl Invalid {
    /// The reason, for a person.
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::TooLarge => "The request is too large.".to_owned(),
            Self::Unreadable(_) => "The request body could not be read.".to_owned(),
            Self::Shape(error) => format!("Not a request this route takes: {error}"),
            Self::Rules(report) => report
                .iter()
                .map(|(path, error)| format!("{path}: {error}"))
                .collect::<Vec<_>>()
                .join("; "),
        }
    }

    /// Whether the rule broken is one on the field `field` (its path's first part).
    pub(crate) fn breaks_rule_on(&self, field: &str) -> bool {
        match self {
            Self::Rules(report) => report
                .iter()
                .any(|(path, _)| path.to_string().split('.').next() == Some(field)),
            Self::TooLarge | Self::Unreadable(_) | Self::Shape(_) => false,
        }
    }
}

impl IntoResponse for Invalid {
    fn into_response(self) -> Response {
        ApiError::from(self).into_response()
    }
}

impl From<Invalid> for ApiError {
    fn from(invalid: Invalid) -> Self {
        Self::Invalid(invalid)
    }
}

impl<S: Send + Sync> FromRequest<S> for JsonBody {
    type Rejection = Invalid;

    async fn from_request(request: Request, state: &S) -> Result<Self, Invalid> {
        let bytes =
            Bytes::from_request(request, state)
                .await
                .map_err(|rejection| match rejection {
                    BytesRejection::FailedToBufferBody(FailedToBufferBody::LengthLimitError(_)) => {
                        Invalid::TooLarge
                    }
                    other => Invalid::Unreadable(Box::new(other)),
                })?;
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(Self(Value::Object(serde_json::Map::new())));
        }
        serde_json::from_slice(&bytes)
            .map(Self)
            .map_err(Invalid::Shape)
    }
}

impl<T, S> FromRequest<S> for Valid<T>
where
    T: DeserializeOwned + garde::Validate<Context = ()>,
    S: Send + Sync,
{
    type Rejection = Invalid;

    async fn from_request(request: Request, state: &S) -> Result<Self, Invalid> {
        let JsonBody(value) = JsonBody::from_request(request, state).await?;
        Self::check(value)
    }
}

/// The room a query names (`?room=`). Missing, not a positive whole number, or (later, at the
/// service) not a room that exists: Main. The contract has no error for a room.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RoomParam(pub(crate) Option<RoomId>);

#[derive(Deserialize)]
struct RawRoom {
    room: Option<String>,
}

impl<S: Send + Sync> FromRequestParts<S> for RoomParam {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        let room = Query::<RawRoom>::from_request_parts(parts, state)
            .await
            .ok()
            .and_then(|Query(raw)| raw.room)
            .and_then(|room| room.trim().parse::<i64>().ok())
            .filter(|room| *room > 0)
            .map(RoomId::new);
        Ok(Self(room))
    }
}

/// The session gate: a request that takes this passed it, with a signed-in device or (in
/// report mode only, noted in the log) without one. In enforce mode a request without a
/// session is refused here, with 401 `signIn`. It holds the signed-in device, if there is one,
/// for what only a signed-in device may do even in report mode.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Viewer(pub(crate) Option<DeviceId>);

impl FromRequestParts<AppState> for Viewer {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        if let Some(device) = session(parts, state).await? {
            return Ok(Self(Some(device)));
        }
        match state.auth_mode() {
            AuthMode::Enforce => Err(ApiError::SignInRequired),
            AuthMode::Report => {
                let peer = parts
                    .extensions
                    .get::<ConnectInfo<SocketAddr>>()
                    .map(|ConnectInfo(address)| address.ip());
                state.report_log().note(peer, parts.uri.path());
                Ok(Self(None))
            }
        }
    }
}

/// A signed-in device, in either auth mode: for what a session made without one would outlive
/// (making a pairing code).
#[derive(Clone, Copy, Debug)]
pub(crate) struct SignedIn(pub(crate) DeviceId);

impl FromRequestParts<AppState> for SignedIn {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        session(parts, state)
            .await?
            .map(Self)
            .ok_or(ApiError::DeviceRequired)
    }
}

impl SignedIn {
    /// The signed-in device behind a request that acts (runs something, spends money,
    /// records, or changes data for the owner), in either auth mode: report mode's pass for a
    /// request without a session is for reading, never for acting. A route takes
    /// `Result<SignedIn, ApiError>` and hands it here.
    ///
    /// # Errors
    ///
    /// [`ApiError::SignInRequired`] (401 `signIn`, as enforce mode answers) without a session;
    /// the session check's own failure.
    pub(crate) fn acting(device: Result<Self, ApiError>) -> Result<DeviceId, ApiError> {
        device
            .map(|Self(device)| device)
            .map_err(|error| match error {
                ApiError::DeviceRequired => ApiError::SignInRequired,
                other => other,
            })
    }
}

/// The signed-in device behind the request, if there is one, in either auth mode: for what
/// answers with or without a session (`GET /api/auth/status`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct MaybeSignedIn(pub(crate) Option<DeviceId>);

impl FromRequestParts<AppState> for MaybeSignedIn {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        Ok(Self(session(parts, state).await?))
    }
}

/// The device whose access token the request carries: `Authorization: Bearer <token>` (the
/// prefix in any case), or `?token=<token>` for media a player opens itself.
async fn session(parts: &Parts, state: &AppState) -> Result<Option<DeviceId>, ApiError> {
    let Some(token) = bearer(parts).or_else(|| query_token(parts)) else {
        return Ok(None);
    };
    Ok(state.auth().authenticate(token).await?)
}

fn bearer(parts: &Parts) -> Option<&str> {
    let value = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim())
        .filter(|token| !token.is_empty())
}

fn query_token(parts: &Parts) -> Option<&str> {
    parts
        .uri
        .query()?
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == "token")
        .map(|(_, token)| token)
        .filter(|token| !token.is_empty())
}

#[cfg(test)]
mod tests {
    use axum::http::Request as HttpRequest;

    use super::*;

    fn parts(request: HttpRequest<()>) -> Parts {
        request.into_parts().0
    }

    #[test]
    fn the_bearer_prefix_is_read_in_any_case() {
        for value in ["Bearer abc", "bearer abc", "BEARER  abc "] {
            let request = HttpRequest::get("/")
                .header("authorization", value)
                .body(())
                .unwrap();
            assert_eq!(bearer(&parts(request)), Some("abc"), "{value}");
        }
        let basic = HttpRequest::get("/")
            .header("authorization", "Basic abc")
            .body(())
            .unwrap();
        assert_eq!(bearer(&parts(basic)), None);
    }

    #[test]
    fn a_token_may_come_in_the_query() {
        let request = HttpRequest::get("/uploads/x?room=2&token=abc")
            .body(())
            .unwrap();
        assert_eq!(query_token(&parts(request)), Some("abc"));
        let none = HttpRequest::get("/uploads/x?token=").body(()).unwrap();
        assert_eq!(query_token(&parts(none)), None);
    }

    #[tokio::test]
    async fn a_room_that_is_not_a_positive_number_is_main() {
        for (query, room) in [
            ("/?room=2", Some(2)),
            ("/?room=0", None),
            ("/?room=-3", None),
            ("/?room=two", None),
            ("/", None),
        ] {
            let mut parts = parts(HttpRequest::get(query).body(()).unwrap());
            let RoomParam(got) = RoomParam::from_request_parts(&mut parts, &())
                .await
                .unwrap();
            assert_eq!(got, room.map(RoomId::new), "{query}");
        }
    }
}
