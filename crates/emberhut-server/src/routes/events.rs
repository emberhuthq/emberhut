//! `POST /api/events`: a program (a script when a long job ends, a webhook) tells the
//! assistant something, with a paired device's token.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use emberhut_api::error::{ErrorBody, OkReply};
use emberhut_api::wake::EventRequest;
use emberhut_core::RoomId;
use serde_json::Value;

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{SignedIn, Valid};

/// The refusal of a body without text.
const TEXT_REQUIRED: &str = "text is required";

/// The kind an event has when the body names none.
const NOTE: &str = "note";

/// Tells the assistant `text`: an event it reads at its next wake, in the room of `project`
/// (a room's name) when the server has one by that name, in Main otherwise.
#[utoipa::path(
    post,
    path = "/api/events",
    tag = "talk",
    security(("bearer" = [])),
    request_body = EventRequest,
    responses(
        (status = 200, description = "Told; it wakes the room by the wake rule", body = OkReply),
        (
            status = 400,
            description = "No text (`{ ok: false, error: \"text is required\" }`), a text \
                over 100 000 characters, a kind that isn't a plain word, or a body that isn't \
                JSON",
            body = ErrorBody
        ),
        (status = 413, description = "The body is over the server's limit", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
    )
)]
pub(super) async fn events(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Valid(request): Valid<EventRequest>,
) -> Result<Response, ApiError> {
    // An event wakes the assistant, which then acts with its tools: a signed-in device only.
    SignedIn::acting(device)?;
    let Some(text) = request.text else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                ok: Some(false),
                ..ErrorBody::new(TEXT_REQUIRED)
            }),
        )
            .into_response());
    };
    let kind = request
        .kind
        .as_deref()
        .filter(|kind| !kind.is_empty())
        .unwrap_or(NOTE);
    let room = match request.project.as_ref().and_then(Value::as_str) {
        Some(name) if !name.trim().chars().all(|c| c.is_ascii_digit()) => app
            .rooms()
            .find(name)
            .await
            .map_err(fault)?
            .map_or(RoomId::MAIN, |room| room.id),
        _ => RoomId::MAIN,
    };
    app.wake().event(room, kind, &text).await.map_err(fault)?;
    Ok(Json(OkReply::OK).into_response())
}

/// A database failure, answered as the server's fault.
fn fault(error: emberhut_db::DbError) -> ApiError {
    ApiError::State(error.into())
}
