//! `GET /api/waitlist` and `PUT /api/waitlist/{list}`: what the owner asked to hear about once
//! it exists (a home rented in the cloud), kept on this server until a central list exists.
//! Nothing is sent anywhere and nothing is paid.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use emberhut_api::error::ErrorBody;
use emberhut_api::waitlist::{Waitlist, Waitlists};
use emberhut_db::DbError;

use crate::app::AppState;
use crate::error::{ApiError, CauseChain};
use crate::extract::{SignedIn, Viewer};

/// A settings read or write that failed: logged, 500.
fn fault(error: &DbError) -> ApiError {
    tracing::error!(error = %CauseChain(error), "a waitlist couldn't be read or written");
    ApiError::Internal
}

/// Every list, and whether the owner is on it.
#[utoipa::path(
    get,
    path = "/api/waitlist",
    tag = "setup",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The lists", body = Waitlists),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
    )
)]
pub(super) async fn lists(
    State(app): State<AppState>,
    _viewer: Viewer,
) -> Result<Json<Waitlists>, ApiError> {
    app.waitlist().all().await.map(Json).map_err(|e| fault(&e))
}

/// Puts the owner on a list (`cloud-home`); joining again keeps the first time.
#[utoipa::path(
    put,
    path = "/api/waitlist/{list}",
    tag = "setup",
    security(("bearer" = [])),
    params(("list" = String, Path, description = "The list: `cloud-home`")),
    responses(
        (status = 200, description = "On the list", body = Waitlist),
        (status = 400, description = "No such list", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
    )
)]
pub(super) async fn join(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Path(list): Path<String>,
) -> Result<Response, ApiError> {
    SignedIn::acting(device)?;
    Ok(
        match app.waitlist().join(&list).await.map_err(|e| fault(&e))? {
            Some(joined) => Json(joined).into_response(),
            None => (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody::new("There is no such waitlist.")),
            )
                .into_response(),
        },
    )
}
