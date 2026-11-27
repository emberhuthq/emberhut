//! `GET /api/jobs` (every background job running now on the runners' machines) and `POST
//! /api/jobs/stop` (stop one, which the app confirms first).

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use emberhut_api::error::{ErrorBody, OkReply};
use emberhut_api::jobs::{JobActionRequest, JobsPage};
use serde::Deserialize;

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{Invalid, JsonBody, SignedIn, Viewer};
use crate::services::jobs::JobError;

/// A refusal: 400 `{ ok: false, error }`, 500 for the server's own failure, or 401 `signIn`
/// for a stop without a signed-in device.
#[derive(Debug)]
pub(super) enum JobRefusal {
    /// The jobs refused it, or failed.
    Job(JobError),
    /// The request may not make it ([`SignedIn::acting`]).
    Api(ApiError),
}

impl From<ApiError> for JobRefusal {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl IntoResponse for JobRefusal {
    fn into_response(self) -> Response {
        let error = match self {
            Self::Job(error) => error,
            Self::Api(error) => return error.into_response(),
        };
        let (status, error) = match &error {
            JobError::Database(cause) => {
                tracing::error!(error = %cause, "the jobs could not be read");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Something went wrong on the server.".to_owned(),
                )
            }
            other => (StatusCode::BAD_REQUEST, other.to_string()),
        };
        let body = ErrorBody {
            ok: Some(false),
            error,
            sign_in: false,
        };
        (status, Json(body)).into_response()
    }
}

/// `?fresh` looks again now rather than answering the last look (kept 4 s).
#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(super) struct FreshQuery {
    /// Present: look again now.
    #[serde(default)]
    fresh: Option<String>,
}

/// Every background job running now on the runners' machines: the runners' sessions with a
/// turn on, and the runs and scheduled jobs each runner lists of its machine.
#[utoipa::path(
    get,
    path = "/api/jobs",
    tag = "runners",
    security(("bearer" = [])),
    params(FreshQuery),
    responses(
        (status = 200, description = "The jobs", body = JobsPage),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
    )
)]
pub(super) async fn jobs(
    State(app): State<AppState>,
    _viewer: Viewer,
    query: Result<Query<FreshQuery>, QueryRejection>,
) -> Result<Json<JobsPage>, JobRefusal> {
    let fresh = query.is_ok_and(|Query(query)| query.fresh.is_some());
    app.jobs()
        .now(fresh)
        .await
        .map(Json)
        .map_err(|error| JobRefusal::Job(JobError::Database(error)))
}

/// Stops a job (`{ id }`): a session on its runner, a run by its runner.
#[utoipa::path(
    post,
    path = "/api/jobs/stop",
    tag = "runners",
    security(("bearer" = [])),
    request_body = JobActionRequest,
    responses(
        (status = 200, description = "Stopped", body = OkReply),
        (status = 400, description = "Gone, not one to stop from here, or its runner refused (`ok: false`)", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
    )
)]
pub(super) async fn stop(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    body: Result<JsonBody, Invalid>,
) -> Result<Json<OkReply>, JobRefusal> {
    SignedIn::acting(device)?;
    // Read leniently: no id stops nothing (and says so).
    let id = body
        .ok()
        .and_then(|JsonBody(body)| {
            body.get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default();
    app.jobs()
        .stop(&id)
        .await
        .map(|()| Json(OkReply::OK))
        .map_err(JobRefusal::Job)
}
