//! `POST /api/devices`: where the server pushes a phone's alerts.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use emberhut_api::error::{ErrorBody, OkReply};
use emberhut_api::push::PushRegistration;

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{Invalid, MaybeSignedIn, Valid};
use crate::services::push::PushError;

/// Keeps a signed-in phone's push address: an iPhone's APNs device token (with its service), or
/// an Android phone's FCM registration token. The same token again updates it.
///
/// Signing the phone out forgets it. A session is needed in either auth mode: report mode's
/// pass for a request without one would let anyone who reaches the server have the replies
/// pushed to them.
#[utoipa::path(
    post,
    path = "/api/devices",
    tag = "push",
    security(("bearer" = [])),
    request_body = PushRegistration,
    responses(
        (status = 200, description = "Kept: the phone gets the alerts", body = OkReply),
        (
            status = 400,
            description = "Not a push token (`not a device token`, `not an FCM registration \
                token`), or a body the route refuses",
            body = ErrorBody
        ),
        (status = 413, description = "The body is over the server's limit", body = ErrorBody),
        (
            status = 401,
            description = "There is no session (in either auth mode)",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn register(
    State(app): State<AppState>,
    MaybeSignedIn(device): MaybeSignedIn,
    body: Result<Valid<PushRegistration>, Invalid>,
) -> Response {
    let Some(device) = device else {
        return ApiError::SignInRequired.into_response();
    };
    let registration = match body {
        Ok(Valid(registration)) => registration,
        Err(invalid) => return ApiError::from(invalid).into_response(),
    };
    match app.push().register(&registration, device).await {
        Ok(()) => Json(OkReply::OK).into_response(),
        Err(PushError::Token(refusal)) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new(refusal.to_string())),
        )
            .into_response(),
        Err(error) => {
            tracing::error!(%error, "could not keep a push address");
            ApiError::Internal.into_response()
        }
    }
}
