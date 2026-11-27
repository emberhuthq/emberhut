//! `GET /api/theme` and `POST /api/theme`: the theme choice, its colours, and changing it.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use emberhut_api::error::ErrorBody;
use emberhut_api::theme::{ThemeChange, ThemePage, ThemeSetReply};

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{JsonBody, Viewer};
use crate::services::theme::{ThemeError, page};

/// The theme choice, its colours in both appearances, its contrast, and every preset's colours.
#[utoipa::path(
    get,
    path = "/api/theme",
    tag = "theme",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The theme page", body = ThemePage),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn theme(
    State(app): State<AppState>,
    _viewer: Viewer,
) -> Result<Json<ThemePage>, ApiError> {
    Ok(Json(app.theme().page().await.map_err(fault)?))
}

/// Changes the fields given and answers the page as it is now; with `preview`, as it would be,
/// saving nothing.
#[utoipa::path(
    post,
    path = "/api/theme",
    tag = "theme",
    security(("bearer" = [])),
    request_body = ThemeChange,
    responses(
        (status = 200, description = "Changed (or worked out, with preview)", body = ThemeSetReply),
        (
            status = 400,
            description = "A value the theme doesn't take, or a body that isn't JSON \
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
pub(super) async fn set_theme(
    State(app): State<AppState>,
    _viewer: Viewer,
    JsonBody(change): JsonBody,
) -> Result<Json<ThemeSetReply>, ThemeFailure> {
    let preview = change
        .get("preview")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let choice = app.theme().change(&change, preview).await?;
    Ok(Json(ThemeSetReply {
        ok: true,
        page: page(&choice),
    }))
}

/// A theme change not made: a refusal (400, `{ ok: false, error }`) or any other failure.
#[derive(Debug)]
pub(super) enum ThemeFailure {
    Refused(String),
    Api(ApiError),
}

/// A database failure, answered as the server's fault (500).
pub(super) fn fault(error: emberhut_db::DbError) -> ApiError {
    ApiError::State(error.into())
}

impl From<ThemeError> for ThemeFailure {
    fn from(error: ThemeError) -> Self {
        match error {
            ThemeError::Refused(refusal) => Self::Refused(refusal.to_string()),
            ThemeError::Database(error) => Self::Api(fault(error)),
        }
    }
}

impl IntoResponse for ThemeFailure {
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
