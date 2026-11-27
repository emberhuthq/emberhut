//! Tasks, the decisions they wait on, what they produced, and the projects and goals they
//! belong to.

use serde::{Deserialize, Serialize};

use crate::choices::Choice;
use crate::ids::{AreaId, ArtifactId, GoalId, MessageId, ProjectId, RoomId, TaskId};
use crate::open_enum::open_enum;
use crate::plan::GoalWeek;
use crate::task_live::TaskLive;
use crate::time::{Day, Timestamp};
use crate::x_reply::XReply;

open_enum! {
    /// Where a task is.
    pub enum TaskStatus {
        /// Not started.
        Todo = "todo",
        /// Being worked on.
        Working = "working",
        /// Waiting on the person: it has an action or a decision.
        Waiting = "waiting",
        /// Done.
        Done = "done",
        /// Put away for good (shown as Archived).
        Canceled = "canceled",
    }
}

open_enum! {
    /// Where a project is; a room carries its project's.
    pub enum ProjectStatus {
        /// Worked on; its room is in the switcher.
        Active = "active",
        /// Off Today, its room folded, until it comes back.
        Paused = "paused",
        /// Its room out of the switcher, quiet; its open tasks parked.
        Archived = "archived",
        /// Finished.
        Done = "done",
    }
}

impl ProjectStatus {
    /// [`ProjectStatus::Active`], for serde defaults: older servers sent no status, and `active`
    /// said it.
    pub(crate) fn active() -> Self {
        Self::Active
    }
}

open_enum! {
    /// What an ask costs the person.
    pub enum AskKind {
        /// A yes or no.
        Approve = "approve",
        /// Read and pick.
        Choose = "choose",
        /// Something only the person can do, outside the assistant.
        Do = "do",
    }
}

open_enum! {
    /// How a project runs.
    pub enum ProjectKind {
        /// It ends with something delivered.
        Deliverable = "deliverable",
        /// It goes on.
        Ongoing = "ongoing",
    }
}

open_enum! {
    /// A project's health, from its hard deadlines.
    pub enum ProjectHealth {
        /// More than a day of slack before every hard deadline.
        OnTrack = "on_track",
        /// Less than a day of slack before one.
        AtRisk = "at_risk",
        /// A hard deadline has passed.
        Late = "late",
    }
}

/// What a waiting task asks of the person. Only the fields it has are sent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Decision {
    /// The question, in full.
    #[serde(default)]
    pub question: String,
    /// The answers to pick from, as their buttons say them. One is sent back as an
    /// `AnswerRequest`'s `choice`.
    #[serde(default)]
    pub options: Vec<String>,
    /// A message to send that comes with it, word for word.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<String>,
    /// What it costs the person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<AskKind>,
    /// A few bullets of what the person needs to know.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<Vec<String>>,
    /// What each answer does, one line each.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<Vec<String>>,
    /// The assistant's pick: an index into `options`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommend: Option<i32>,
    /// A line or two on why it is asked now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// When it was first asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked: Option<Timestamp>,
    /// When the assistant last made sure it still holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<Timestamp>,
    /// The options as cards to read and compare, one per option in the same order (newer
    /// servers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<Choice>>,
    /// Where the draft goes when the person taps its send answer (a chat's id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<String>,
}

impl Decision {
    /// How many of its options are cards to compare.
    #[must_use]
    pub fn card_count(&self) -> usize {
        self.choices
            .as_ref()
            .map_or(0, |choices| choices.iter().filter(|c| c.is_card()).count())
    }
}

/// Something a task produced: a picture or video (with a small thumbnail), a note, a link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Artifact {
    /// Its id.
    pub id: ArtifactId,
    /// `image`, `video`, `pdf`, `md`, `html`, `link`, `x_reply`, ... Text: the server takes
    /// these words from what the work produced, not from a list of its own.
    pub kind: String,
    /// A line about it.
    pub caption: Option<String>,
    /// When it was made.
    pub at: Timestamp,
    /// Its name.
    pub name: String,
    /// Where it opens (`/api/artifacts/<id>`).
    pub url: String,
    /// Its thumbnail, for pictures and videos.
    pub thumb: Option<String>,
    /// An `x_reply`: the post and the replies drafted for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<XReply>,
}

/// A task, as the state lists them: its own fields and, from the board, its effective deadline
/// and its score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is its own field on the wire (today, hard, ultra)"
)]
pub struct Issue {
    /// Its id.
    pub id: TaskId,
    /// Its key, as shown: the server's task-key prefix followed by the task's number.
    pub key: String,
    /// Its title.
    pub title: String,
    /// Who does it: the person's or the assistant's id.
    pub owner: String,
    /// Where it is.
    pub status: TaskStatus,
    /// Its goal, when it has one of its own.
    pub goal: Option<GoalId>,
    /// Its project's name.
    pub project: Option<String>,
    /// Its project.
    #[serde(default)]
    pub pid: Option<ProjectId>,
    /// 0 none, 1 urgent ... 4 low.
    pub priority: i32,
    /// When it is due.
    pub due: Option<Day>,
    /// One of today's.
    pub today: bool,
    /// An ask: the day to decide it by.
    #[serde(default)]
    pub decide_by: Option<Day>,
    /// What it asks, when it waits on a decision.
    pub decision: Option<Decision>,
    /// What the person should do, one line, when it waits on them.
    pub action: Option<String>,
    /// When it last changed.
    pub updated: Timestamp,
    /// When it was done.
    pub done_at: Option<Timestamp>,
    /// Its parent, for a subtask.
    #[serde(default)]
    pub parent: Option<TaskId>,
    /// What it produced, newest first.
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    /// Its deadline, or its project's.
    #[serde(default)]
    pub deadline: Option<Day>,
    /// Whether that deadline is someone else's (hard).
    #[serde(default)]
    pub hard_deadline: bool,
    /// How much it matters now: Today is ranked by it.
    #[serde(default)]
    pub score: f64,
    /// Out of sight until then (a day or an instant).
    #[serde(default)]
    pub parked_until: Option<String>,
    /// When it was made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<Timestamp>,
    /// The newest line of its history.
    #[serde(default)]
    pub last: Option<String>,
    /// Done elsewhere: the assistant only keeps its status, no worker takes it.
    #[serde(default)]
    pub tracked: bool,
    /// The person's notes on it, or the brief the assistant's worker works from.
    #[serde(default)]
    pub body: Option<String>,
    /// Its own deadline (`due`) is someone else's (hard).
    #[serde(default)]
    pub hard: bool,
    /// How long it takes.
    #[serde(default)]
    pub size: Option<TaskSize>,
    /// A counted task: how many are done so far.
    #[serde(default)]
    pub progress: Option<u32>,
    /// A counted task: how many there are in all.
    #[serde(default)]
    pub total: Option<u32>,
    /// A counted task: what is counted (`episodes`).
    #[serde(default)]
    pub unit: Option<String>,
    /// A parent: its subtasks at a glance.
    #[serde(default)]
    pub sub: Option<Rollup>,
    /// The assistant's estimate, in words (`about 2 hours`).
    #[serde(default)]
    pub eta: Option<String>,
    /// Its next run splits the work across many agents (ultra mode).
    #[serde(default)]
    pub ultra: bool,
    /// Its sub-agents at a glance, when it has any.
    #[serde(default)]
    pub agents: Option<AgentsLine>,
    /// What is actually happening on it, from its runs: what a client shows, rather than its
    /// status, which is the assistant's intent. Older servers don't send it.
    #[serde(default)]
    pub live: Option<TaskLive>,
    /// `finding` for a finding (a scan's or a review's result filed under it); none for a task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// A finding: whether its fix landed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landed: Option<Landed>,
    /// A finding: the task that fixes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_by: Option<TaskId>,
    /// Its project's area, when it is in one.
    #[serde(default)]
    pub area: Option<AreaId>,
    /// The person's own message that asked for it, its own or its parent's.
    #[serde(default)]
    pub requested: Option<Requested>,
    /// Someone other than the person asked for it, its own or its parent's.
    #[serde(default)]
    pub requested_by: Option<RequestedBy>,
    /// What the person's last tap on its send card came to.
    #[serde(default)]
    pub send: Option<TaskSend>,
}

/// The person's own request for a task: when, and the message and room it was made from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Requested {
    /// When.
    pub at: Timestamp,
    /// The message that asked; none for a task added in the app.
    #[serde(default)]
    pub msg: Option<MessageId>,
    /// The room it was asked in.
    #[serde(default)]
    pub room: Option<RoomId>,
}

/// A task asked for by someone other than the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RequestedBy {
    /// Who asked.
    pub who: String,
    /// When.
    pub at: Timestamp,
    /// How it was marked as theirs, in words.
    #[serde(default)]
    pub how: String,
}

/// What the person's tap on a task's send card (a message to send as them) came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TaskSend {
    /// `sending`, `sent`, `failed` or `canceled`.
    pub state: String,
    /// When.
    pub at: Timestamp,
    /// Why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// The channel (`Slack`, `WhatsApp`).
    #[serde(default)]
    pub via: String,
    /// Who it was for.
    #[serde(default)]
    pub to: String,
}

impl Issue {
    /// Whether it is still to be done: to do, being worked, or waiting on the person.
    #[must_use]
    pub fn is_open(&self) -> bool {
        matches!(
            self.status,
            TaskStatus::Todo | TaskStatus::Working | TaskStatus::Waiting
        )
    }

    /// Whether it is a finding rather than a task.
    #[must_use]
    pub fn is_finding(&self) -> bool {
        self.kind.as_deref() == Some(FINDING)
    }
}

/// [`Issue::kind`] of a finding.
pub const FINDING: &str = "finding";

open_enum! {
    /// How long a task takes.
    pub enum TaskSize {
        /// About an hour.
        Small = "S",
        /// About three hours.
        Medium = "M",
        /// About a day.
        Large = "L",
        /// Several days.
        ExtraLarge = "XL",
    }
}

open_enum! {
    /// Whether a finding's fix landed.
    pub enum Landed {
        /// It landed.
        Landed = "landed",
        /// Some of it landed.
        Partly = "partly",
        /// It didn't land.
        NotLanded = "not_landed",
        /// Something else replaced it.
        Superseded = "superseded",
        /// Not checked yet.
        Unchecked = "unchecked",
    }
}

/// A parent's subtasks at a glance: how many are done (archived ones don't count), which wait on
/// the person, and their counts added up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Rollup {
    /// Subtasks done.
    pub done: u32,
    /// Subtasks in all.
    pub total: u32,
    /// The subtasks waiting on the person.
    #[serde(default)]
    pub waiting: Vec<TaskId>,
    /// Their counts added up, when every subtask has one.
    pub progress: Option<u32>,
    /// Their totals added up, when every subtask has one.
    pub of: Option<u32>,
    /// What the open ones still need, when each of them has a count.
    #[serde(default)]
    pub left: Option<u32>,
    /// What is counted.
    pub unit: Option<String>,
}

/// A task's sub-agents at a glance: parts done of all, and how many run now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentsLine {
    /// Parts done.
    pub done: u32,
    /// Parts in all.
    pub total: u32,
    /// Sub-agents running now.
    pub running: u32,
}

/// A project: what tasks are grouped by. Its health comes from its hard deadlines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Project {
    /// Its id.
    pub id: ProjectId,
    /// Its name.
    pub name: String,
    /// The goal it serves.
    pub goal: Option<GoalId>,
    /// 0 none, 1 urgent ... 4 low.
    pub priority: i32,
    /// How it runs.
    pub kind: ProjectKind,
    /// Its own deadline.
    pub deadline: Option<Day>,
    /// Whether that deadline is someone else's.
    pub hard: bool,
    /// Where it is.
    pub status: ProjectStatus,
    /// A paused project's day to come back.
    #[serde(default)]
    pub paused_until: Option<Day>,
    /// Its open tasks.
    pub open: u32,
    /// Its tasks done this week.
    pub done_week: u32,
    /// Its health; none without a hard deadline.
    pub health: Option<ProjectHealth>,
    /// The nearest deadline of its open tasks.
    pub next: Option<Day>,
    /// The work left, in hours.
    pub hours_left: f64,
}

/// One of the person's goals, ranked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Goal {
    /// Its id.
    pub id: GoalId,
    /// Its title.
    pub title: String,
    /// Why it matters.
    pub why: String,
    /// 1 matters most.
    pub rank: i32,
    /// The assistant's line on how it goes this week.
    pub status: Option<String>,
    /// The label under its ring.
    pub short: Option<String>,
    /// When it last changed.
    #[serde(default)]
    pub at: Option<Timestamp>,
    /// Its steps done this week, by who did them.
    #[serde(default)]
    pub week: GoalWeek,
}

/// `POST /api/tasks`: a program on the server's machine hands the assistant a job the person
/// asked it for. Only with the server's program key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NewTaskRequest {
    /// Its title (the first 200 characters are kept).
    #[serde(default)]
    pub title: Option<String>,
    /// Its brief (the first 20,000 characters are kept).
    #[serde(default)]
    pub body: Option<String>,
    /// The program, in words (`a program` when left out).
    #[serde(default)]
    pub from: Option<String>,
}

/// `POST /api/tasks`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NewTaskReply {
    /// Always `true`.
    pub ok: bool,
    /// The new task.
    pub id: TaskId,
    /// Its key.
    pub key: String,
}
