//! `POST /api/health` (the phone's daily Health summaries) and `POST /api/screentime` (the Mac
//! helper's screen-time totals). Both store the owner's data, so both need a signed-in device in
//! either auth mode (report mode's pass for a request without a session reads, never writes);
//! the phone and the Mac helper sign in as devices.

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use emberhut_api::error::ErrorBody;
use emberhut_api::health_data::{HealthUpload, Saved, ScreenTimeUpload};

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{JsonBody, SignedIn};

/// The phone's daily Health summaries, one per day, each kept as it came in place of what the
/// day had; `why` when it could send none; `tz`, its time zone. A day without a `YYYY-MM-DD`
/// date is skipped. A real walk closes its walk task.
#[utoipa::path(
    post,
    path = "/api/health",
    tag = "health",
    security(("bearer" = [])),
    request_body = HealthUpload,
    responses(
        (status = 200, description = "How many days were kept", body = Saved),
        (status = 400, description = "Not JSON", body = ErrorBody),
        (
            status = 401,
            description = "No signed-in device (in either auth mode)",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn health(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    JsonBody(body): JsonBody,
) -> Response {
    if let Err(error) = SignedIn::acting(device) {
        return error.into_response();
    }
    match app.health_data().take_health(&body).await {
        Ok(saved) => Json(saved).into_response(),
        Err(error) => {
            tracing::error!(%error, "the phone's health days were not kept");
            ApiError::Internal.into_response()
        }
    }
}

/// The Mac helper's screen time, one entry per day and device, each kept in place of what
/// that day and device had. An entry without a `YYYY-MM-DD` date or a `device` is skipped.
#[utoipa::path(
    post,
    path = "/api/screentime",
    tag = "health",
    security(("bearer" = [])),
    request_body = ScreenTimeUpload,
    responses(
        (status = 200, description = "How many entries were kept", body = Saved),
        (status = 400, description = "Not JSON", body = ErrorBody),
        (
            status = 401,
            description = "No signed-in device (in either auth mode)",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn screentime(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    JsonBody(body): JsonBody,
) -> Response {
    if let Err(error) = SignedIn::acting(device) {
        return error.into_response();
    }
    match app.health_data().take_screentime(&body).await {
        Ok(saved) => Json(saved).into_response(),
        Err(error) => {
            tracing::error!(%error, "the Mac's screen time was not kept");
            ApiError::Internal.into_response()
        }
    }
}
