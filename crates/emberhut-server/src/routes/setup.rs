//! `GET /api/setup`: the setup checklist, for the apps' onboarding.
//!
//! It is read with a session, as the rest of what the owner has connected is: before pairing,
//! an app asks `GET /api/auth/status` whether the server is in use, and needs nothing more.

use axum::Json;
use axum::extract::State;
use emberhut_api::error::ErrorBody;
use emberhut_api::setup::SetupPage;

use crate::app::AppState;
use crate::error::{ApiError, CauseChain};
use crate::extract::Viewer;

/// Every item of the checklist and how it is: `ready`, `missing` (the owner can set it up
/// from the apps), `off` (it takes the server's own configuration) or `error` (set up, but
/// failing now), with a line saying how many there are or what to do. The Mac helper is asked
/// for its health, within 5 s; nothing else leaves the server.
#[utoipa::path(
    get,
    path = "/api/setup",
    tag = "setup",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The checklist", body = SetupPage),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn setup(
    State(app): State<AppState>,
    _viewer: Viewer,
) -> Result<Json<SetupPage>, ApiError> {
    app.setup().page().await.map(Json).map_err(|error| {
        tracing::error!(error = %CauseChain(&error), "the setup checklist could not be read");
        ApiError::Internal
    })
}
