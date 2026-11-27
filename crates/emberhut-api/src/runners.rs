//! The runners on the person's machines: their sessions (`r-<hex>`), the person's own sessions
//! opened from the app, the folder and branch pickers, and what the runners report to the server.
//!
//! A runner is a program on one of the person's machines that runs coding-agent sessions. The
//! server reaches each runner's own HTTP API; the runner reports its sessions' events
//! (`POST /api/runner/events`) and its machine's sample (`POST /api/runner/machine`) back.
//! Much of what a runner answers (a transcript's steps, a folder listing) is passed through as
//! the runner wrote it: those parts are JSON objects here.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::chat::ChatFile;
use crate::work::RunnerSession;

/// `GET /api/runner/sessions`: the runner sessions, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SessionsPage {
    /// The sessions (50 at most).
    pub sessions: Vec<RunnerSession>,
}

/// `GET /api/runner/sessions`'s query.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams))]
#[cfg_attr(feature = "openapi", into_params(parameter_in = Query))]
pub struct SessionsQuery {
    /// Only the sessions linked to this task.
    #[serde(default)]
    pub task: Option<String>,
    /// Only this room's sessions (Main: every room's).
    #[serde(default)]
    pub room: Option<String>,
    /// Present: only the person's own sessions.
    #[serde(default)]
    pub direct: Option<String>,
}

/// `POST /api/runner/sessions`: one of the person's own sessions, on the machine and in the
/// folder they picked, with their first message.
///
/// The server relays it as the runner sends it; [`crate::runner::new_session::OpenSession`] describes the same wire.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NewSession {
    /// The runner machine (default `runner`).
    #[serde(default)]
    pub machine: Option<String>,
    /// The folder there.
    #[serde(default)]
    pub cwd: Option<String>,
    /// The first message.
    #[serde(default)]
    pub text: Option<String>,
    /// Files sent with it (uploads of the chat).
    #[serde(default)]
    pub files: Vec<ChatFile>,
    /// The account to run on (the runner picks when none is given).
    #[serde(default)]
    pub account: Option<String>,
    /// The model.
    #[serde(default)]
    pub model: Option<String>,
    /// The reasoning effort.
    #[serde(default)]
    pub effort: Option<String>,
    /// `true` or `{ branch?, base? }` for a worktree of its own, passed to the runner as is.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub worktree: Option<Value>,
    /// Its title (default: the runner's).
    #[serde(default)]
    pub title: Option<String>,
}

/// `POST /api/runner/sessions`'s answer: the runner's session, its id and the first turn's id.
///
/// The server relays it as the runner sends it; [`crate::runner::new_session::OpenedSession`] describes the same wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NewSessionReply {
    /// `true`.
    pub ok: bool,
    /// The session's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Its first turn's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<String>,
    /// The session, as the runner answered.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub session: Map<String, Value>,
}

/// `POST /api/runner/sessions/{id}/send`: the person's next message in a session.
///
/// The server relays it as the runner sends it; [`crate::runner::actions::SendToSession`] describes the same wire.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SendToSession {
    /// The message.
    #[serde(default)]
    pub text: Option<String>,
    /// Files sent with it.
    #[serde(default)]
    pub files: Vec<ChatFile>,
}

/// A runner's answer passed through with `ok: true` in front (a send's `{ turn, queued }`, a
/// stop's `{ running }`), or a worker's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RunnerReply {
    /// `true`.
    pub ok: bool,
    /// The rest of the answer, as the runner wrote it.
    #[serde(flatten)]
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub rest: Map<String, Value>,
}

/// `POST /api/runner/sessions/{id}/settings`'s answer: the session's new head.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SessionSettingsReply {
    /// `true`.
    pub ok: bool,
    /// The session, as the runner answered.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub session: Value,
}

/// A transcript (`GET /api/runner/sessions/{id}/transcript`): the session's head and its steps.
///
/// The server relays it as the runner sends it; [`crate::runner::transcript::Transcript`] describes the same wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Transcript {
    /// The session's head, with what the server knows of it (`place`, `accountName`, `tasks`).
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub session: Map<String, Value>,
    /// Its steps, oldest first.
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<Object>))]
    pub events: Vec<Value>,
    /// Older steps exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub earlier: Option<bool>,
    /// The rest of the runner's answer.
    #[serde(flatten)]
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub rest: Map<String, Value>,
}

/// A transcript's or a live stream's query.
///
/// The server relays it as the runner sends it; [`crate::runner::transcript::StepsQuery`] describes the same wire.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams))]
#[cfg_attr(feature = "openapi", into_params(parameter_in = Query))]
pub struct StepsQuery {
    /// Only the steps after this one.
    #[serde(default)]
    pub after: Option<String>,
    /// How many (400 by default, 1000 at most).
    #[serde(default)]
    pub limit: Option<String>,
    /// A worker's: only this sub-agent call's steps.
    #[serde(default)]
    pub agent: Option<String>,
}

/// `GET /api/runner/folders`'s and `GET /api/runner/branches`' query.
///
/// The server relays it as the runner sends it; [`crate::runner::new_session::FolderQuery`] describes the same wire.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams))]
#[cfg_attr(feature = "openapi", into_params(parameter_in = Query))]
pub struct PickerQuery {
    /// The runner machine (default `runner`).
    #[serde(default)]
    pub machine: Option<String>,
    /// Folders: the folder to list (none: where to start).
    #[serde(default)]
    pub path: Option<String>,
    /// Branches: the folder whose repository's branches to list.
    #[serde(default)]
    pub cwd: Option<String>,
}

/// A runner's listing passed through (folders: `{ machine, home, recent, documents, projects }`
/// or `{ machine, home, path, git, folders }`; branches: `{ repo, current, branches }`).
///
/// The server relays it as the runner sends it; [`crate::runner::new_session::FolderList`] describes the same wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Listing {
    /// The runner's answer.
    #[serde(flatten)]
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub fields: Map<String, Value>,
}

/// `GET /api/runner/mcp`: the MCP servers the runners' sessions get too, as the server's
/// operator listed them (`{ name: { type, url, headers, ... } }`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SharedMcp {
    /// The servers, by name.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub mcp_servers: Map<String, Value>,
}
