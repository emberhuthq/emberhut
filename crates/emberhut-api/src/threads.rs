//! Coding threads: the state's `threads` (every coding agent's thread the server knows about),
//! `followed` (the threads the assistant handed work to and follows) and the running threads (the
//! coding-thread app's threads still running, shown read-only beside the runner's sessions),
//! and a thread's latest steps from the coding-thread app ([`ACTIVITY`]).
//!
//! The coding-thread app is another coding tool on the person's machines whose threads the
//! server follows over its MCP endpoint. The wire names that carry its name are listed in
//! [`crate::wire`]: the path [`ACTIVITY`], and the state's running-threads field ([`crate::state::State::t3_running`]), whose serde name
//! is written where the field is.
//!
//! A thread always carries its machine, id, title, project, status, `updatedAt` and `parent`
//! (`null` for a top-level thread); every other field is left out when the server knows
//! nothing for it, and reads as `None` when it is `null` or
//! missing.

use serde::{Deserialize, Serialize};

use crate::ids::{MachineId, RoomId, TaskId};
use crate::serde_util::is_false;
use crate::time::Timestamp;

/// A coding agent's thread on one machine, as last seen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CodingThread {
    /// The machine it runs on.
    pub machine: MachineId,
    /// Its id on that machine.
    pub id: String,
    /// Its title.
    pub title: String,
    /// Its project, by name (empty for none).
    pub project: String,
    /// `queued`, `running`, `waiting`, `idle`, `completed`, `failed`, `interrupted`,
    /// `cancelled`, ... (the runner's word).
    pub status: String,
    /// When it last changed.
    pub updated_at: Timestamp,
    /// The thread that started it, for a helper agent's thread.
    #[serde(default)]
    pub parent: Option<String>,
    /// What it was first asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<String>,
    /// What it was asked last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ask: Option<String>,
    /// Its latest answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
    /// The status line its latest answer ended with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_line: Option<String>,
    /// What it is doing now, while a turn runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doing: Option<String>,
    /// The account it runs on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The folder it works in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The branch it works on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Since when its current turn runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<Timestamp>,
    /// The person opened it and drives it themselves (sent only when true).
    #[serde(default, skip_serializing_if = "is_false")]
    pub direct: bool,
    /// The model it runs on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The reasoning effort it runs with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// A question it waits on the person to answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
}

/// A thread the assistant handed work to and follows: its end is news for the assistant at
/// once, and then it is no longer followed (a thread waiting for an answer stays).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FollowedThread {
    /// The thread's id.
    pub id: String,
    /// The machine it runs on.
    pub machine: MachineId,
    /// Why it is followed: the work it was given, in a few words.
    pub why: String,
    /// When it was followed.
    pub at: Timestamp,
    /// Its latest look; `null` until the thread has been seen.
    #[serde(default)]
    pub thread: Option<CodingThread>,
}

/// `GET ?machine=&thread=`: a coding-thread app thread's latest steps, read-only
/// ([`ThreadActivity`]). The path is in [`crate::wire`].
pub const ACTIVITY: &str = crate::wire::THREAD_ACTIVITY_PATH;

/// A thread of the coding-thread app still running, queued or waiting for an answer: the
/// state's `t3Running`, newest first, shown read-only beside the runner's sessions until it
/// ends there. Unlike [`CodingThread`], every field is always written (`null` when unknown).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExternalThread {
    /// Its id on its machine.
    pub id: String,
    /// The machine it runs on, as the server names the coding-thread app there.
    pub machine: MachineId,
    /// Its title.
    pub title: String,
    /// Its project in the coding-thread app, by name.
    pub project: String,
    /// `running`, `queued` or `waiting`.
    pub status: String,
    /// When it last changed.
    pub updated_at: Timestamp,
    /// What it was first asked.
    #[serde(default)]
    pub ask: Option<String>,
    /// Its latest answer.
    #[serde(default)]
    pub last: Option<String>,
    /// The status line its latest answer ended with.
    #[serde(default)]
    pub status_line: Option<String>,
    /// The tasks it works on.
    #[serde(default)]
    pub tasks: Vec<TaskId>,
    /// Its room (Main for one with no project).
    #[serde(default)]
    pub room: RoomId,
}

/// The query of [`ACTIVITY`]: which thread, on which machine. A missing one reads as empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams), into_params(parameter_in = Query))]
pub struct ActivityQuery {
    /// The machine, as the server names the coding-thread app there.
    #[serde(default)]
    pub machine: String,
    /// The thread's id there.
    #[serde(default)]
    pub thread: String,
}

/// [`ACTIVITY`]'s answer: a thread's title, status and latest steps, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadActivity {
    /// Its title (empty when the app gave none).
    pub title: String,
    /// Its status, in the app's word (empty when it gave none).
    pub status: String,
    /// When it last changed, as the app wrote it.
    #[serde(default)]
    pub updated_at: Option<String>,
    /// Its latest steps (at most 40), checkpoints and blank ones left out, each text cut to
    /// 1200 characters.
    pub items: Vec<ActivityStep>,
}

/// One step of a thread's activity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ActivityStep {
    /// Its kind, in the app's word: `user_message`, `assistant_message`, a tool's step...
    #[serde(rename = "type")]
    pub kind: String,
    /// Its title, when it has one.
    #[serde(default)]
    pub title: Option<String>,
    /// What it says.
    pub text: String,
    /// When it last changed, as the app wrote it.
    #[serde(default)]
    pub at: Option<String>,
}
