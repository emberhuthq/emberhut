//! What is actually happening on a task, from its runs (a task's `live`).
//!
//! A task's status is the assistant's intent; its live state is the fact: a run open and
//! started, one queued behind another turn, a watch that looks now and then, nothing running at
//! all. A client words a task from this, not from its status.

use serde::{Deserialize, Serialize};

use crate::ids::{MachineId, TaskId, WatchId};
use crate::open_enum::open_enum;
use crate::time::Timestamp;

open_enum! {
    /// What is happening on a task now.
    pub enum LiveState {
        /// A run of its own is open and started.
        Running = "running",
        /// Sent, waiting behind another turn.
        Queued = "queued",
        /// Nothing runs until its next look (a watch, or a check-in that names it).
        Scheduled = "scheduled",
        /// It waits on the person.
        Waiting = "waiting",
        /// Open, nothing running and nothing asked.
        Stalled = "stalled",
        /// The person's own, not started.
        Todo = "todo",
        /// Put off until a day.
        Parked = "parked",
        /// Held by its parent, which waits on the person.
        Held = "held",
        /// Done.
        Done = "done",
        /// Put away for good (shown as Archived).
        Canceled = "canceled",
    }
}

/// What is happening on a task. The fields after `alarm` are sent only when they say something.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TaskLive {
    /// What is happening.
    pub state: LiveState,
    /// Since when.
    #[serde(default)]
    pub since: Option<Timestamp>,
    /// Stalled and woken twice with nothing changed: it needs a look.
    #[serde(default)]
    pub alarm: bool,
    /// The run's task was guessed from its words rather than named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inferred: Option<bool>,
    /// The run open on it, running or queued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<LiveRun>,
    /// What does the work when it isn't a turn: `browser`, `watch:<id>`, `unit:<name>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<String>,
    /// Parked until then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<Timestamp>,
    /// Held by this parent, which waits on the person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<TaskId>,
    /// Scheduled: the watch that looks at it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch: Option<LiveWatch>,
    /// Scheduled: the check-in that names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkin: Option<LiveCheckin>,
    /// Scheduled: a look is on now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checking: Option<bool>,
    /// The last run that ended, and its first line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<LiveLast>,
}

/// The run open on a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LiveRun {
    /// The machine it runs on.
    pub machine: MachineId,
    /// That machine in words (`the Mac`); older servers don't send it.
    #[serde(default)]
    pub place: Option<String>,
    /// Its thread or session.
    pub thread: String,
    /// The thread's title.
    pub title: Option<String>,
    /// It is the task's own worker.
    pub worker: bool,
    /// When it was sent.
    pub requested: Option<Timestamp>,
    /// When it started; none while queued.
    pub started: Option<Timestamp>,
}

/// The last run that ended on a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LiveLast {
    /// When it ended.
    pub ended: Option<Timestamp>,
    /// Its first line.
    pub line: Option<String>,
}

/// The watch that looks at a scheduled task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LiveWatch {
    /// Its id.
    pub id: WatchId,
    /// What it watches.
    pub title: Option<String>,
    /// Its next look.
    pub next_at: Timestamp,
    /// How often it looks, in minutes.
    pub every: f64,
    /// What it saw last.
    pub note: Option<String>,
    /// When it saw that.
    pub note_at: Option<Timestamp>,
}

/// The check-in that names a scheduled task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LiveCheckin {
    /// When it is.
    pub at: Timestamp,
    /// Why.
    pub why: String,
}
