//! Settings' own pages: MCP servers and skills (`GET /api/switches`, `POST /api/switches`) and
//! Environments (`GET /api/environments`, `POST /api/environments/check`).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use emberhut_api::environments::{EnvActionRequest, EnvPage as EnvironmentsPage};
use emberhut_api::error::ErrorBody;
use emberhut_api::switches::{SwitchAction, SwitchPage};

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{Invalid, JsonBody, SignedIn, Viewer};
use crate::services::environments::{CheckRequest, EnvironmentError};
use crate::services::switches::{SwitchError, SwitchRequest};

/// A refusal: 400 `{ error }`, 500 for the server's own failure, or an error answered as every
/// route answers it (401 `signIn` for a change without a signed-in device).
#[derive(Debug)]
pub(super) enum PageRefusal {
    /// `status` with `{ error }`.
    Answer {
        /// The status.
        status: StatusCode,
        /// The error, in words.
        error: String,
    },
    /// The request may not make it ([`SignedIn::acting`]).
    Api(ApiError),
}

impl PageRefusal {
    fn refused(error: String) -> Self {
        Self::Answer {
            status: StatusCode::BAD_REQUEST,
            error,
        }
    }

    fn failed(error: &emberhut_db::DbError) -> Self {
        tracing::error!(%error, "a settings page could not be read");
        Self::Answer {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error: "Something went wrong on the server.".to_owned(),
        }
    }
}

impl From<ApiError> for PageRefusal {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl IntoResponse for PageRefusal {
    fn into_response(self) -> Response {
        match self {
            Self::Answer { status, error } => (status, Json(ErrorBody::new(error))).into_response(),
            Self::Api(error) => error.into_response(),
        }
    }
}

/// A body read as `T`, `T::default()` when it isn't one.
fn read<T: serde::de::DeserializeOwned + Default>(body: Result<JsonBody, Invalid>) -> T {
    body.ok()
        .and_then(|JsonBody(body)| serde_json::from_value(body).ok())
        .unwrap_or_default()
}

/// The assistant's MCP servers and skills, each with its state and the person's switch.
#[utoipa::path(
    get,
    path = "/api/switches",
    tag = "settings",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The page", body = SwitchPage),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
    )
)]
pub(super) async fn switches(
    State(app): State<AppState>,
    _viewer: Viewer,
) -> Result<Json<SwitchPage>, PageRefusal> {
    app.switches()
        .page()
        .await
        .map(Json)
        .map_err(|error| PageRefusal::failed(&error))
}

/// Turns a server or skill on or off (`{ kind: mcp | skill, name, on }`), or reconnects a
/// server (`{ kind: reconnect, name }`): the page after it.
#[utoipa::path(
    post,
    path = "/api/switches",
    tag = "settings",
    security(("bearer" = [])),
    request_body = SwitchAction,
    responses(
        (status = 200, description = "Done: the page", body = SwitchPage),
        (status = 400, description = "Not a change that can be made", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
    )
)]
pub(super) async fn switch(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    body: Result<JsonBody, Invalid>,
) -> Result<Json<SwitchPage>, PageRefusal> {
    SignedIn::acting(device)?;
    let action: SwitchRequest = read(body);
    app.switches()
        .act(&action)
        .await
        .map(Json)
        .map_err(|error| match error {
            SwitchError::Refused(why) => PageRefusal::refused(why),
            SwitchError::Database(error) => PageRefusal::failed(&error),
        })
}

/// Every place the assistant works and connection it reads through, live.
#[utoipa::path(
    get,
    path = "/api/environments",
    tag = "settings",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The page", body = EnvironmentsPage),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
    )
)]
pub(super) async fn environments(
    State(app): State<AppState>,
    _viewer: Viewer,
) -> Result<Json<EnvironmentsPage>, PageRefusal> {
    app.environments()
        .page()
        .await
        .map(Json)
        .map_err(|error| PageRefusal::failed(&error))
}

/// Looks at one place again now (`{ id, kind: check }`): the page after it.
#[utoipa::path(
    post,
    path = "/api/environments/check",
    tag = "settings",
    security(("bearer" = [])),
    request_body = EnvActionRequest,
    responses(
        (status = 200, description = "Looked again: the page", body = EnvironmentsPage),
        (status = 400, description = "Not an action this page has", body = ErrorBody),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
    )
)]
pub(super) async fn check(
    State(app): State<AppState>,
    _viewer: Viewer,
    body: Result<JsonBody, Invalid>,
) -> Result<Json<EnvironmentsPage>, PageRefusal> {
    let asked: CheckRequest = read(body);
    app.environments()
        .check(
            asked.id.as_deref().unwrap_or_default(),
            asked.kind.as_deref().unwrap_or("check"),
        )
        .await
        .map(Json)
        .map_err(|error| match error {
            EnvironmentError::Refused(why) => PageRefusal::refused(why),
            EnvironmentError::Database(error) => PageRefusal::failed(&error),
        })
}
