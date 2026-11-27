//! `POST /api/talk` and `POST /api/seen`.

use axum::Json;
use axum::extract::State;
use emberhut_api::error::{ErrorBody, OkReply};
use emberhut_api::seen::SeenRequest;
use emberhut_api::talk::{TalkReply, TalkRequest};

use crate::app::AppState;
use crate::convert::{message_from_api, room_from_api};
use crate::error::ApiError;
use crate::extract::{Valid, Viewer};

/// Says something in a room. The message is stored at once; the reply comes with the next
/// polls and on the live stream.
#[utoipa::path(
    post,
    path = "/api/talk",
    tag = "talk",
    security(("bearer" = [])),
    request_body = TalkRequest,
    responses(
        (status = 200, description = "Said; the assistant's turn has started", body = TalkReply),
        (
            status = 400,
            description = "A body the route refuses, or a blank message without files the \
                server stored",
            body = ErrorBody
        ),
        (status = 413, description = "The body is over the server's limit", body = ErrorBody),
        (
            status = 429,
            description = "Eight messages in this room are waiting for an answer already",
            body = ErrorBody
        ),
        (status = 503, description = "The server is shutting down", body = ErrorBody),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn talk(
    State(app): State<AppState>,
    _viewer: Viewer,
    Valid(request): Valid<TalkRequest>,
) -> Result<Json<TalkReply>, ApiError> {
    // Only files the server stored are taken; the others are dropped, as on the reference
    // server.
    let files = app.uploads().attachments(&request.files).await?;
    let room = app
        .state()
        .resolve(Some(room_from_api(request.room)))
        .await?;
    app.talk()
        .talk_with_files(
            room,
            request.text,
            files,
            request.reply_to.map(message_from_api),
        )
        .await?;
    Ok(Json(TalkReply {
        ok: true,
        started: true,
        id: None,
    }))
}

/// A room's messages have been seen on screen: its `unread` clears.
#[utoipa::path(
    post,
    path = "/api/seen",
    tag = "talk",
    security(("bearer" = [])),
    request_body = SeenRequest,
    responses(
        (status = 200, description = "Seen", body = OkReply),
        (status = 400, description = "A body the route refuses", body = ErrorBody),
        (status = 413, description = "The body is over the server's limit", body = ErrorBody),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn seen(
    State(app): State<AppState>,
    _viewer: Viewer,
    Valid(request): Valid<SeenRequest>,
) -> Result<Json<OkReply>, ApiError> {
    let room = app
        .state()
        .resolve(Some(room_from_api(request.room)))
        .await?;
    app.talk().seen(room).await?;
    Ok(Json(OkReply::OK))
}
