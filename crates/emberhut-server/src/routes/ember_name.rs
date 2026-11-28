//! `GET /api/ember/name` and `POST /api/ember/name`: what the owner calls their ember, and
//! renaming it.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use emberhut_api::ember_name::{EmberName, EmberNameChange, EmberNameReply};
use emberhut_api::error::ErrorBody;

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{JsonBody, Viewer};
use crate::services::ember_name::EmberNameError;

use super::theme::fault;

/// The ember's name: the owner's, or "Your ember" (`custom: false`).
#[utoipa::path(
    get,
    path = "/api/ember/name",
    tag = "ember",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The ember's name", body = EmberName),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn name(
    State(app): State<AppState>,
    _viewer: Viewer,
) -> Result<Json<EmberName>, ApiError> {
    Ok(Json(app.ember_name().name().await.map_err(fault)?))
}

/// Renames the ember: `{ "name": "Pip" }`, or `{ "name": null }` (or `""`) for the default.
/// The name is trimmed, its runs of spaces made one, and must be one line of at most 24
/// characters, an emoji counting as one.
#[utoipa::path(
    post,
    path = "/api/ember/name",
    tag = "ember",
    security(("bearer" = [])),
    request_body = EmberNameChange,
    responses(
        (status = 200, description = "Renamed", body = EmberNameReply),
        (
            status = 400,
            description = "A name the server doesn't take, or a body that isn't `{ name }` \
                (`{ ok: false, error }`)",
            body = ErrorBody
        ),
        (status = 413, description = "The body is over the server's limit", body = ErrorBody),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn set_name(
    State(app): State<AppState>,
    _viewer: Viewer,
    JsonBody(change): JsonBody,
) -> Result<Json<EmberNameReply>, NameFailure> {
    let ember = app.ember_name().change(&change).await?;
    Ok(Json(EmberNameReply { ok: true, ember }))
}

/// A rename not made: a refusal (400, `{ ok: false, error }`) or any other failure.
#[derive(Debug)]
pub(super) enum NameFailure {
    Refused(String),
    Api(ApiError),
}

impl From<EmberNameError> for NameFailure {
    fn from(error: EmberNameError) -> Self {
        match error {
            EmberNameError::Database(error) => Self::Api(fault(error)),
            refused => Self::Refused(refused.to_string()),
        }
    }
}

impl IntoResponse for NameFailure {
    fn into_response(self) -> Response {
        match self {
            Self::Refused(error) => (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    ok: Some(false),
                    ..ErrorBody::new(error)
                }),
            )
                .into_response(),
            Self::Api(error) => error.into_response(),
        }
    }
}
