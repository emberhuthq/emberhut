//! Work in flight: the state's `working` rows and the sessions on the runners.

use serde::{Deserialize, Deserializer, Serialize};

use crate::ids::{AreaId, MachineId, ProjectId, RoomId, SessionId, TaskId, WorkId};
use crate::open_enum::open_enum;
use crate::serde_util::is_false;
use crate::tasks::{Artifact, AskKind};
use crate::time::{Day, Timestamp};

open_enum! {
    /// What a row of work in flight is.
    pub enum WorkKind {
        /// A task.
        Task = "task",
        /// A thread with no task.
        Thread = "thread",
    }
}

open_enum! {
    /// What a piece of work in flight is doing.
    pub enum WorkState {
        /// Waiting on the person: "Needs you".
        Waiting = "waiting",
        /// Running now.
        Running = "running",
        /// Stuck: it needs a look.
        Stuck = "stuck",
        /// Its account hit a usage limit.
        Limit = "limit",
        /// Stopped.
        Stopped = "stopped",
        /// Sent, waiting behind another turn.
        Queued = "queued",
        /// Open, nothing running and nothing asked.
        Stalled = "stalled",
        /// Nothing runs until its next look.
        Scheduled = "scheduled",
    }
}

/// One piece of work in flight: a task (worked, or waiting on the person) or a thread with no
/// task. A row that waits on the person adds `Waiting`'s fields; `paused` is sent only when
/// true.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkRow {
    /// Its id.
    pub id: WorkId,
    /// A task's or a thread's.
    pub kind: WorkKind,
    /// Its task.
    pub task: Option<TaskId>,
    /// The machine it runs on.
    pub machine: Option<MachineId>,
    /// Its thread.
    pub thread: Option<String>,
    /// Its title.
    pub title: String,
    /// Its project.
    pub pid: Option<ProjectId>,
    /// Who works it.
    pub who: String,
    /// What it is doing.
    pub state: WorkState,
    /// How far along, in words.
    pub progress: Option<String>,
    /// Its latest line.
    pub line: Option<String>,
    /// When that was.
    pub at: Option<Timestamp>,
    /// The estimate, in words.
    pub eta: Option<String>,
    /// What it asks the person.
    pub ask: Option<String>,
    /// The last thing it said.
    pub last: Option<String>,
    /// What the person should do, one line.
    #[serde(default)]
    pub action: Option<String>,
    /// The answers to that.
    #[serde(default)]
    pub options: Option<Vec<String>>,
    /// A message to send comes with it.
    #[serde(default)]
    pub draft: bool,
    /// What waiting on the person costs, on the rows that wait on the person.
    ///
    /// Read as present when the row has a `need`. A row with a `need` whose group is malformed
    /// (`asked` not a time, `hardBy` `null` or not a bool) is an error, as a bad field is
    /// anywhere else, not a row that doesn't wait. Fields of the group without a `need` are
    /// ignored: the server never writes them alone.
    #[serde(flatten, deserialize_with = "waiting_group")]
    #[cfg_attr(feature = "openapi", schema(schema_with = waiting_fields))]
    pub waiting: Option<Waiting>,
    /// Its project is paused: it waits quietly until the project is back.
    #[serde(default, skip_serializing_if = "is_false")]
    pub paused: bool,
    /// Its project's area, when it is in one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<AreaId>,
    /// A task's row: its newest picture's thumbnail, at a glance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    /// A task's row: an X post to reply to (an `x_reply` artifact), whose replies the card offers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<Artifact>,
}

/// The waiting group as the document describes it on a row: its fields, none required (a row
/// that doesn't wait has none of them; one that waits has them all, as [`Waiting`] says).
#[cfg(feature = "openapi")]
fn waiting_fields() -> utoipa::openapi::schema::Object {
    use utoipa::openapi::schema::{ObjectBuilder, OneOfBuilder, Schema, Type};
    use utoipa::openapi::{Ref, RefOr};
    let or_null = |name: &str| -> RefOr<Schema> {
        RefOr::T(Schema::OneOf(
            OneOfBuilder::new()
                .item(RefOr::Ref(Ref::from_schema_name(name)))
                .item(ObjectBuilder::new().schema_type(Type::Null).build())
                .build(),
        ))
    };
    ObjectBuilder::new()
        .schema_type(Type::Object)
        .property("need", RefOr::Ref(Ref::from_schema_name("AskKind")))
        .property("asked", or_null("Timestamp"))
        .property("checked", or_null("Timestamp"))
        .property("decideBy", or_null("Day"))
        .property(
            "hardBy",
            ObjectBuilder::new().schema_type(Type::Boolean).build(),
        )
        .description(Some(
            "What waiting on the person costs: on a row that waits, `need`, `asked` and \
             `checked` (a task's row adds `decideBy` and `hardBy`); none on the others.",
        ))
        .build()
}

/// What a row that waits on the person adds: the server writes `need`, `asked` and `checked`
/// together (`null` included), and on a task's row `decideBy` and `hardBy` too. `deadline` is
/// read as present when `hardBy` is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Waiting {
    /// What it costs the person.
    pub need: AskKind,
    /// When it was asked.
    pub asked: Option<Timestamp>,
    /// When the assistant last checked it still holds.
    pub checked: Option<Timestamp>,
    /// When it has to be decided by: a task's row only.
    #[serde(flatten)]
    pub deadline: Option<WaitingDeadline>,
}

/// The waiting group's fields as they come, each optional, so that a missing one is told apart
/// from a malformed one (which is an error). Serde's own reading of a flattened `Option` would
/// turn any error inside the group into `None`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WaitingFields {
    #[serde(default)]
    need: Option<AskKind>,
    #[serde(default)]
    asked: Option<Timestamp>,
    #[serde(default)]
    checked: Option<Timestamp>,
    #[serde(default)]
    decide_by: Option<Day>,
    #[serde(default, deserialize_with = "not_null")]
    hard_by: Option<bool>,
}

impl WaitingFields {
    fn into_waiting(self) -> Option<Waiting> {
        let deadline = self.hard_by.map(|hard_by| WaitingDeadline {
            decide_by: self.decide_by,
            hard_by,
        });
        self.need.map(|need| Waiting {
            need,
            asked: self.asked,
            checked: self.checked,
            deadline,
        })
    }
}

/// A field that may be left out but, when given, isn't `null`.
fn not_null<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

/// [`WorkRow::waiting`]: present with a `need`, an error when malformed.
fn waiting_group<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Waiting>, D::Error> {
    WaitingFields::deserialize(deserializer).map(WaitingFields::into_waiting)
}

impl<'de> Deserialize<'de> for Waiting {
    /// The group's fields, `need` required; `deadline` present when `hardBy` is.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        WaitingFields::deserialize(deserializer)?
            .into_waiting()
            .ok_or_else(|| serde::de::Error::missing_field("need"))
    }
}

/// When a waiting task has to be decided by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WaitingDeadline {
    /// The day to decide it by: its decide-by or its deadline, the earlier; none when it has
    /// neither.
    pub decide_by: Option<Day>,
    /// That day is someone else's deadline.
    pub hard_by: bool,
}

impl WorkRow {
    /// Whether it waits on the person.
    #[must_use]
    pub fn needs_you(&self) -> bool {
        self.state == WorkState::Waiting
    }
}

/// A session on one of the runners, for the sessions list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RunnerSession {
    /// Its id.
    pub id: SessionId,
    /// The machine it runs on.
    pub machine: MachineId,
    /// Its title.
    pub title: String,
    /// Its project, by name.
    pub project: String,
    /// `running`, `queued`, `idle`, `done`, ...
    pub status: String,
    /// When it last changed.
    pub updated_at: Timestamp,
    /// The machine, in words (`Mac`, `Linux PC`).
    pub place: String,
    /// What it is doing now.
    pub doing: Option<String>,
    /// The account it runs on.
    pub account: Option<String>,
    /// That account's name.
    pub account_name: Option<String>,
    /// What it asks.
    pub ask: Option<String>,
    /// The last thing it said.
    pub last: Option<String>,
    /// The person started it (not a task's worker).
    pub direct: bool,
    /// The model it runs on.
    pub model: Option<String>,
    /// The folder it works in.
    #[serde(default)]
    pub cwd: Option<String>,
    /// The branch it works on.
    #[serde(default)]
    pub branch: Option<String>,
    /// The status line its latest answer ended with.
    #[serde(default)]
    pub status_line: Option<String>,
    /// Since when its current turn runs (only while it runs).
    #[serde(default)]
    pub since: Option<Timestamp>,
    /// The reasoning effort it runs with.
    #[serde(default)]
    pub effort: Option<String>,
    /// A question it waits on the person to answer.
    #[serde(default)]
    pub question: Option<String>,
    /// The tasks it works on.
    #[serde(default)]
    pub tasks: Vec<TaskId>,
    /// Its room (Main for one with no project).
    #[serde(default)]
    pub room: RoomId,
}

impl RunnerSession {
    /// Whether a turn runs or is about to.
    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(self.status.as_str(), "running" | "queued")
    }

    /// Whether it waits on the person.
    #[must_use]
    pub fn is_waiting(&self) -> bool {
        self.status == "waiting"
    }

    /// Its account, by name when it has one (the same account on two machines is one).
    #[must_use]
    pub fn account_label(&self) -> Option<&str> {
        self.account_name.as_deref().or(self.account.as_deref())
    }
}
