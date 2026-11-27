//! `GET /api/mac` and `POST /api/mac/lock`: the owner's Mac row and "Lock now".

use axum::Json;
use axum::extract::State;
use emberhut_api::error::{ErrorBody, OkReply};
use emberhut_api::mac::MacStatus;

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::Viewer;

/// The Mac's row: whether its helper answers (within 5 s), and then whether the screen is
/// locked and how long the Mac has been idle.
#[utoipa::path(
    get,
    path = "/api/mac",
    tag = "mac",
    security(("bearer" = [])),
    responses(
        (
            status = 200,
            description = "The Mac's row; `{ online: false }` alone when its helper doesn't \
                answer or none is set up",
            body = MacStatus
        ),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn mac(State(app): State<AppState>, _viewer: Viewer) -> Json<MacStatus> {
    Json(app.mac().status().await)
}

/// Locks the Mac's screen now. Nothing unlocks it.
#[utoipa::path(
    post,
    path = "/api/mac/lock",
    tag = "mac",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Locked", body = OkReply),
        (
            status = 400,
            description = "Not locked: no Mac helper is set up, it doesn't answer (within \
                10 s), or it refused, as `error` says",
            body = ErrorBody
        ),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn lock(
    State(app): State<AppState>,
    _viewer: Viewer,
) -> Result<Json<OkReply>, ApiError> {
    app.mac().lock().await?;
    Ok(Json(OkReply::OK))
}
