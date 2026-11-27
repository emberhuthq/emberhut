//! `GET /api/calendar?from=&to=`: the owner's calendar for the app's Day, Week and Month.

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use emberhut_api::calendar::{CalendarPage, CalendarQuery};
use emberhut_api::error::ErrorBody;

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::Viewer;
use crate::services::calendar::CalendarViewError;

/// The range as read: leniently, so a missing day is the range's refusal with its words (the
/// contract's typed form is [`CalendarQuery`]).
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(super) struct RangeQuery {
    /// The first day.
    #[serde(default)]
    from: Option<String>,
    /// The last day.
    #[serde(default)]
    to: Option<String>,
}

/// The events of `from..=to` (at most 62 days) and what there is to work on, each on its day;
/// `note` says when the Mac didn't answer and the events are what was last seen.
#[utoipa::path(
    get,
    path = "/api/calendar",
    tag = "calendar",
    security(("bearer" = [])),
    params(CalendarQuery),
    responses(
        (status = 200, description = "The days' events and items", body = CalendarPage),
        (
            status = 400,
            description = "`from` and `to` aren't two days in order, or are over 62 days apart",
            body = ErrorBody
        ),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn calendar(
    State(app): State<AppState>,
    _viewer: Viewer,
    query: Result<Query<RangeQuery>, QueryRejection>,
) -> Response {
    let query = query.map_or_default(|Query(query)| query);
    let from = query.from.unwrap_or_default();
    let to = query.to.unwrap_or_default();
    match app.calendar().view(&from, &to).await {
        Ok(view) => Json(view).into_response(),
        Err(CalendarViewError::Range(error)) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new(error.to_string())),
        )
            .into_response(),
        Err(error) => {
            tracing::error!(%error, "the calendar could not be read");
            ApiError::Internal.into_response()
        }
    }
}
