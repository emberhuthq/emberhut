//! The person's actions on tasks, each a body for `POST /api/settings`: mark done, put off a
//! week, archive and bring back, stop a worker, add a task, change one's dates, size, notes and
//! ultra mode.
//!
//! As with an answer ([`crate::answers`]), the 200 answer is `{ ok: true }` with the whole state
//! spread in ([`crate::error::OkReply`] reads the `ok`), and a refusal (a task that doesn't
//! exist) is 400 with an [`crate::error::ErrorBody`]. Every request is one [`TaskAction`].

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ids::{GoalId, ProjectId, TaskId};
use crate::tasks::TaskSize;
use crate::time::{Day, Timestamp};

/// One action on tasks, written as its own body (`{ "todoDone": 12, "done": true }`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum TaskAction {
    /// Mark the person's task done (or not done again).
    MarkDone(MarkDone),
    /// Put a task off until a time.
    Park(Park),
    /// Put a task away for good, its open subtasks with it.
    Archive(Archive),
    /// Bring an archived task back to to do.
    Unarchive(Unarchive),
    /// Stop the worker on a task: its current run ends where it is.
    StopWorker(StopWorker),
    /// Add a task of the person's own.
    Create(CreateTask),
    /// Change a task.
    Set(SetTask),
}

impl TaskAction {
    /// The person did `task`.
    #[must_use]
    pub fn mark_done(task: TaskId) -> Self {
        Self::MarkDone(MarkDone {
            todo_done: task,
            done: true,
        })
    }

    /// `task` is off the lists until `until`.
    #[must_use]
    pub fn park(task: TaskId, until: Timestamp) -> Self {
        Self::Park(Park {
            todo_park: task,
            until,
        })
    }

    /// `task` is put away for good.
    #[must_use]
    pub fn archive(task: TaskId) -> Self {
        Self::Archive(Archive { task_archive: task })
    }

    /// `task` is back in to do.
    #[must_use]
    pub fn unarchive(task: TaskId) -> Self {
        Self::Unarchive(Unarchive {
            task_unarchive: task,
        })
    }

    /// The worker on `task` stops.
    #[must_use]
    pub fn stop_worker(task: TaskId) -> Self {
        Self::StopWorker(StopWorker { task_stop: task })
    }

    /// A new task of the person's own, titled `title`.
    #[must_use]
    pub fn create(title: impl Into<String>) -> Self {
        Self::Create(CreateTask {
            task_create: title.into(),
            goal: None,
            project: None,
        })
    }

    /// The task the action is about, when it is about one.
    #[must_use]
    pub fn task(&self) -> Option<TaskId> {
        match self {
            Self::MarkDone(action) => Some(action.todo_done),
            Self::Park(action) => Some(action.todo_park),
            Self::Archive(action) => Some(action.task_archive),
            Self::Unarchive(action) => Some(action.task_unarchive),
            Self::StopWorker(action) => Some(action.task_stop),
            Self::Set(action) => Some(action.task_set.id),
            Self::Create(_) => None,
        }
    }
}

impl From<TaskChange> for TaskAction {
    fn from(change: TaskChange) -> Self {
        Self::Set(SetTask { task_set: change })
    }
}

/// `{ todoDone, done }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MarkDone {
    /// The task.
    pub todo_done: TaskId,
    /// Done, or back to to do.
    pub done: bool,
}

/// `{ todoPark, until }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Park {
    /// The task.
    pub todo_park: TaskId,
    /// Off the lists until then.
    pub until: Timestamp,
}

/// `{ taskArchive }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Archive {
    /// The task.
    pub task_archive: TaskId,
}

/// `{ taskUnarchive }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Unarchive {
    /// The task.
    pub task_unarchive: TaskId,
}

/// `{ taskStop }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct StopWorker {
    /// The task.
    pub task_stop: TaskId,
}

/// `{ taskCreate, goal?, project? }`: the server makes it the person's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateTask {
    /// Its title; a blank one adds nothing.
    pub task_create: String,
    /// Its goal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<GoalId>,
    /// Its project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectId>,
}

/// `{ taskSet: { id, ... } }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SetTask {
    /// The change.
    pub task_set: TaskChange,
}

/// One field of a [`TaskChange`]: left as it is (not written), cleared (written `null`), or set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Edit<T> {
    /// Left as it is.
    #[default]
    Keep,
    /// Cleared.
    Clear,
    /// Set to this.
    Set(T),
}

impl<T> Edit<T> {
    /// Whether it leaves the field as it is (and so isn't written).
    #[must_use]
    pub fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }

    /// Set to `value`, or cleared when there is none.
    #[must_use]
    pub fn to(value: Option<T>) -> Self {
        value.map_or(Self::Clear, Self::Set)
    }
}

impl<T: Serialize> Serialize for Edit<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Keep | Self::Clear => serializer.serialize_none(),
            Self::Set(value) => serializer.serialize_some(value),
        }
    }
}

/// A field that is there reads as [`Edit::Clear`] (`null`) or [`Edit::Set`]; one left out takes
/// the `default`, [`Edit::Keep`].
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Edit<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Option::<T>::deserialize(deserializer).map(Self::to)
    }
}

/// A change to a task: only the fields given change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TaskChange {
    /// The task.
    pub id: TaskId,
    /// Its own deadline.
    #[serde(default, skip_serializing_if = "Edit::is_keep")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Day>))]
    pub deadline: Edit<Day>,
    /// Whether that deadline is someone else's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hard: Option<bool>,
    /// How long it takes.
    #[serde(default, skip_serializing_if = "Edit::is_keep")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<TaskSize>))]
    pub size: Edit<TaskSize>,
    /// The person's notes, or the worker's brief; blank clears them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Whether its next run is an ultra run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ultra: Option<bool>,
    /// An ask's day to decide it by.
    #[serde(default, skip_serializing_if = "Edit::is_keep")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Day>))]
    pub decide_by: Edit<Day>,
}

impl TaskChange {
    /// A change to `task` that changes nothing yet.
    #[must_use]
    pub fn of(task: TaskId) -> Self {
        Self {
            id: task,
            deadline: Edit::Keep,
            hard: None,
            size: Edit::Keep,
            body: None,
            ultra: None,
            decide_by: Edit::Keep,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn written(action: &TaskAction) -> serde_json::Value {
        serde_json::to_value(action).unwrap()
    }

    #[test]
    fn each_action_is_its_own_body() {
        let task = TaskId::new(12);
        assert_eq!(
            written(&TaskAction::mark_done(task)),
            json!({"todoDone": 12, "done": true})
        );
        assert_eq!(
            written(&TaskAction::park(
                task,
                Timestamp::new("2026-12-02T00:00:00.000Z")
            )),
            json!({"todoPark": 12, "until": "2026-12-02T00:00:00.000Z"})
        );
        assert_eq!(
            written(&TaskAction::archive(task)),
            json!({"taskArchive": 12})
        );
        assert_eq!(
            written(&TaskAction::unarchive(task)),
            json!({"taskUnarchive": 12})
        );
        assert_eq!(
            written(&TaskAction::stop_worker(task)),
            json!({"taskStop": 12})
        );
        assert_eq!(
            written(&TaskAction::create("Call the bank")),
            json!({"taskCreate": "Call the bank"})
        );
    }

    #[test]
    fn a_change_writes_only_what_it_changes_and_null_clears() {
        let change = TaskChange {
            deadline: Edit::Clear,
            size: Edit::Set(TaskSize::Medium),
            decide_by: Edit::Set(Day::new("2026-12-01")),
            ..TaskChange::of(TaskId::new(4))
        };
        let body = written(&change.clone().into());
        assert_eq!(
            body,
            json!({"taskSet": {"id": 4, "deadline": null, "size": "M", "decideBy": "2026-12-01"}})
        );
        let read: TaskAction = serde_json::from_value(body).unwrap();
        assert_eq!(read, TaskAction::from(change));
        assert_eq!(read.task(), Some(TaskId::new(4)));
    }

    #[test]
    fn bodies_read_back_as_their_action() {
        for action in [
            TaskAction::mark_done(TaskId::new(1)),
            TaskAction::archive(TaskId::new(2)),
            TaskAction::unarchive(TaskId::new(3)),
            TaskAction::stop_worker(TaskId::new(4)),
            TaskAction::create("A task"),
        ] {
            let read: TaskAction = serde_json::from_value(written(&action)).unwrap();
            assert_eq!(read, action);
        }
    }
}
