//! `GET /api/history?room=&before=&after=`: a page of a room's chat.

use axum::Json;
use axum::extract::{RawQuery, State};
use emberhut_api::chat::{ChatPage, HistoryQuery};
use emberhut_api::error::ErrorBody;
use emberhut_core::message;

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{RoomParam, Viewer};

/// The first value of the query's `key` (as `searchParams.get` reads it): a repeated one
/// counts once, by its first value.
fn first(query: Option<&str>, key: &str) -> Option<String> {
    form_urlencoded::parse(query?.as_bytes())
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}

/// Up to 60 of a room's messages, oldest first: the newest before `before` (older messages,
/// for "Show earlier"), or the newest after `after` (what came since), or the newest of all.
/// A room that doesn't exist reads as Main.
#[utoipa::path(
    get,
    path = "/api/history",
    tag = "state",
    security(("bearer" = [])),
    params(HistoryQuery),
    responses(
        (status = 200, description = "The page", body = ChatPage),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn history(
    State(app): State<AppState>,
    _viewer: Viewer,
    RoomParam(asked): RoomParam,
    RawQuery(raw): RawQuery,
) -> Result<Json<ChatPage>, ApiError> {
    // Read leniently ([`message::HistoryQuery::lenient`]):
    // a bound that isn't a number is no bound, not a 400.
    let before = first(raw.as_deref(), "before");
    let after = first(raw.as_deref(), "after");
    let query = message::HistoryQuery::lenient(before.as_deref(), after.as_deref());
    let room = app.state().resolve(asked).await?;
    let page = app
        .history()
        .page(room, query)
        .await
        .map_err(|error| ApiError::State(error.into()))?;
    Ok(Json(page))
}
