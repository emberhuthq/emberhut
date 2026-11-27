//! `GET /health` and `GET /api/loop`: whether the server answers, and how late its runtime is.

use axum::Json;
use axum::extract::State;
use emberhut_api::error::ErrorBody;
use emberhut_api::health::{Health, LoopReport};

use crate::app::AppState;
use crate::extract::Viewer;

/// Whether the server answers: `{ ok: true, lag }`. No sign-in, so a supervisor or a load
/// balancer can ask; it says nothing about anyone's data.
#[utoipa::path(
    get,
    path = "/health",
    tag = "meta",
    responses((status = 200, description = "The server answers", body = Health))
)]
pub(super) async fn health(State(app): State<AppState>) -> Json<Health> {
    Json(app.lag().health())
}

/// How late the server's runtime is, and the work that held it 50 ms or more in the last five
/// minutes, by name, the longest in all first.
#[utoipa::path(
    get,
    path = "/api/loop",
    tag = "meta",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The lag and the slow work", body = LoopReport),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn lag(State(app): State<AppState>, _viewer: Viewer) -> Json<LoopReport> {
    Json(app.lag().report())
}
