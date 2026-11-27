//! The parties' routes: `GET`/`POST /api/parties`, `GET /api/parties/{id}`, its live stream,
//! and the person's controls (`say`, `pause`, `resume`, `stop`, `caps`, `delete`). A refusal
//! answers `{ ok: false, error }`: 400 for what the rules don't
//! take, 404 for a party that isn't there.

use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderName, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use emberhut_api::error::ErrorBody;
use emberhut_api::live::PING_INTERVAL_SECS;
use emberhut_api::parties::{CapsRequest, PartyPage};
use emberhut_api::party::{PartiesPage, PartyEvent as PartyLiveEvent, PartyReply};
use emberhut_core::PartyId;
use emberhut_core::party::{NewParty, PersonaDraft};
use futures_util::Stream;
use serde_json::Value;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{JsonBody, SignedIn, Viewer};
use crate::services::parties::{PartyError, PartyNews, PartyService};

/// Asks proxies (nginx) not to buffer the stream.
const NO_BUFFERING: (HeaderName, &str) = (HeaderName::from_static("x-accel-buffering"), "no");

/// The person's parties (the most recently changed first), the ready-made personas, their own
/// from past parties, the models and the defaults.
#[utoipa::path(
    get,
    path = "/api/parties",
    tag = "parties",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The parties", body = PartiesPage),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
    )
)]
pub(super) async fn list(
    State(app): State<AppState>,
    _viewer: Viewer,
) -> Result<Json<PartiesPage>, ApiError> {
    Ok(Json(app.parties().page().await.map_err(fault)?))
}

/// A new party, which starts talking at once.
#[utoipa::path(
    post,
    path = "/api/parties",
    tag = "parties",
    security(("bearer" = [])),
    request_body = emberhut_api::party::NewParty,
    responses(
        (status = 200, description = "Made, and talking", body = PartyReply),
        (
            status = 400,
            description = "A party the rules don't take: no topic, 2 to 5 personas with names of \
                their own (`{ ok: false, error }`)",
            body = ErrorBody
        ),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 413, description = "The body is over the server's limit", body = ErrorBody),
    )
)]
pub(super) async fn create(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    JsonBody(body): JsonBody,
) -> Result<Json<PartyReply>, PartyFailure> {
    super::acting(device)?;
    let personas = body
        .get("personas")
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |list| {
            list.iter()
                .map(|p| PersonaDraft {
                    name: text(p, "name").unwrap_or_default(),
                    role: text(p, "role").unwrap_or_default(),
                    model: text(p, "model").unwrap_or_default(),
                })
                .collect()
        });
    let new = NewParty {
        topic: text(&body, "topic").unwrap_or_default(),
        personas,
        title: text(&body, "title"),
        max_turns: number(&body, "maxTurns"),
        max_tokens: number(&body, "maxTokens"),
    };
    let party = app.parties().create(&new, true).await?;
    Ok(Json(PartyReply {
        ok: true,
        party: Some(party),
    }))
}

/// The party and its talk, oldest first.
#[utoipa::path(
    get,
    path = "/api/parties/{id}",
    tag = "parties",
    security(("bearer" = [])),
    params(("id" = i64, Path, description = "The party's id")),
    responses(
        (status = 200, description = "The party and its talk", body = PartyPage),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
        (status = 404, description = "No such party (`{ ok: false, error }`)", body = ErrorBody),
    )
)]
pub(super) async fn get(
    State(app): State<AppState>,
    _viewer: Viewer,
    Path(id): Path<String>,
) -> Result<Json<PartyPage>, PartyFailure> {
    Ok(Json(app.parties().get(party_id(&id)?).await?))
}

/// The party's live talk: a snapshot on each connection, then each change (the party, a line
/// said, who is speaking and their words so far); a `: ping` comment every 15 s.
#[utoipa::path(
    get,
    path = "/api/parties/{id}/live",
    tag = "parties",
    security(("bearer" = [])),
    params(("id" = i64, Path, description = "The party's id")),
    responses(
        (
            status = 200,
            description = "A text/event-stream whose every event's data is a PartyLiveEvent, a \
                snapshot first",
            content_type = "text/event-stream",
            body = PartyLiveEvent
        ),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
        (status = 404, description = "No such party (`{ ok: false, error }`)", body = ErrorBody),
    )
)]
pub(super) async fn live(
    State(app): State<AppState>,
    _viewer: Viewer,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, PartyFailure> {
    let id = party_id(&id)?;
    // Listening before the snapshot is read: nothing said in between is missed.
    let news = app.parties().subscribe();
    let first = app.parties().get(id).await?;
    let events = events(
        app.parties().clone(),
        id,
        first,
        news,
        app.stopping().clone(),
    );
    let keep_alive = KeepAlive::new()
        .interval(Duration::from_secs(PING_INTERVAL_SECS))
        .text("ping");
    Ok(([NO_BUFFERING], Sse::new(events).keep_alive(keep_alive)))
}

/// The snapshot, then party `id`'s news, until the party is gone or the server stops. A
/// listener that falls behind gets a fresh snapshot.
fn events(
    parties: PartyService,
    id: PartyId,
    first: PartyPage,
    news: broadcast::Receiver<PartyNews>,
    stopping: CancellationToken,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let snapshot = |page: PartyPage| PartyLiveEvent::Snapshot {
        party: Some(page.party),
        messages: page.messages,
    };
    let start = snapshot(first);
    futures_util::stream::unfold((Some(start), news), move |(pending, mut news)| {
        let parties = parties.clone();
        let stopping = stopping.clone();
        async move {
            if let Some(event) = pending {
                return Some((Ok(data(&event)), (None, news)));
            }
            loop {
                let got = tokio::select! {
                    got = news.recv() => got,
                    () = stopping.cancelled() => return None,
                };
                match got {
                    Ok(item) if item.party == id => {
                        return Some((Ok(data(&item.event)), (None, news)));
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let page = parties.get(id).await.ok()?;
                        let event = PartyLiveEvent::Snapshot {
                            party: Some(page.party),
                            messages: page.messages,
                        };
                        return Some((Ok(data(&event)), (None, news)));
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        }
    })
}

/// The `data:` line of `event`: its JSON on one line.
fn data(event: &PartyLiveEvent) -> Event {
    let json = serde_json::to_string(event).unwrap_or_else(|_| "null".to_owned());
    Event::default().data(json)
}

/// The person's words join the talk (`kind: say`), or a direction everyone follows (`steer`).
#[utoipa::path(
    post,
    path = "/api/parties/{id}/say",
    tag = "parties",
    security(("bearer" = [])),
    params(("id" = i64, Path, description = "The party's id")),
    request_body = emberhut_api::party::PartySay,
    responses(
        (status = 200, description = "Said; a party that waited talks again", body = PartyReply),
        (status = 400, description = "Blank or over 4,000 characters (`{ ok: false, error }`)", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such party (`{ ok: false, error }`)", body = ErrorBody),
        (status = 413, description = "The body is over the server's limit", body = ErrorBody),
    )
)]
pub(super) async fn say(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Path(id): Path<String>,
    JsonBody(body): JsonBody,
) -> Result<Json<PartyReply>, PartyFailure> {
    super::acting(device)?;
    let id = party_id(&id)?;
    let said = text(&body, "text").unwrap_or_default();
    let kind = text(&body, "kind").unwrap_or_default();
    reply(app.parties().say(id, &said, &kind).await)
}

/// Pauses the party once the turn being said ends.
#[utoipa::path(
    post,
    path = "/api/parties/{id}/pause",
    tag = "parties",
    security(("bearer" = [])),
    params(("id" = i64, Path, description = "The party's id")),
    responses(
        (status = 200, description = "Paused (or as it was, if it wasn't running)", body = PartyReply),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such party (`{ ok: false, error }`)", body = ErrorBody),
    )
)]
pub(super) async fn pause(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Path(id): Path<String>,
) -> Result<Json<PartyReply>, PartyFailure> {
    super::acting(device)?;
    reply(app.parties().pause(party_id(&id)?).await)
}

/// Carries on: a paused party where it was, an ended one with room for more turns.
#[utoipa::path(
    post,
    path = "/api/parties/{id}/resume",
    tag = "parties",
    security(("bearer" = [])),
    params(("id" = i64, Path, description = "The party's id")),
    responses(
        (status = 200, description = "Talking again", body = PartyReply),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such party (`{ ok: false, error }`)", body = ErrorBody),
    )
)]
pub(super) async fn resume(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Path(id): Path<String>,
) -> Result<Json<PartyReply>, PartyFailure> {
    super::acting(device)?;
    reply(app.parties().resume(party_id(&id)?).await)
}

/// Stops the party, cutting the turn in flight (what was said is kept).
#[utoipa::path(
    post,
    path = "/api/parties/{id}/stop",
    tag = "parties",
    security(("bearer" = [])),
    params(("id" = i64, Path, description = "The party's id")),
    responses(
        (status = 200, description = "Stopped", body = PartyReply),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such party (`{ ok: false, error }`)", body = ErrorBody),
    )
)]
pub(super) async fn stop(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Path(id): Path<String>,
) -> Result<Json<PartyReply>, PartyFailure> {
    super::acting(device)?;
    reply(app.parties().stop(party_id(&id)?).await)
}

/// New caps: turns 1 to 200, tokens 5,000 to 5,000,000; a cap left out stays.
#[utoipa::path(
    post,
    path = "/api/parties/{id}/caps",
    tag = "parties",
    security(("bearer" = [])),
    params(("id" = i64, Path, description = "The party's id")),
    request_body = CapsRequest,
    responses(
        (status = 200, description = "The caps are set", body = PartyReply),
        (status = 400, description = "A body that isn't JSON", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such party (`{ ok: false, error }`)", body = ErrorBody),
        (status = 413, description = "The body is over the server's limit", body = ErrorBody),
    )
)]
pub(super) async fn caps(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Path(id): Path<String>,
    JsonBody(body): JsonBody,
) -> Result<Json<PartyReply>, PartyFailure> {
    super::acting(device)?;
    let id = party_id(&id)?;
    let turns = number(&body, "maxTurns");
    let tokens = number(&body, "maxTokens");
    reply(app.parties().caps(id, turns, tokens).await)
}

/// Deletes the party and its talk (`party` is `null` in the answer).
#[utoipa::path(
    post,
    path = "/api/parties/{id}/delete",
    tag = "parties",
    security(("bearer" = [])),
    params(("id" = i64, Path, description = "The party's id")),
    responses(
        (status = 200, description = "Deleted", body = PartyReply),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such party (`{ ok: false, error }`)", body = ErrorBody),
    )
)]
pub(super) async fn delete(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Path(id): Path<String>,
) -> Result<Json<PartyReply>, PartyFailure> {
    super::acting(device)?;
    app.parties().delete(party_id(&id)?).await?;
    Ok(Json(PartyReply {
        ok: true,
        party: None,
    }))
}

fn reply(
    result: Result<emberhut_api::party::Party, PartyError>,
) -> Result<Json<PartyReply>, PartyFailure> {
    Ok(Json(PartyReply {
        ok: true,
        party: Some(result?),
    }))
}

/// A party's id from the path: a number, or no such party.
fn party_id(text: &str) -> Result<PartyId, PartyFailure> {
    text.parse::<i64>()
        .ok()
        .filter(|id| *id > 0 && text.bytes().all(|b| b.is_ascii_digit()))
        .map(PartyId::new)
        .ok_or(PartyFailure::Party(PartyError::NotFound))
}

/// A string field of `body`; anything else is none.
fn text(body: &Value, key: &str) -> Option<String> {
    body.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// A number field of `body`; anything else is none.
fn number(body: &Value, key: &str) -> Option<f64> {
    body.get(key).and_then(Value::as_f64)
}

fn fault(error: emberhut_db::DbError) -> ApiError {
    ApiError::State(error.into())
}

/// A party change refused or failed, or the server's own answer (no session).
#[derive(Debug)]
pub(super) enum PartyFailure {
    /// The parties' answer.
    Party(PartyError),
    /// The server's own.
    Api(ApiError),
}

impl From<PartyError> for PartyFailure {
    fn from(error: PartyError) -> Self {
        Self::Party(error)
    }
}

impl From<ApiError> for PartyFailure {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl IntoResponse for PartyFailure {
    fn into_response(self) -> Response {
        let error = match self {
            Self::Party(error) => error,
            Self::Api(error) => return error.into_response(),
        };
        let said = error.to_string();
        let status = match error {
            PartyError::Refused(_) => StatusCode::BAD_REQUEST,
            PartyError::NotFound => StatusCode::NOT_FOUND,
            PartyError::Database(error) => return fault(error).into_response(),
        };
        let body = ErrorBody {
            ok: Some(false),
            ..ErrorBody::new(said)
        };
        (status, Json(body)).into_response()
    }
}
