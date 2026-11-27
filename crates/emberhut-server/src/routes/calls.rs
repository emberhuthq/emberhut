//! The calls: `POST /api/calls/record` (the app's Record button), `POST /api/calls/in` and
//! `POST /api/calls/live` (what the Mac's recorder sends), `GET /api/calls`,
//! `GET /api/calls/{id}/transcript` and `GET /api/calls/{id}/live`.
//!
//! The recorder on the Mac signs in as a device, like the apps. Recording (the Record button,
//! the tracks and the live chunks) needs a signed-in device in either auth mode: report mode's
//! pass for a request without a session reads, and never turns the microphone on or stores
//! audio. Reading the calls passes the session gate as any route does.

use axum::Json;
use axum::body::Body;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use emberhut_api::calls::{
    CallList, CallLive, ChunkQuery, ReceivedTrack, RecordAction, RecordCallReply,
    RecordCallRequest, TrackQuery,
};
use emberhut_api::error::{ErrorBody, OkReply};
use emberhut_core::calls::CallId;

use crate::app::AppState;
use crate::convert::room_from_api;
use crate::error::ApiError;
use crate::extract::{SignedIn, Valid, Viewer};
use crate::services::calls::CallError;
use crate::services::mac::RecorderCommand;

/// A call route's refusal, or a fault (logged here).
fn refusal(error: CallError) -> Response {
    match error {
        CallError::Refused(why) => {
            (StatusCode::BAD_REQUEST, Json(ErrorBody::new(why))).into_response()
        }
        CallError::Storage(error) => {
            tracing::warn!(%error, "a call's audio was not kept");
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody::new("The audio did not arrive whole. Try again.")),
            )
                .into_response()
        }
        CallError::Mac(error) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new(error.to_string())),
        )
            .into_response(),
        error => fault(&error),
    }
}

/// A server fault: logged with its causes, answered 500.
fn fault(error: &dyn std::error::Error) -> Response {
    tracing::error!(error = %causes(error), "a call route failed");
    ApiError::Internal.into_response()
}

/// `error` with its causes, for the log.
fn causes(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// Starts recording a call on the owner's Mac (`action` anything but `stop`), or stops it. The
/// transcript goes to `room`. Always 200: `ok: false` with
/// `error` when the Mac didn't do it or the call couldn't be kept.
#[utoipa::path(
    post,
    path = "/api/calls/record",
    tag = "calls",
    security(("bearer" = [])),
    request_body = RecordCallRequest,
    responses(
        (status = 200, description = "What the Mac's recorder said, or why it (or the server) didn't", body = RecordCallReply),
        (status = 400, description = "A body the route refuses", body = ErrorBody),
        (
            status = 401,
            description = "No signed-in device (in either auth mode)",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn record(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Valid(request): Valid<RecordCallRequest>,
) -> Result<Response, ApiError> {
    SignedIn::acting(device)?;
    let room = app
        .state()
        .resolve(Some(room_from_api(request.room)))
        .await?;
    let action = match request.action {
        RecordAction::Start => RecorderCommand::Start,
        RecordAction::Stop => RecorderCommand::Stop,
    };
    Ok(match app.calls().record(action, None, None, room).await {
        Ok(text) => Json(RecordCallReply {
            ok: true,
            text,
            error: None,
        })
        .into_response(),
        Err(error) => {
            if !matches!(error, CallError::Mac(_)) {
                tracing::error!(error = %causes(&error), "a call was not started or stopped");
            }
            Json(RecordCallReply {
                ok: false,
                text: None,
                error: Some(error.to_string()),
            })
            .into_response()
        }
    })
}

/// One track of a call the Mac recorded, its audio (m4a) as the body; `last` queues the call
/// to be transcribed (and ends its live copilot).
#[utoipa::path(
    post,
    path = "/api/calls/in",
    tag = "calls",
    security(("bearer" = [])),
    params(TrackQuery),
    request_body(
        content = Vec<u8>,
        content_type = "application/octet-stream",
        description = "The track's audio, raw"
    ),
    responses(
        (status = 200, description = "Kept", body = ReceivedTrack),
        (
            status = 400,
            description = "No usable call id or track, or the audio didn't arrive whole",
            body = ErrorBody
        ),
        (
            status = 401,
            description = "No signed-in device (in either auth mode)",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn track(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    query: Result<Query<TrackQuery>, QueryRejection>,
    body: Body,
) -> Response {
    if let Err(error) = SignedIn::acting(device) {
        return error.into_response();
    }
    let query = query.map_or_default(|Query(query)| query);
    match app
        .calls()
        .receive_track(&query, body.into_data_stream())
        .await
    {
        Ok(received) => {
            if query.last.as_deref().is_some_and(|last| !last.is_empty()) {
                app.live_calls().end(&received.call);
            }
            Json(received).into_response()
        }
        Err(error) => refusal(error),
    }
}

/// About ten seconds of a track of a call being recorded, as the body (WAV): heard in the
/// background for the call's live transcript.
#[utoipa::path(
    post,
    path = "/api/calls/live",
    tag = "calls",
    security(("bearer" = [])),
    params(ChunkQuery),
    request_body(
        content = Vec<u8>,
        content_type = "application/octet-stream",
        description = "The chunk's audio, raw"
    ),
    responses(
        (status = 200, description = "Taken", body = OkReply),
        (
            status = 400,
            description = "No usable call id, track or time, or the audio didn't arrive whole",
            body = ErrorBody
        ),
        (
            status = 401,
            description = "No signed-in device (in either auth mode)",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn chunk(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    query: Result<Query<ChunkQuery>, QueryRejection>,
    body: Body,
) -> Response {
    if let Err(error) = SignedIn::acting(device) {
        return error.into_response();
    }
    let query = query.map_or_default(|Query(query)| query);
    match app
        .live_calls()
        .receive_chunk(&query, body.into_data_stream())
        .await
    {
        Ok(()) => Json(OkReply::OK).into_response(),
        Err(error) => refusal(error),
    }
}

/// The latest 8 calls, newest first (the Mac's menu lists them).
#[utoipa::path(
    get,
    path = "/api/calls",
    tag = "calls",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The latest calls", body = CallList),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn calls(State(app): State<AppState>, _viewer: Viewer) -> Response {
    match app.calls().latest().await {
        Ok(calls) => Json(CallList { calls }).into_response(),
        Err(error) => fault(&error),
    }
}

/// A call's transcript, as Markdown.
#[utoipa::path(
    get,
    path = "/api/calls/{id}/transcript",
    tag = "calls",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The call's id")),
    responses(
        (status = 200, description = "The transcript", body = String, content_type = "text/markdown"),
        (status = 404, description = "No transcript (yet) for this call", body = ErrorBody),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn transcript(
    State(app): State<AppState>,
    _viewer: Viewer,
    Path(id): Path<String>,
) -> Response {
    match app.calls().transcript(&id).await {
        Ok(Some(text)) => (
            [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
            text,
        )
            .into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(ErrorBody::new("no transcript"))).into_response(),
        Err(error) => fault(&error),
    }
}

/// A call's last lines (the microphone's echo of the other side left out) and the copilot's
/// hints, for the panel on the Mac.
#[utoipa::path(
    get,
    path = "/api/calls/{id}/live",
    tag = "calls",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The call's id")),
    responses(
        (status = 200, description = "The live lines and hints", body = CallLive),
        (status = 404, description = "Not a call's id", body = ErrorBody),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn live(
    State(app): State<AppState>,
    _viewer: Viewer,
    Path(id): Path<String>,
) -> Response {
    if CallId::parse(&id).is_none() {
        return ApiError::NotFound.into_response();
    }
    match app.live_calls().live(&id).await {
        Ok(live) => Json(live).into_response(),
        Err(error) => fault(&error),
    }
}
