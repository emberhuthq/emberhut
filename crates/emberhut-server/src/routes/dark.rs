//! `GET`, `POST` and `DELETE /api/dark`: the off-the-record room. Its chat, a message and its
//! reply as server-sent events, and clearing it.

use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderName, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use emberhut_api::dark::{DarkEvent, DarkRequest, DarkRoom};
use emberhut_api::error::{ErrorBody, OkReply};
use emberhut_api::live::PING_INTERVAL_SECS;
use futures_util::Stream;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{JsonBody, SignedIn, Viewer};
use crate::services::dark::DarkError;

/// Asks proxies (nginx) not to buffer the stream.
const NO_BUFFERING: (HeaderName, &str) = (HeaderName::from_static("x-accel-buffering"), "no");

/// The room: its model, where it runs, whether a reply is being written, and the chat.
#[utoipa::path(
    get,
    path = "/api/dark",
    tag = "dark",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The room", body = DarkRoom),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
    )
)]
pub(super) async fn page(
    State(app): State<AppState>,
    _viewer: Viewer,
) -> Result<Json<DarkRoom>, ApiError> {
    Ok(Json(app.dark().page().await.map_err(fault)?))
}

/// A message, and its reply as server-sent events: `{ thinking: true }` while the model
/// reasons, `{ delta }` as it writes, then `{ done: message }` or `{ error }`.
#[utoipa::path(
    post,
    path = "/api/dark",
    tag = "dark",
    security(("bearer" = [])),
    request_body = DarkRequest,
    responses(
        (
            status = 200,
            description = "A text/event-stream whose every event's data is a DarkEvent, the \
                last one `done` or `error`",
            content_type = "text/event-stream",
            body = DarkEvent
        ),
        (status = 400, description = "Nothing said, or a body that isn't JSON", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 409, description = "A reply is still being written", body = ErrorBody),
        (status = 413, description = "The body is over the server's limit", body = ErrorBody),
    )
)]
pub(super) async fn say(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    JsonBody(body): JsonBody,
) -> Result<impl IntoResponse, DarkFailure> {
    super::acting(device)?;
    let text = match body.get("text") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    let events = app.dark().say(&text).await?;
    let keep_alive = KeepAlive::new()
        .interval(Duration::from_secs(PING_INTERVAL_SECS))
        .text("ping");
    Ok((
        [NO_BUFFERING],
        Sse::new(stream(events)).keep_alive(keep_alive),
    ))
}

/// The reply's events until its last.
fn stream(
    events: mpsc::UnboundedReceiver<DarkEvent>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    futures_util::stream::unfold(events, |mut events| async move {
        let event = events.recv().await?;
        let json = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_owned());
        Some((Ok(Event::default().data(json)), events))
    })
}

/// Clears the chat.
#[utoipa::path(
    delete,
    path = "/api/dark",
    tag = "dark",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Cleared", body = OkReply),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 409, description = "A reply is still being written", body = ErrorBody),
    )
)]
pub(super) async fn clear(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
) -> Result<Json<OkReply>, DarkFailure> {
    super::acting(device)?;
    app.dark().clear().await?;
    Ok(Json(OkReply::OK))
}

fn fault(error: emberhut_db::DbError) -> ApiError {
    ApiError::State(error.into())
}

/// A message or a clear refused (`{ error }`) or failed, or
/// the server's own answer (no session).
#[derive(Debug)]
pub(super) enum DarkFailure {
    /// The room's answer.
    Dark(DarkError),
    /// The server's own.
    Api(ApiError),
}

impl From<DarkError> for DarkFailure {
    fn from(error: DarkError) -> Self {
        Self::Dark(error)
    }
}

impl From<ApiError> for DarkFailure {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl IntoResponse for DarkFailure {
    fn into_response(self) -> Response {
        let error = match self {
            Self::Dark(error) => error,
            Self::Api(error) => return error.into_response(),
        };
        let said = error.to_string();
        let status = match error {
            DarkError::Blank => StatusCode::BAD_REQUEST,
            DarkError::Busy => StatusCode::CONFLICT,
            DarkError::Database(error) => return fault(error).into_response(),
        };
        (status, Json(ErrorBody::new(said))).into_response()
    }
}
