//! `/family/...`: a family member's own way in, passed on by the household server on this
//! machine with the device's key as the bearer. Their state, history, live stream, talking,
//! their phone, marking their room read, and their own theme. A request from another machine
//! is refused (403); a key the household server doesn't say is a family member's is 401.
//!
//! While family rooms are off ([`FAMILY_ROOMS`]) none of these routes is served: every
//! `/family/...` request is 501, before its caller or key is looked at.

use std::net::SocketAddr;
use std::time::Duration;

use axum::Json;
use axum::extract::{ConnectInfo, FromRequestParts, RawQuery, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::sse::{KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use emberhut_api::error::{ErrorBody, OkReply};
use emberhut_api::family::{
    FamilyDevice, FamilyHistory, FamilyLine, FamilyReminder, FamilyRoom, FamilySeenReply,
    FamilyState, FamilyTalk, FamilyTalkReply,
};
use emberhut_api::live::{LiveEvent, PING_INTERVAL_SECS};
use emberhut_api::theme::{ThemeChange, ThemePage, ThemeSetReply};
use emberhut_core::message::Message;
use emberhut_core::{ETag, MessageId, RoomId};
use serde_json::Value;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::app::AppState;
use crate::convert::{message_to_api, room_to_api, timestamp_to_api};
use crate::error::ApiError;
use crate::extract::JsonBody;
use crate::services::family::{FAMILY_ROOMS, FamilyError, NOT_AVAILABLE};
use crate::services::theme::{ThemeError, page};

/// Asks proxies (nginx) not to buffer the stream.
const NO_BUFFERING: (HeaderName, &str) = (HeaderName::from_static("x-accel-buffering"), "no");

/// How many lines the state and a history page hold.
const PAGE: u64 = 60;

/// The longest message kept, in characters.
const MAX_TALK: usize = 8000;

/// A family member, by the household server's word, calling from this machine.
#[derive(Clone, Debug)]
pub(super) struct FamilyMember(String);

impl FromRequestParts<AppState> for FamilyMember {
    type Rejection = FamilyFailure;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, FamilyFailure> {
        // Served on a socket, the peer is known: only this machine (the household server)
        // passes family requests on, over IPv4 or IPv6 (`::ffff:127.0.0.1` is this machine
        // too).
        let remote = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .is_some_and(|ConnectInfo(peer)| !peer.ip().to_canonical().is_loopback());
        if remote {
            return Err(refuse(
                StatusCode::FORBIDDEN,
                "through the household app only",
            ));
        }
        let key = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map_or("", |value| {
                let value = value.trim();
                value
                    .get(..7)
                    .filter(|scheme| scheme.eq_ignore_ascii_case("bearer "))
                    .map_or(value, |_| value[7..].trim_start())
            });
        match state.family().who_is(key).await {
            Some(person) => Ok(Self(person)),
            None => Err(refuse(
                StatusCode::UNAUTHORIZED,
                "Only a family member's own devices.",
            )),
        }
    }
}

/// The family routes: each of them while family rooms are on; else one refusal for any of
/// their paths.
pub(super) fn routes(router: OpenApiRouter<AppState>) -> OpenApiRouter<AppState> {
    if FAMILY_ROOMS {
        router
            .routes(routes!(state))
            .routes(routes!(history))
            .routes(routes!(live))
            .routes(routes!(talk))
            .routes(routes!(devices))
            .routes(routes!(seen))
            .routes(routes!(theme, set_theme))
    } else {
        router.routes(routes!(not_available, not_available_to_post))
    }
}

/// Family rooms are not available yet: any `/family/...` read is 501.
#[utoipa::path(
    get,
    path = "/family/{route}",
    tag = "family",
    security(("bearer" = [])),
    params(("route" = String, Path, description = "A family route: state, history, live or theme")),
    responses(
        (status = 501, description = "Family rooms are not available yet", body = ErrorBody),
    )
)]
pub(super) async fn not_available() -> FamilyFailure {
    unavailable()
}

/// Family rooms are not available yet: any `/family/...` request that changes something is
/// 501.
#[utoipa::path(
    post,
    path = "/family/{route}",
    tag = "family",
    security(("bearer" = [])),
    params(("route" = String, Path, description = "A family route: talk, devices, seen or theme")),
    responses(
        (status = 501, description = "Family rooms are not available yet", body = ErrorBody),
    )
)]
pub(super) async fn not_available_to_post() -> FamilyFailure {
    unavailable()
}

/// The refusal while family rooms are off.
fn unavailable() -> FamilyFailure {
    refuse(StatusCode::NOT_IMPLEMENTED, NOT_AVAILABLE)
}

/// A family request that was not done: refused as `error` says, or failed.
fn failed(error: FamilyError) -> FamilyFailure {
    match error {
        FamilyError::NotAvailable => unavailable(),
        FamilyError::Token(error) => refuse(StatusCode::BAD_REQUEST, &error.to_string()),
        FamilyError::Database(error) => fault(error),
    }
}

fn refuse(status: StatusCode, error: &str) -> FamilyFailure {
    FamilyFailure::Refused(status, error.to_owned())
}

fn fault(error: impl Into<crate::services::state::StateError>) -> FamilyFailure {
    FamilyFailure::Api(ApiError::State(error.into()))
}

/// A family request refused (`{ error }`) or failed.
#[derive(Debug)]
pub(super) enum FamilyFailure {
    Refused(StatusCode, String),
    Api(ApiError),
}

impl From<ApiError> for FamilyFailure {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl IntoResponse for FamilyFailure {
    fn into_response(self) -> Response {
        match self {
            Self::Refused(status, error) => (status, Json(ErrorBody::new(error))).into_response(),
            Self::Api(error) => error.into_response(),
        }
    }
}

/// `?room=` and `?before=` as numbers, read leniently (anything else is none).
fn query_number(query: Option<&str>, name: &str) -> Option<i64> {
    let query = query?;
    form_urlencoded::parse(query.as_bytes())
        .find(|(key, _)| key == name)
        .and_then(|(_, value)| value.trim().parse::<f64>().ok())
        .filter(|n| n.is_finite() && *n >= 1.0 && n.fract() == 0.0 && *n < 9.0e15)
        .map(|n| {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a whole number below 2^53, checked just above"
            )]
            let whole = n as i64;
            whole
        })
}

async fn room_of(
    app: &AppState,
    person: &str,
    query: Option<&str>,
) -> Result<RoomId, FamilyFailure> {
    let asked = query_number(query, "room").map(RoomId::new);
    let (room, _) = app.family().rooms(person, asked).await.map_err(failed)?;
    if room.id == RoomId::MAIN {
        // Never: their rooms are made with their name on them.
        return Err(refuse(StatusCode::FORBIDDEN, "not their room"));
    }
    Ok(room.id)
}

fn line(message: &Message) -> FamilyLine {
    FamilyLine {
        id: message_to_api(message.id),
        mine: message.author == emberhut_core::message::Author::User,
        text: message.text.clone(),
        at: timestamp_to_api(message.created_at),
    }
}

/// Who they are, the room, their rooms, its latest lines, what the assistant is doing there,
/// their reminders and their theme's choice. Tagged: `If-None-Match` with the tag gets a 304.
#[utoipa::path(
    get,
    path = "/family/state",
    tag = "family",
    security(("bearer" = [])),
    params(("room" = Option<i64>, Query, description = "One of their rooms; none: their Home room")),
    responses(
        (status = 200, description = "Their state", body = FamilyState),
        (status = 304, description = "Unchanged since the tag in If-None-Match"),
        (status = 401, description = "Not a family member's key", body = ErrorBody),
        (status = 403, description = "Not from this machine", body = ErrorBody),
    )
)]
pub(super) async fn state(
    State(app): State<AppState>,
    FamilyMember(person): FamilyMember,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Result<Response, FamilyFailure> {
    let room = room_of(&app, &person, query.as_deref()).await?;
    // They have it open: its answers skip their phone's push.
    app.presence().looking(room);
    let (_, rooms) = app
        .family()
        .rooms(&person, Some(room))
        .await
        .map_err(failed)?;
    let chat = app
        .family()
        .store()
        .conversation(room, None, PAGE)
        .await
        .map_err(fault)?;
    let theme = app.family().theme(&person).choice().await.map_err(fault)?;
    let live = app.live().now(room);
    let body = FamilyState {
        me: person,
        room: room_to_api(room),
        rooms: rooms
            .iter()
            .map(|r| FamilyRoom {
                id: room_to_api(r.id),
                name: r.name.clone(),
            })
            .collect(),
        chat: chat.iter().map(line).collect(),
        working: live.is_some(),
        live,
        reminders: app
            .family()
            .reminders(room)
            .await
            .into_iter()
            .zip(1..)
            .map(|(r, id)| FamilyReminder {
                id,
                due: timestamp_to_api(r.due),
                text: r.text,
            })
            .collect(),
        theme,
    };
    let bytes = serde_json::to_vec(&body).map_err(ApiError::Encode)?;
    let tag = ETag::of(&bytes);
    let tag_value = HeaderValue::from_str(tag.as_str()).map_err(|_| ApiError::Internal)?;
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| tag.matches_if_none_match(value));
    if unchanged {
        return Ok((StatusCode::NOT_MODIFIED, [(header::ETAG, tag_value)]).into_response());
    }
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (header::ETAG, tag_value),
        ],
        bytes,
    )
        .into_response())
}

/// An older page of their conversation: the 60 lines before `before`, oldest first.
#[utoipa::path(
    get,
    path = "/family/history",
    tag = "family",
    security(("bearer" = [])),
    params(
        ("room" = Option<i64>, Query, description = "One of their rooms; none: their Home room"),
        ("before" = Option<i64>, Query, description = "Only lines older than this one"),
    ),
    responses(
        (status = 200, description = "The page", body = FamilyHistory),
        (status = 401, description = "Not a family member's key", body = ErrorBody),
        (status = 403, description = "Not from this machine", body = ErrorBody),
    )
)]
pub(super) async fn history(
    State(app): State<AppState>,
    FamilyMember(person): FamilyMember,
    RawQuery(query): RawQuery,
) -> Result<Json<FamilyHistory>, FamilyFailure> {
    let room = room_of(&app, &person, query.as_deref()).await?;
    let before = query_number(query.as_deref(), "before").map(MessageId::new);
    let chat = app
        .family()
        .store()
        .conversation(room, before, PAGE)
        .await
        .map_err(fault)?;
    Ok(Json(FamilyHistory {
        chat: chat.iter().map(line).collect(),
    }))
}

/// What the assistant is doing in their room, as server-sent events: as it is now, then each
/// change (`null` when its turn ends); a `: ping` comment every 15 s.
#[utoipa::path(
    get,
    path = "/family/live",
    tag = "family",
    security(("bearer" = [])),
    params(("room" = Option<i64>, Query, description = "One of their rooms; none: their Home room")),
    responses(
        (
            status = 200,
            description = "A text/event-stream whose every event's data is a LiveEvent",
            content_type = "text/event-stream",
            body = LiveEvent
        ),
        (status = 401, description = "Not a family member's key", body = ErrorBody),
        (status = 403, description = "Not from this machine", body = ErrorBody),
    )
)]
pub(super) async fn live(
    State(app): State<AppState>,
    FamilyMember(person): FamilyMember,
    RawQuery(query): RawQuery,
) -> Result<impl IntoResponse, FamilyFailure> {
    let room = room_of(&app, &person, query.as_deref()).await?;
    let events = super::live::events(app.live().subscribe(room), app.stopping().clone());
    let keep_alive = KeepAlive::new()
        .interval(Duration::from_secs(PING_INTERVAL_SECS))
        .text("ping");
    Ok(([NO_BUFFERING], Sse::new(events).keep_alive(keep_alive)))
}

/// What they say, to the assistant in their room.
#[utoipa::path(
    post,
    path = "/family/talk",
    tag = "family",
    security(("bearer" = [])),
    params(("room" = Option<i64>, Query, description = "One of their rooms; none: their Home room")),
    request_body = FamilyTalk,
    responses(
        (status = 200, description = "Said; the assistant answers in the room", body = FamilyTalkReply),
        (status = 400, description = "Nothing said, or a body that isn't JSON", body = ErrorBody),
        (status = 401, description = "Not a family member's key", body = ErrorBody),
        (status = 403, description = "Not from this machine", body = ErrorBody),
        (status = 413, description = "The body is over the server's limit", body = ErrorBody),
        (status = 429, description = "Too many messages wait in the room", body = ErrorBody),
        (status = 503, description = "The server is shutting down", body = ErrorBody),
    )
)]
pub(super) async fn talk(
    State(app): State<AppState>,
    FamilyMember(person): FamilyMember,
    RawQuery(query): RawQuery,
    JsonBody(body): JsonBody,
) -> Result<Json<FamilyTalkReply>, FamilyFailure> {
    let room = room_of(&app, &person, query.as_deref()).await?;
    let text: String = body
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .chars()
        .take(MAX_TALK)
        .collect();
    if text.is_empty() {
        return Err(refuse(StatusCode::BAD_REQUEST, "say something"));
    }
    let said = app
        .talk()
        .talk(room, text, None)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(FamilyTalkReply {
        ok: true,
        id: message_to_api(said.id),
    }))
}

/// Their iPhone's push address. Kept apart from the owner's phones: the owner's alerts never
/// reach it.
#[utoipa::path(
    post,
    path = "/family/devices",
    tag = "family",
    security(("bearer" = [])),
    request_body = FamilyDevice,
    responses(
        (status = 200, description = "Kept", body = OkReply),
        (status = 400, description = "Not a device token, or a body that isn't JSON", body = ErrorBody),
        (status = 401, description = "Not a family member's key", body = ErrorBody),
        (status = 403, description = "Not from this machine", body = ErrorBody),
        (status = 413, description = "The body is over the server's limit", body = ErrorBody),
    )
)]
pub(super) async fn devices(
    State(app): State<AppState>,
    FamilyMember(person): FamilyMember,
    JsonBody(body): JsonBody,
) -> Result<Json<OkReply>, FamilyFailure> {
    let token = body
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let env = body.get("env").and_then(Value::as_str);
    app.family()
        .register_device(&person, token, env)
        .await
        .map_err(failed)?;
    Ok(Json(OkReply::OK))
}

/// Marks their room read.
#[utoipa::path(
    post,
    path = "/family/seen",
    tag = "family",
    security(("bearer" = [])),
    params(("room" = Option<i64>, Query, description = "One of their rooms; none: their Home room")),
    responses(
        (status = 200, description = "Marked read", body = FamilySeenReply),
        (status = 401, description = "Not a family member's key", body = ErrorBody),
        (status = 403, description = "Not from this machine", body = ErrorBody),
    )
)]
pub(super) async fn seen(
    State(app): State<AppState>,
    FamilyMember(person): FamilyMember,
    RawQuery(query): RawQuery,
) -> Result<Json<FamilySeenReply>, FamilyFailure> {
    let room = room_of(&app, &person, query.as_deref()).await?;
    app.talk().seen(room).await.map_err(ApiError::from)?;
    Ok(Json(FamilySeenReply {
        ok: true,
        at: timestamp_to_api(app.family().now()),
    }))
}

/// Their own theme: the choice, its colours in both appearances, and every preset's.
#[utoipa::path(
    get,
    path = "/family/theme",
    tag = "family",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Their theme page", body = ThemePage),
        (status = 401, description = "Not a family member's key", body = ErrorBody),
        (status = 403, description = "Not from this machine", body = ErrorBody),
    )
)]
pub(super) async fn theme(
    State(app): State<AppState>,
    FamilyMember(person): FamilyMember,
) -> Result<Json<ThemePage>, FamilyFailure> {
    Ok(Json(
        app.family().theme(&person).page().await.map_err(fault)?,
    ))
}

/// Changes their own theme (as `POST /api/theme`), and answers its page.
#[utoipa::path(
    post,
    path = "/family/theme",
    tag = "family",
    security(("bearer" = [])),
    request_body = ThemeChange,
    responses(
        (status = 200, description = "Changed (or worked out, with preview)", body = ThemeSetReply),
        (status = 400, description = "A value the theme doesn't take, or a body that isn't JSON", body = ErrorBody),
        (status = 401, description = "Not a family member's key", body = ErrorBody),
        (status = 403, description = "Not from this machine", body = ErrorBody),
        (status = 413, description = "The body is over the server's limit", body = ErrorBody),
    )
)]
pub(super) async fn set_theme(
    State(app): State<AppState>,
    FamilyMember(person): FamilyMember,
    JsonBody(body): JsonBody,
) -> Result<Json<ThemeSetReply>, FamilyFailure> {
    let change = body;
    let preview = change
        .get("preview")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let choice = app
        .family()
        .theme(&person)
        .change(&change, preview)
        .await
        .map_err(|error| match error {
            ThemeError::Refused(refusal) => refuse(StatusCode::BAD_REQUEST, &refusal.to_string()),
            ThemeError::Database(error) => fault(error),
        })?;
    Ok(Json(ThemeSetReply {
        ok: true,
        page: page(&choice),
    }))
}
