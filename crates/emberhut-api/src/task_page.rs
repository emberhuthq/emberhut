//! `GET /api/tasks/{id}`: one task with everything its page shows. Any task, done or archived
//! ones too, so a task done weeks ago still opens; 404 with an [`ErrorBody`] when there is none.
//!
//! The answer is the task's own fields ([`Issue`], without the board's `deadline`, `hardDeadline`
//! and `score`, which take their defaults) with its history, subtasks, findings, links, runs,
//! sessions and ultra run spread in.
//!
//! [`ErrorBody`]: crate::error::ErrorBody

use serde::{Deserialize, Serialize};

use crate::ids::{MachineId, RunId};
use crate::open_enum::open_enum;
use crate::task_plan::Plan;
use crate::tasks::Issue;
use crate::time::Timestamp;
use crate::ultra::UltraRun;
use crate::work::RunnerSession;

/// A task's page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TaskPage {
    /// The task.
    #[serde(flatten)]
    pub task: Issue,
    /// Every step of it, oldest first.
    #[serde(default)]
    pub history: Vec<Moment>,
    /// Its subtasks, in order.
    #[serde(default)]
    pub subtasks: Vec<Issue>,
    /// The findings filed under it (a scan's or a review's), in order.
    #[serde(default)]
    pub findings: Vec<Issue>,
    /// The work that moves it: threads, tickets, chats, with the latest word on each.
    #[serde(default)]
    pub links: Vec<TaskLink>,
    /// Every run of work on it, newest first; none from an older server, whose finished turns
    /// are only in the history.
    #[serde(default)]
    pub runs: Option<Vec<Run>>,
    /// Its sessions on the runners.
    #[serde(default)]
    pub sessions: Vec<RunnerSession>,
    /// Its run's lead and sub-agents, when it is an ultra run, has sub-agents or a worker to
    /// watch.
    #[serde(default)]
    pub ultra_run: Option<UltraRun>,
    /// Its plan, step by step, on a server that keeps plans.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<Plan>,
}

/// One step of a task's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Moment {
    /// When.
    pub at: Timestamp,
    /// Who: the person's id, the assistant's, or `worker`; empty when the server has none
    /// (it may send `null`).
    #[serde(default, deserialize_with = "crate::serde_util::null_as_default")]
    pub who: String,
    /// What happened, or what was said; empty when the server has none (it may send `null`).
    #[serde(default, deserialize_with = "crate::serde_util::null_as_default")]
    pub text: String,
}

/// One run of work on a task: a turn of a thread, or a worker's session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Run {
    /// Its id.
    pub id: RunId,
    /// `thread` or `worker`.
    pub kind: String,
    /// The machine it ran on.
    pub machine: MachineId,
    /// Its thread or session.
    pub thread: String,
    /// Its thread's title (a worker's has none).
    pub title: Option<String>,
    /// What it was asked, cut short.
    pub asked: Option<String>,
    /// Its answer, cut short.
    pub answer: Option<String>,
    /// Its STATUS line (`STATUS: done`, `STATUS: needs …: …`).
    pub status: Option<String>,
    /// When it was sent.
    pub requested: Option<Timestamp>,
    /// When it started.
    pub started: Option<Timestamp>,
    /// When it ended; none while it runs.
    pub ended: Option<Timestamp>,
    /// How it ended: `completed`, `failed`, `stopped`, ...
    pub outcome: Option<String>,
}

open_enum! {
    /// What a task's link is.
    pub enum LinkKind {
        /// A thread on a machine.
        Thread = "thread",
        /// A ticket.
        Ticket = "ticket",
        /// A chat.
        Chat = "chat",
    }
}

/// A piece of work that moves a task, with the latest word on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TaskLink {
    /// What it is.
    pub kind: LinkKind,
    /// Its reference: a thread id, a ticket key, a chat.
    #[serde(rename = "ref")]
    pub reference: String,
    /// The machine a thread runs on.
    pub machine: Option<MachineId>,
    /// Its title.
    #[serde(default)]
    pub title: String,
    /// Where it stands, in its own words (`completed`, `failed`, `Waiting`, ...).
    pub status: Option<String>,
    /// When it last moved.
    pub at: Option<Timestamp>,
    /// Its latest line.
    pub line: Option<String>,
}
