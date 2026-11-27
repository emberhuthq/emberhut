//! Voice: `POST /api/voice/session`, `POST /api/voice/ask`, `POST /api/voice/done` and
//! `POST /api/transcribe`. A failure underneath is 502 `{ ok: false, error }`, the reference
//! server's answer.
//!
//! Every one of them needs a signed-in device in either auth mode: a voice session hands out a
//! key billed to the provider's account, dictation is paid for the same way, and a call's
//! request and its end speak to the assistant for the owner. Report mode's pass for a request
//! without a session is for reading only.

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use emberhut_api::error::{ErrorBody, OkReply};
use emberhut_api::transcribe::{MAX_SIZE, TranscribeQuery, TranscribeReply};
use emberhut_api::voice::{VoiceAnswer, VoiceAsk, VoiceDone, VoiceSession};
use futures_util::StreamExt as _;

use crate::app::AppState;
use crate::convert::room_from_api;
use crate::error::ApiError;
use crate::extract::{SignedIn, Valid};

/// `{ ok: false, error }` with `status`.
fn failed(status: StatusCode, error: &dyn std::error::Error) -> Response {
    tracing::warn!(%error, "a voice request failed");
    let mut body = ErrorBody::new(error.to_string());
    body.ok = Some(false);
    (status, Json(body)).into_response()
}

/// A key for one voice call (10 minutes), with the realtime session set up.
#[utoipa::path(
    post,
    path = "/api/voice/session",
    tag = "voice",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The call's key", body = VoiceSession),
        (status = 502, description = "The voice provider isn't set up or refused", body = ErrorBody),
        (
            status = 401,
            description = "No signed-in device (in either auth mode)",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn session(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
) -> Response {
    if let Err(error) = SignedIn::acting(device) {
        return error.into_response();
    }
    match app.voice().session().await {
        Ok(session) => Json(session).into_response(),
        Err(error) => failed(StatusCode::BAD_GATEWAY, &error),
    }
}

/// The call asks the assistant: the request is said in the room's chat (marked as voice) and
/// the assistant's answer comes back (two minutes at most; then a line saying it will be in
/// the chat).
#[utoipa::path(
    post,
    path = "/api/voice/ask",
    tag = "voice",
    security(("bearer" = [])),
    request_body = VoiceAsk,
    responses(
        (status = 200, description = "The assistant's answer", body = VoiceAnswer),
        (status = 400, description = "A body the route refuses", body = ErrorBody),
        (status = 502, description = "The request could not be said", body = ErrorBody),
        (
            status = 401,
            description = "No signed-in device (in either auth mode)",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn ask(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Valid(request): Valid<VoiceAsk>,
) -> Result<Response, ApiError> {
    SignedIn::acting(device)?;
    let room = app
        .state()
        .resolve(Some(room_from_api(request.room)))
        .await?;
    Ok(
        match app.voice().ask(app.talk(), &request.request, room).await {
            Ok(answer) => Json(VoiceAnswer { ok: true, answer }).into_response(),
            Err(error) => failed(StatusCode::BAD_GATEWAY, &error),
        },
    )
}

/// The voice call ended: the phone's report is kept, and what was said goes to the room's
/// assistant to follow up.
#[utoipa::path(
    post,
    path = "/api/voice/done",
    tag = "voice",
    security(("bearer" = [])),
    request_body = VoiceDone,
    responses(
        (status = 200, description = "Taken", body = OkReply),
        (status = 400, description = "A body the route refuses", body = ErrorBody),
        (
            status = 401,
            description = "No signed-in device (in either auth mode)",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn done(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Valid(request): Valid<VoiceDone>,
) -> Result<Response, ApiError> {
    SignedIn::acting(device)?;
    let room = app
        .state()
        .resolve(Some(room_from_api(request.room)))
        .await?;
    let done = app
        .voice()
        .done(
            request.seconds.unwrap_or(0.0),
            request.diag.as_deref(),
            request.transcript.as_deref().unwrap_or_default(),
            room,
        )
        .await;
    match done {
        Ok(()) => Ok(Json(OkReply::OK).into_response()),
        Err(error) => {
            tracing::error!(%error, "a voice call's end was not kept");
            Err(ApiError::Internal)
        }
    }
}

/// Dictation: what the recording (the body) says. `words` are names it may have, so they are
/// spelled right.
#[utoipa::path(
    post,
    path = "/api/transcribe",
    tag = "voice",
    security(("bearer" = [])),
    params(TranscribeQuery),
    request_body(
        content = Vec<u8>,
        content_type = "application/octet-stream",
        description = "The recording, raw"
    ),
    responses(
        (status = 200, description = "What was said", body = TranscribeReply),
        (status = 413, description = "The recording is over 25 MiB", body = ErrorBody),
        (
            status = 502,
            description = "The speech provider isn't set up or failed, or the recording didn't \
                arrive whole",
            body = ErrorBody
        ),
        (
            status = 401,
            description = "No signed-in device (in either auth mode)",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn transcribe(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    query: Result<Query<TranscribeQuery>, QueryRejection>,
    body: Body,
) -> Response {
    if let Err(error) = SignedIn::acting(device) {
        return error.into_response();
    }
    let query = query.map_or_default(|Query(query)| query);
    let mut stream = body.into_data_stream();
    let mut audio = Vec::new();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            let mut body = ErrorBody::new("The recording did not arrive whole. Try again.");
            body.ok = Some(false);
            return (StatusCode::BAD_GATEWAY, Json(body)).into_response();
        };
        audio.extend_from_slice(&chunk);
        if u64::try_from(audio.len()).is_ok_and(|size| size > MAX_SIZE) {
            let mut body = ErrorBody::new("recording too long");
            body.ok = Some(false);
            return (StatusCode::PAYLOAD_TOO_LARGE, Json(body)).into_response();
        }
    }
    match app
        .voice()
        .transcribe(Bytes::from(audio), &query.name, &query.words)
        .await
    {
        Ok(text) => Json(TranscribeReply { ok: true, text }).into_response(),
        Err(error) => failed(StatusCode::BAD_GATEWAY, &error),
    }
}
