//! `GET` [`emberhut_api::threads::ACTIVITY`] `?machine=&thread=`: a coding-thread app thread's
//! latest steps, read-only, for the sessions list.

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use emberhut_api::error::ErrorBody;
use emberhut_api::threads::{ActivityQuery, ThreadActivity};

use crate::app::AppState;
use crate::extract::Viewer;
use crate::services::coding_app::CodingAppError;
use crate::services::threads::ThreadError;

/// The thread's title, status and latest steps, read from the coding-thread app now.
#[utoipa::path(
    get,
    // `emberhut_api::wire::THREAD_ACTIVITY_PATH`; the macro takes only a literal path, and a
    // test checks the two agree.
    path = "/api/t3/activity",
    tag = "threads",
    security(("bearer" = [])),
    params(ActivityQuery),
    responses(
        (status = 200, description = "The thread's latest steps", body = ThreadActivity),
        (
            status = 400,
            description = "No coding-thread app on that machine (`ok: false`)",
            body = ErrorBody
        ),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
        (
            status = 502,
            description = "The coding-thread app could not be read: its error (`ok: false`)",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn activity(
    State(app): State<AppState>,
    _viewer: Viewer,
    query: Result<Query<ActivityQuery>, QueryRejection>,
) -> Result<Json<ThreadActivity>, ActivityRefusal> {
    // A query that can't be read names no machine, as a missing one: 400.
    let query = query.map_or_default(|Query(query)| query);
    app.threads()
        .activity(&query.machine, &query.thread)
        .await
        .map(Json)
        .map_err(ActivityRefusal)
}

/// Why the activity could not be read, answered `{ ok: false, error }`: 400 for a machine
/// without the app, 502 with the app's error otherwise.
#[derive(Debug)]
pub(super) struct ActivityRefusal(ThreadError);

impl IntoResponse for ActivityRefusal {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            ThreadError::App(CodingAppError::UnknownMachine) => StatusCode::BAD_REQUEST,
            ThreadError::App(_) => StatusCode::BAD_GATEWAY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let error = if status.is_server_error() && status != StatusCode::BAD_GATEWAY {
            tracing::error!(error = %self.0, "could not read a thread's activity");
            "Something went wrong on the server.".to_owned()
        } else {
            if status == StatusCode::BAD_GATEWAY {
                tracing::warn!(error = %self.0, "the coding-thread app could not be read");
            }
            self.0.to_string()
        };
        let body = ErrorBody {
            ok: Some(false),
            error,
            sign_in: false,
        };
        (status, Json(body)).into_response()
    }
}
