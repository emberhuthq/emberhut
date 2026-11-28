//! The runners: what they report (`POST /api/runner/events`, `POST /api/runner/machine`, `GET
//! /api/runner/mcp`, with their own token), the Machines view (`GET /api/machines`), the sessions
//! list and the person's own new sessions (`/api/runner/sessions`), the folder and branch
//! pickers, and one session's transcript, live stream, reply, stop, interrupt and settings
//! (`/api/runner/sessions/r-{id}/...`), and one of the assistant's workers watched the same way
//! (`/api/runner/sessions/w-{id}/...`).
//!
//! The worker routes read the assistant's workers through
//! [`crate::services::workers_view::WorkerSessions`]. Without a worker runtime (no agent CLI is
//! configured) the server runs with [`crate::services::workers_view::NoWorkers`]: no task has a
//! worker, so every `w-{id}` answers 404 `no such task`.

use axum::Json;
use axum::body::Body;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use emberhut_api::error::{ErrorBody, OkReply};
use emberhut_api::machines::MachinesPage;
use emberhut_api::runners::{
    Listing, NewSession, NewSessionReply, PickerQuery, RunnerReply, SendToSession,
    SessionSettingsReply, SessionsPage, SessionsQuery, SharedMcp, StepsQuery, Transcript,
};
use emberhut_core::RoomId;
use futures_util::StreamExt as _;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{JsonBody, SignedIn, Viewer};
use crate::services::machines::MachineError;
use crate::services::runner_api::RunnerError;
use crate::services::runners::{DEFAULT_RUNNER, DirectSession, Filter, SessionError, SessionId};

/// Asks proxies (nginx) not to buffer a stream.
const NO_BUFFERING: (HeaderName, HeaderValue) = (
    HeaderName::from_static("x-accel-buffering"),
    HeaderValue::from_static("no"),
);

/// A refusal: a status and `{ ok?, error }`, or an error answered as every route answers it.
#[derive(Debug)]
pub(super) enum Refusal {
    /// `status`, with `body` (none: no body).
    Answer {
        /// The status.
        status: StatusCode,
        /// The body, if there is one.
        body: Option<ErrorBody>,
    },
    /// The server's own answer for the error: 401 `signIn` for a request that acts without a
    /// signed-in device ([`SignedIn::acting`]).
    Api(ApiError),
}

impl Refusal {
    /// `status` with `{ ok: false, error }`.
    fn not_ok(status: StatusCode, error: impl Into<String>) -> Self {
        Self::Answer {
            status,
            body: Some(ErrorBody {
                ok: Some(false),
                error: error.into(),
                sign_in: false,
            }),
        }
    }

    /// `status` with `{ error }`.
    fn plain(status: StatusCode, error: impl Into<String>) -> Self {
        Self::Answer {
            status,
            body: Some(ErrorBody::new(error)),
        }
    }

    /// 404 with no body: a path whose session id isn't one (as a missing file answers).
    fn not_found() -> Self {
        Self::Answer {
            status: StatusCode::NOT_FOUND,
            body: None,
        }
    }

    /// A session operation's failure: 400 for what can't be done as asked, 502 for the
    /// runner's (as `{ ok: false, error }`), 500 for the server's own.
    fn of(error: &SessionError) -> Self {
        match error {
            SessionError::Refused(why) => Self::plain(StatusCode::BAD_REQUEST, why.clone()),
            SessionError::Database(cause) => {
                tracing::error!(error = %cause, "a runner session operation failed");
                Self::not_ok(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Something went wrong on the server.",
                )
            }
            other => {
                tracing::warn!(error = %other, "a runner refused or didn't answer");
                Self::not_ok(StatusCode::BAD_GATEWAY, other.to_string())
            }
        }
    }
}

impl From<ApiError> for Refusal {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        match self {
            Self::Answer {
                status,
                body: Some(body),
            } => (status, Json(body)).into_response(),
            Self::Answer { status, body: None } => status.into_response(),
            Self::Api(error) => error.into_response(),
        }
    }
}

/// The runner a report comes from: the one whose token it carries
/// (`Authorization: Bearer <token>`).
#[derive(Debug)]
pub(super) struct Runner(String);

impl FromRequestParts<AppState> for Runner {
    type Rejection = Refusal;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Refusal> {
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, token)| token.trim())
            .filter(|token| !token.is_empty());
        let refused = || Refusal::plain(StatusCode::UNAUTHORIZED, "runners use their token");
        let token = token.ok_or_else(refused)?;
        state
            .runners()
            .runner_of_token(token)
            .await
            .map(Self)
            .ok_or_else(refused)
    }
}

/// One event from a runner about one of its sessions: the session and its turn as they are
/// now, so taking it twice changes nothing.
#[utoipa::path(
    post,
    path = "/api/runner/events",
    tag = "runners",
    security(("bearer" = [])),
    request_body(content = Object, description = "The event: `{ machine, kind, at, session, turn?, ... }`"),
    responses(
        (status = 200, description = "Taken", body = OkReply),
        (status = 400, description = "Not an event of this runner's", body = ErrorBody),
        (status = 401, description = "No runner's token", body = ErrorBody),
    )
)]
pub(super) async fn events(
    State(app): State<AppState>,
    Runner(machine): Runner,
    body: Result<JsonBody, crate::extract::Invalid>,
) -> Result<Json<OkReply>, Refusal> {
    let Ok(JsonBody(event)) = body else {
        return Err(Refusal::plain(StatusCode::BAD_REQUEST, "not an event"));
    };
    app.runners()
        .event(&machine, &event)
        .await
        .map(|()| Json(OkReply::OK))
        .map_err(|error| Refusal::of(&error))
}

/// A runner's sample of its machine (every 20 s): kept as its latest and folded into its
/// 5-minute point.
#[utoipa::path(
    post,
    path = "/api/runner/machine",
    tag = "runners",
    security(("bearer" = [])),
    request_body(content = Object, description = "The sample: `{ machine, host, platform, cpu, memory, gpus, disks, battery, runner, sessions }`"),
    responses(
        (status = 200, description = "Taken", body = OkReply),
        (status = 400, description = "Not a sample of this runner's", body = ErrorBody),
        (status = 401, description = "No runner's token", body = ErrorBody),
    )
)]
pub(super) async fn machine(
    State(app): State<AppState>,
    Runner(machine): Runner,
    body: Result<JsonBody, crate::extract::Invalid>,
) -> Result<Json<OkReply>, Refusal> {
    let Ok(JsonBody(sample)) = body else {
        return Err(Refusal::plain(StatusCode::BAD_REQUEST, "not a sample"));
    };
    match app.machines().take(&machine, &sample).await {
        Ok(()) => Ok(Json(OkReply::OK)),
        Err(MachineError::Refused(why)) => Err(Refusal::plain(StatusCode::BAD_REQUEST, why)),
        Err(MachineError::Database(error)) => {
            tracing::error!(%error, "a machine's sample could not be kept");
            Err(Refusal::plain(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong on the server.",
            ))
        }
    }
}

/// The MCP servers the runners' sessions get too, as the server's operator listed them (none
/// when no list is configured). On a server that keeps tasks' plans, the entry for the server's
/// own MCP endpoint gets the session header too, so each session's calls name the session.
#[utoipa::path(
    get,
    path = "/api/runner/mcp",
    tag = "runners",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The shared MCP servers", body = SharedMcp),
        (status = 401, description = "No runner's token", body = ErrorBody),
    )
)]
pub(super) async fn mcp(State(app): State<AppState>, Runner(machine): Runner) -> Json<SharedMcp> {
    app.runners().heard(&machine);
    let Some(path) = app.runner_mcp() else {
        return Json(SharedMcp::default());
    };
    let servers = match tokio::fs::read_to_string(path).await {
        Ok(text) => {
            if let Ok(Value::Object(mut file)) = serde_json::from_str::<Value>(&text) {
                match file.remove("mcpServers") {
                    Some(Value::Object(servers)) => servers,
                    _ => Map::new(),
                }
            } else {
                tracing::warn!(path = %path.display(), "the runners' MCP list isn't a JSON object");
                Map::new()
            }
        }
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "the runners' MCP list could not be read");
            Map::new()
        }
    };
    let servers = if app.task_plans() {
        servers
            .into_iter()
            .map(|(name, entry)| (name, with_session(entry)))
            .collect()
    } else {
        servers
    };
    Json(SharedMcp {
        mcp_servers: servers,
    })
}

/// `entry` of the shared MCP list, with the session header
/// ([`emberhut_api::wire::mcp::SESSION_HEADER`], the session's id from the variable the runner
/// sets) when its URL is this server's MCP endpoint and it doesn't set that header itself.
fn with_session(mut entry: Value) -> Value {
    let ours = entry.get("url").and_then(Value::as_str).is_some_and(|url| {
        let url = url.split(['?', '#']).next().unwrap_or_default();
        url.trim_end_matches('/')
            .ends_with(emberhut_api::wire::mcp::ROOM_PATH)
    });
    if !ours {
        return entry;
    }
    if let Value::Object(fields) = &mut entry {
        let headers = fields
            .entry("headers")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(headers) = headers
            && !headers
                .keys()
                .any(|name| name.eq_ignore_ascii_case(emberhut_api::wire::mcp::SESSION_HEADER))
        {
            headers.insert(
                emberhut_api::wire::mcp::SESSION_HEADER.to_owned(),
                Value::from(emberhut_api::wire::mcp::SESSION_HEADER_VALUE),
            );
        }
    }
    entry
}

/// `?history=0` leaves each machine's last day out.
#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(super) struct HistoryQuery {
    /// `0`: no history.
    #[serde(default)]
    history: Option<String>,
}

/// Every runner machine's capacity, warnings, sessions and (unless `?history=0`) its last day.
#[utoipa::path(
    get,
    path = "/api/machines",
    tag = "runners",
    security(("bearer" = [])),
    params(HistoryQuery),
    responses(
        (status = 200, description = "The machines", body = MachinesPage),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
    )
)]
pub(super) async fn machines(
    State(app): State<AppState>,
    _viewer: Viewer,
    query: Result<Query<HistoryQuery>, QueryRejection>,
) -> Result<Json<MachinesPage>, Refusal> {
    let history = query.map_or(true, |Query(query)| query.history.as_deref() != Some("0"));
    app.machines()
        .view(history)
        .await
        .map(Json)
        .map_err(|error| Refusal::of(&SessionError::Database(error)))
}

/// The room a query names: a positive whole number, else Main.
fn room_arg(text: Option<&str>) -> Option<RoomId> {
    text.and_then(|text| text.trim().parse::<f64>().ok())
        .filter(|n| n.fract() == 0.0 && *n > 0.0 && *n < 9e15)
        .map(|n| {
            #[expect(clippy::cast_possible_truncation, reason = "a whole number in range")]
            let n = n as i64;
            RoomId::from(n)
        })
}

/// The runner sessions: a task's (`?task=`), a room's (`?room=`; Main: every room's), or the
/// person's own (`?direct`); newest first, 50 at most.
#[utoipa::path(
    get,
    path = "/api/runner/sessions",
    tag = "runners",
    security(("bearer" = [])),
    params(SessionsQuery),
    responses(
        (status = 200, description = "The sessions", body = SessionsPage),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
    )
)]
pub(super) async fn sessions(
    State(app): State<AppState>,
    _viewer: Viewer,
    query: Result<Query<SessionsQuery>, QueryRejection>,
) -> Result<Json<SessionsPage>, Refusal> {
    let query = query.map_or_else(|_| SessionsQuery::default(), |Query(query)| query);
    let task = query
        .task
        .as_deref()
        .and_then(|task| task.trim().parse::<f64>().ok())
        .filter(|n| n.is_finite() && *n != 0.0)
        .map(|n| {
            #[expect(clippy::cast_possible_truncation, reason = "a task id")]
            let n = n.trunc() as i64;
            n
        });
    let filter = Filter {
        task,
        room: query
            .room
            .as_deref()
            .filter(|room| !room.is_empty())
            .map(|room| room_arg(Some(room)).unwrap_or(RoomId::MAIN)),
        direct: query.direct.is_some(),
    };
    app.runners()
        .sessions(filter)
        .await
        .map(|sessions| Json(SessionsPage { sessions }))
        .map_err(|error| Refusal::of(&SessionError::Database(error)))
}

/// One of the person's own sessions: on the machine and in the folder they picked (a worktree,
/// the account, model and effort if they chose them), their first message and files its first
/// turn.
#[utoipa::path(
    post,
    path = "/api/runner/sessions",
    tag = "runners",
    security(("bearer" = [])),
    request_body = NewSession,
    responses(
        (status = 200, description = "Started", body = NewSessionReply),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 409, description = "Not started: no such runner, nothing to start with, or the runner's refusal (`ok: false`)", body = ErrorBody),
    )
)]
pub(super) async fn start(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    body: Result<JsonBody, crate::extract::Invalid>,
) -> Result<Json<NewSessionReply>, Refusal> {
    SignedIn::acting(device)?;
    let ask: NewSession = body
        .ok()
        .and_then(|JsonBody(body)| serde_json::from_value(body).ok())
        .unwrap_or_default();
    let machine = ask
        .machine
        .clone()
        .unwrap_or_else(|| DEFAULT_RUNNER.to_owned());
    let started = app
        .runners()
        .start_direct(DirectSession {
            machine: machine.clone(),
            cwd: ask.cwd.unwrap_or_default(),
            message: ask.text.unwrap_or_default(),
            files: ask.files,
            account: ask.account,
            model: ask.model,
            effort: ask.effort,
            worktree: ask.worktree,
            title: ask.title,
        })
        .await
        .map_err(|error| Refusal::not_ok(StatusCode::CONFLICT, error.to_string()))?;
    let title = started.get("title").map_or_else(
        || "undefined".to_owned(),
        |title| {
            title
                .as_str()
                .map_or_else(|| title.to_string(), str::to_owned)
        },
    );
    let place = app.runners().place(&machine);
    if let Err(error) = app
        .activity()
        .note(format!(
            "Opened a session on the {place} from the app: {title}"
        ))
        .await
    {
        tracing::warn!(%error, "an activity line could not be written");
    }
    let field = |key: &str| started.get(key).and_then(Value::as_str).map(str::to_owned);
    Ok(Json(NewSessionReply {
        ok: true,
        id: field("id"),
        turn: field("turn"),
        session: started,
    }))
}

/// The folders on a runner's machine for the new-session sheet: where to start (no `path`), or
/// one folder's folders.
#[utoipa::path(
    get,
    path = "/api/runner/folders",
    tag = "runners",
    security(("bearer" = [])),
    params(PickerQuery),
    responses(
        (status = 200, description = "The runner's listing", body = Listing),
        (status = 400, description = "No such runner (`ok: false`)", body = ErrorBody),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
        (status = 502, description = "The runner refused or didn't answer (`ok: false`)", body = ErrorBody),
    )
)]
pub(super) async fn folders(
    State(app): State<AppState>,
    _viewer: Viewer,
    query: Result<Query<PickerQuery>, QueryRejection>,
) -> Result<Json<Listing>, Refusal> {
    let query = query.map_or_else(|_| PickerQuery::default(), |Query(query)| query);
    let machine = picked(&app, query.machine.as_deref())?;
    app.runners()
        .folders(&machine, query.path.as_deref())
        .await
        .map(|fields| Json(Listing { fields }))
        .map_err(|error| Refusal::of(&error))
}

/// The branches of the repository a folder on a runner's machine is in.
#[utoipa::path(
    get,
    path = "/api/runner/branches",
    tag = "runners",
    security(("bearer" = [])),
    params(PickerQuery),
    responses(
        (status = 200, description = "The runner's listing: `{ repo, current, branches }`", body = Listing),
        (status = 400, description = "No such runner (`ok: false`)", body = ErrorBody),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
        (status = 502, description = "The runner refused or didn't answer (`ok: false`)", body = ErrorBody),
    )
)]
pub(super) async fn branches(
    State(app): State<AppState>,
    _viewer: Viewer,
    query: Result<Query<PickerQuery>, QueryRejection>,
) -> Result<Json<Listing>, Refusal> {
    let query = query.map_or_else(|_| PickerQuery::default(), |Query(query)| query);
    let machine = picked(&app, query.machine.as_deref())?;
    app.runners()
        .branches(&machine, query.cwd.as_deref().unwrap_or_default())
        .await
        .map(|fields| Json(Listing { fields }))
        .map_err(|error| Refusal::of(&error))
}

/// The runner a picker names (default `runner`), when it is one.
fn picked(app: &AppState, machine: Option<&str>) -> Result<String, Refusal> {
    let machine = machine.unwrap_or(DEFAULT_RUNNER);
    if app.runners().is_runner(machine) {
        Ok(machine.to_owned())
    } else {
        Err(Refusal::not_ok(StatusCode::BAD_REQUEST, "no such runner"))
    }
}

/// The session `r-<hex>` a path names.
fn session_id(path: Result<Path<String>, PathRejection>) -> Result<SessionId, Refusal> {
    let Ok(Path(hex)) = path else {
        return Err(Refusal::not_found());
    };
    SessionId::parse(&format!("r-{hex}")).map_err(|_| Refusal::not_found())
}

/// A runner session's transcript: the runner's steps after `?after=` (the newest `?limit=`,
/// 400 by default, without one), with what the server knows of the session.
#[utoipa::path(
    get,
    path = "/api/runner/sessions/r-{id}/transcript",
    tag = "runners",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The session's id after `r-`"), StepsQuery),
    responses(
        (status = 200, description = "The transcript", body = Transcript),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
        (status = 404, description = "Not a session's id (no body)"),
        (status = 502, description = "The runner refused or didn't answer (`ok: false`)", body = ErrorBody),
    )
)]
pub(super) async fn transcript(
    State(app): State<AppState>,
    _viewer: Viewer,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<StepsQuery>, QueryRejection>,
) -> Result<Json<Value>, Refusal> {
    let id = session_id(path)?;
    let query = query.map_or_else(|_| StepsQuery::default(), |Query(query)| query);
    app.runners()
        .transcript(
            &id,
            query.after.as_deref().unwrap_or_default(),
            query.limit.as_deref().unwrap_or_default(),
        )
        .await
        .map(|answer| Json(Value::Object(answer)))
        .map_err(|error| Refusal::of(&error))
}

/// A runner session's live stream, passed through from its runner as it comes (its steps, the
/// words as they are typed, its current step), from the step after `?after=`.
#[utoipa::path(
    get,
    path = "/api/runner/sessions/r-{id}/live",
    tag = "runners",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The session's id after `r-`"), StepsQuery),
    responses(
        (status = 200, description = "A text/event-stream of the runner's events (`head`, `step`, `delta`, `think`, `tokens`)", content_type = "text/event-stream", body = String),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
        (status = 404, description = "Not a session's id (no body)"),
        (status = 502, description = "The runner didn't answer (`ok: false`)", body = ErrorBody),
    )
)]
pub(super) async fn live(
    State(app): State<AppState>,
    _viewer: Viewer,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<StepsQuery>, QueryRejection>,
) -> Result<Response, Refusal> {
    let id = session_id(path)?;
    let query = query.map_or_else(|_| StepsQuery::default(), |Query(query)| query);
    let machine = app
        .runners()
        .machine_of(&id)
        .await
        .map_err(|error| Refusal::of(&error))?;
    let stream = app
        .runners()
        .live(&id, query.after.as_deref().unwrap_or_default())
        .await
        .map_err(|_| {
            Refusal::not_ok(
                StatusCode::BAD_GATEWAY,
                format!("the runner on {machine} didn't answer"),
            )
        })?;
    let stopping = app.stopping().clone();
    // The runner restarting or the client leaving ends the stream: the app reconnects.
    let body = stream
        .take_until(async move { stopping.cancelled().await })
        .map(|chunk| chunk.map_err(|error: RunnerError| std::io::Error::other(error.to_string())));
    let mut response = Response::new(Body::from_stream(body));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(NO_BUFFERING.0, NO_BUFFERING.1);
    Ok(response)
}

/// The person's reply in a runner session: its next turn (queued behind one that runs), with
/// their files put in its inbox first.
#[utoipa::path(
    post,
    path = "/api/runner/sessions/r-{id}/send",
    tag = "runners",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The session's id after `r-`")),
    request_body = SendToSession,
    responses(
        (status = 200, description = "Sent: the runner's `{ turn, queued }`", body = RunnerReply),
        (status = 400, description = "Nothing to send", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "Not a session's id (no body)"),
        (status = 502, description = "The runner refused or didn't answer (`ok: false`)", body = ErrorBody),
    )
)]
pub(super) async fn send(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    path: Result<Path<String>, PathRejection>,
    body: Result<JsonBody, crate::extract::Invalid>,
) -> Result<Json<RunnerReply>, Refusal> {
    SignedIn::acting(device)?;
    let id = session_id(path)?;
    let ask: SendToSession = body
        .ok()
        .and_then(|JsonBody(body)| serde_json::from_value(body).ok())
        .unwrap_or_default();
    app.runners()
        .reply(&id, ask.text.as_deref().unwrap_or_default(), &ask.files)
        .await
        .map(|rest| {
            Json(RunnerReply {
                ok: true,
                rest: without_ok(rest),
            })
        })
        .map_err(|error| Refusal::of(&error))
}

/// The answer without its own `ok` (the reply puts `ok: true` first).
fn without_ok(mut rest: Map<String, Value>) -> Map<String, Value> {
    rest.remove("ok");
    rest
}

/// Stops a runner session: it and the turns queued behind it.
#[utoipa::path(
    post,
    path = "/api/runner/sessions/r-{id}/stop",
    tag = "runners",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The session's id after `r-`")),
    responses(
        (status = 200, description = "Stopped: the runner's `{ running }`", body = RunnerReply),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "Not a session's id (no body)"),
        (status = 502, description = "The runner refused or didn't answer (`ok: false`)", body = ErrorBody),
    )
)]
pub(super) async fn stop(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<RunnerReply>, Refusal> {
    SignedIn::acting(device)?;
    halt(&app, path, true).await
}

/// Stops the turn a runner session is in; its queued turns carry on.
#[utoipa::path(
    post,
    path = "/api/runner/sessions/r-{id}/interrupt",
    tag = "runners",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The session's id after `r-`")),
    responses(
        (status = 200, description = "Interrupted: the runner's `{ running }`", body = RunnerReply),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "Not a session's id (no body)"),
        (status = 502, description = "The runner refused or didn't answer (`ok: false`)", body = ErrorBody),
    )
)]
pub(super) async fn interrupt(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<RunnerReply>, Refusal> {
    SignedIn::acting(device)?;
    halt(&app, path, false).await
}

async fn halt(
    app: &AppState,
    path: Result<Path<String>, PathRejection>,
    stop: bool,
) -> Result<Json<RunnerReply>, Refusal> {
    let id = session_id(path)?;
    app.runners()
        .halt(&id, stop)
        .await
        .map(|rest| {
            Json(RunnerReply {
                ok: true,
                rest: without_ok(rest),
            })
        })
        .map_err(|error| Refusal::of(&error))
}

/// Changes a runner session's model, effort or title, as the body asks.
#[utoipa::path(
    post,
    path = "/api/runner/sessions/r-{id}/settings",
    tag = "runners",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The session's id after `r-`")),
    request_body(content = Object, description = "`{ model?, effort?, title? }`, passed to the runner as is"),
    responses(
        (status = 200, description = "Changed: the session's new head", body = SessionSettingsReply),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "Not a session's id (no body)"),
        (status = 502, description = "The runner refused or didn't answer (`ok: false`)", body = ErrorBody),
    )
)]
pub(super) async fn settings(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    path: Result<Path<String>, PathRejection>,
    body: Result<JsonBody, crate::extract::Invalid>,
) -> Result<Json<SessionSettingsReply>, Refusal> {
    SignedIn::acting(device)?;
    let id = session_id(path)?;
    let body = body.map_or_else(|_| Value::Object(Map::new()), |JsonBody(body)| body);
    app.runners()
        .settings(&id, body)
        .await
        .map(|session| Json(SessionSettingsReply { ok: true, session }))
        .map_err(|error| Refusal::of(&error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_room_is_a_positive_whole_number() {
        assert_eq!(room_arg(Some("3")), Some(RoomId::from(3)));
        assert_eq!(room_arg(Some("0")), None);
        assert_eq!(room_arg(Some("2.5")), None);
        assert_eq!(room_arg(Some("x")), None);
        assert_eq!(room_arg(None), None);
    }
}

// The assistant's workers as the sessions `w-<task>`: these routes wait on the worker runtime
// (see the module's doc); with `NoWorkers` every one is 404 `no such task`.

/// The worker `w-<task>` a path names: its task's id.
fn worker_id(path: Result<Path<String>, PathRejection>) -> Result<i64, Refusal> {
    match path {
        Ok(Path(digits)) if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => {
            digits.parse().map_err(|_| Refusal::not_found())
        }
        _ => Err(Refusal::not_found()),
    }
}

/// The worker's head, or 404 `{ ok: false, error: "no such task" }`.
async fn worker_head(app: &AppState, task: i64) -> Result<Map<String, Value>, Refusal> {
    app.worker_sessions()
        .head(task)
        .await
        .ok_or_else(|| Refusal::not_ok(StatusCode::NOT_FOUND, "no such task"))
}

/// `?after=` as a step number (0 when it isn't one).
fn after_of(query: &StepsQuery) -> u64 {
    query
        .after
        .as_deref()
        .and_then(|after| after.trim().parse::<f64>().ok())
        .filter(|n| n.is_finite() && *n > 0.0)
        .map_or(0, |n| {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a positive step number"
            )]
            let n = n.trunc() as u64;
            n
        })
}

/// A worker's steps, as a session's transcript: after `?after=`, the first `?limit=` (400 by
/// default, 1000 at most) of what came since; else the newest; `?agent=` keeps one sub-agent
/// call's.
///
/// Without a worker runtime, every id is 404 `no such task`.
#[utoipa::path(
    get,
    path = "/api/runner/sessions/w-{id}/transcript",
    tag = "runners",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The worker's task id"), StepsQuery),
    responses(
        (status = 200, description = "The worker's head and steps", body = Transcript),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
        (status = 404, description = "No such task (`ok: false`; no body for an id that isn't a number)", body = ErrorBody),
    )
)]
pub(super) async fn worker_transcript(
    State(app): State<AppState>,
    _viewer: Viewer,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<StepsQuery>, QueryRejection>,
) -> Result<Json<Value>, Refusal> {
    let task = worker_id(path)?;
    let query = query.map_or_else(|_| StepsQuery::default(), |Query(query)| query);
    let head = worker_head(&app, task).await?;
    let after = after_of(&query);
    let limit = query
        .limit
        .as_deref()
        .and_then(|limit| limit.trim().parse::<usize>().ok())
        .filter(|limit| *limit != 0)
        .unwrap_or(400)
        .min(1000);
    let all = app
        .worker_sessions()
        .steps(
            task,
            after,
            query.agent.as_deref().filter(|agent| !agent.is_empty()),
        )
        .await;
    let (events, earlier) = crate::services::workers_view::page(all, after, limit);
    Ok(Json(
        serde_json::json!({ "session": head, "events": events, "earlier": earlier }),
    ))
}

/// A worker's steps as they come: each new step, and its head whenever it changes, looked at
/// every second; a `: ping` every 15 s.
///
/// Without a worker runtime, every id is 404 `no such task`.
#[utoipa::path(
    get,
    path = "/api/runner/sessions/w-{id}/live",
    tag = "runners",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The worker's task id"), StepsQuery),
    responses(
        (status = 200, description = "A text/event-stream of `{ type: step, step }` and `{ type: head, session }`", content_type = "text/event-stream", body = String),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
        (status = 404, description = "No such task (`ok: false`; no body for an id that isn't a number)", body = ErrorBody),
    )
)]
pub(super) async fn worker_live(
    State(app): State<AppState>,
    _viewer: Viewer,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<StepsQuery>, QueryRejection>,
) -> Result<Response, Refusal> {
    use axum::response::sse::{Event, KeepAlive, Sse};

    let task = worker_id(path)?;
    worker_head(&app, task).await?;
    let query = query.map_or_else(|_| StepsQuery::default(), |Query(query)| query);
    let agent = query.agent.clone().filter(|agent| !agent.is_empty());
    let after = after_of(&query);
    let stopping = app.stopping().clone();
    let looks = futures_util::stream::unfold(
        (app, after, String::new(), true),
        move |(app, mut after, mut was, first)| {
            let agent = agent.clone();
            async move {
                if !first {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
                let mut events = Vec::new();
                for step in app
                    .worker_sessions()
                    .steps(task, after, agent.as_deref())
                    .await
                {
                    if let Some(seq) = step.get("seq").and_then(Value::as_u64) {
                        after = after.max(seq);
                    }
                    events.push(serde_json::json!({ "type": "step", "step": step }));
                }
                if let Some(head) = app.worker_sessions().head(task).await {
                    let key = Value::Object(head.clone()).to_string();
                    if key != was {
                        was = key;
                        events.push(serde_json::json!({ "type": "head", "session": head }));
                    }
                }
                Some((events, (app, after, was, false)))
            }
        },
    )
    .flat_map(|events| {
        futures_util::stream::iter(events.into_iter().map(|event| {
            Ok::<_, std::convert::Infallible>(Event::default().data(event.to_string()))
        }))
    })
    .take_until(async move { stopping.cancelled().await });
    let keep_alive = KeepAlive::new()
        .interval(std::time::Duration::from_secs(15))
        .text("ping");
    Ok(([NO_BUFFERING], Sse::new(looks).keep_alive(keep_alive)).into_response())
}

/// The person's reply to a worker: it carries on with it.
///
/// Without a worker runtime, every id is 404 `no such task`.
#[utoipa::path(
    post,
    path = "/api/runner/sessions/w-{id}/send",
    tag = "runners",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The worker's task id")),
    request_body = SendToSession,
    responses(
        (status = 200, description = "Sent: the worker's answer", body = RunnerReply),
        (status = 400, description = "Nothing to send", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such task (`ok: false`; no body for an id that isn't a number)", body = ErrorBody),
        (status = 409, description = "The worker didn't take it (`ok: false`)", body = ErrorBody),
    )
)]
pub(super) async fn worker_send(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    path: Result<Path<String>, PathRejection>,
    body: Result<JsonBody, crate::extract::Invalid>,
) -> Result<Json<RunnerReply>, Refusal> {
    SignedIn::acting(device)?;
    let task = worker_id(path)?;
    worker_head(&app, task).await?;
    let ask: SendToSession = body
        .ok()
        .and_then(|JsonBody(body)| serde_json::from_value(body).ok())
        .unwrap_or_default();
    let text = ask.text.as_deref().unwrap_or_default().trim().to_owned();
    if text.is_empty() {
        return Err(Refusal::plain(StatusCode::BAD_REQUEST, "nothing to send"));
    }
    app.worker_sessions()
        .message(task, text)
        .await
        .map(|rest| {
            Json(RunnerReply {
                ok: true,
                rest: without_ok(rest),
            })
        })
        .map_err(|why| Refusal::not_ok(StatusCode::CONFLICT, why))
}

/// Stops a worker.
///
/// Without a worker runtime, every id is 404 `no such task`.
#[utoipa::path(
    post,
    path = "/api/runner/sessions/w-{id}/stop",
    tag = "runners",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The worker's task id")),
    responses(
        (status = 200, description = "Stopped", body = OkReply),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such task (`ok: false`; no body for an id that isn't a number)", body = ErrorBody),
    )
)]
pub(super) async fn worker_stop(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<OkReply>, Refusal> {
    SignedIn::acting(device)?;
    stop_worker(&app, path).await
}

/// Stops a worker (a worker has no turn of its own to interrupt: the same as stop).
///
/// Without a worker runtime, every id is 404 `no such task`.
#[utoipa::path(
    post,
    path = "/api/runner/sessions/w-{id}/interrupt",
    tag = "runners",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The worker's task id")),
    responses(
        (status = 200, description = "Stopped", body = OkReply),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such task (`ok: false`; no body for an id that isn't a number)", body = ErrorBody),
    )
)]
pub(super) async fn worker_interrupt(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<OkReply>, Refusal> {
    SignedIn::acting(device)?;
    stop_worker(&app, path).await
}

async fn stop_worker(
    app: &AppState,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<OkReply>, Refusal> {
    let task = worker_id(path)?;
    let head = worker_head(app, task).await?;
    app.worker_sessions().stop(task).await;
    let title = head
        .get("title")
        .and_then(Value::as_str)
        .map_or_else(|| format!("task {task}"), str::to_owned);
    if let Err(error) = app
        .activity()
        .note(format!("Stopped the worker of {title}"))
        .await
    {
        tracing::warn!(%error, "an activity line could not be written");
    }
    Ok(Json(OkReply::OK))
}
