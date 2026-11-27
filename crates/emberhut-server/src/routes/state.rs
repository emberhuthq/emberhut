//! `GET /api/state?room=`: everything the app shows for a room, with an `ETag`.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use emberhut_api::error::ErrorBody;
use emberhut_api::headers;
use emberhut_api::live::RoomQuery;
use emberhut_core::{ETag, RoomId};

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{RoomParam, Viewer};

/// The state of a room. Sent back in `If-None-Match`, its `ETag` gets 304 while it is unchanged.
#[utoipa::path(
    get,
    path = "/api/state",
    tag = "state",
    security(("bearer" = [])),
    params(
        RoomQuery,
        (
            "if-none-match" = Option<String>, Header,
            description = "The ETag of the state the client holds"
        ),
        // `emberhut_api::wire::LOOKING_HEADER`; the macro takes only a literal name, and a test
        // checks the two agree.
        (
            "x-emberhut-looking" = Option<String>, Header,
            description = "The room on screen in front of the person, while it is"
        ),
    ),
    responses(
        (
            status = 200,
            description = "The room's state",
            body = emberhut_api::state::State,
            headers(("etag" = String, description = "The state's fingerprint (a weak tag)"))
        ),
        (
            status = 304,
            description = "Unchanged since the ETag sent",
            headers(("etag" = String, description = "The state's fingerprint (a weak tag)"))
        ),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn state(
    State(app): State<AppState>,
    Viewer(viewer): Viewer,
    RoomParam(asked): RoomParam,
    request: HeaderMap,
) -> Result<Response, ApiError> {
    if let Some(looking) = looking_at(&request) {
        app.presence()
            .looking(app.state().resolve(Some(looking)).await?);
    }
    let room = app.state().resolve(asked).await?;
    let state = app.state().state_for(room, viewer).await?;
    let body = serde_json::to_vec(&state).map_err(ApiError::Encode)?;
    let tag = ETag::of(&body);
    let tag_header = HeaderValue::from_str(tag.as_str()).map_err(|_| ApiError::Internal)?;

    let unchanged = request
        .get(headers::IF_NONE_MATCH)
        .and_then(|held| held.to_str().ok())
        .is_some_and(|held| tag.matches_if_none_match(held));
    let mut response = if unchanged {
        Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .body(Body::empty())
    } else {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
    }
    .map_err(|_| ApiError::Internal)?;
    response.headers_mut().insert(header::ETAG, tag_header);
    Ok(response)
}

/// The room the [`headers::LOOKING`] header names, when it names one.
fn looking_at(request: &HeaderMap) -> Option<RoomId> {
    request
        .get(headers::LOOKING)?
        .to_str()
        .ok()?
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|room| *room > 0)
        .map(RoomId::new)
}
