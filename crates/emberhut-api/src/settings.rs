//! `POST /api/settings`: every small change the person makes from an app, one route for all of
//! them. Pausing and resuming the assistant, stopping it, a check-in or a watch, ticking or
//! parking a step, archiving, answering, adding and editing tasks and projects, correcting
//! what the assistant knows, marking a post, signing a device out.
//!
//! The body is one object of optional keys, and the server does every action whose key it
//! finds. Two types model it:
//!
//! - [`SettingsRequest`] is the body as the wire has it: every key the server reads, each
//!   optional. It is what a server reads and what goes on the wire.
//! - [`SettingsAction`] is one action, typed: a client builds one of these and sends it as a
//!   request (`SettingsRequest::from(action)`), so a body never mixes keys that don't belong
//!   together. A new action is one more variant and its arm in the conversion.
//!
//! The 200 answer is `{ "ok": true }` with the whole state of `room` (Main by default) spread
//! in ([`SettingsReply`]); [`crate::error::OkReply`] reads the `ok`, and a client polls the
//! state as usual.
//!
//! The server refuses a change it can't make (no such task, project or fact; a
//! category it doesn't know; a text too long) with 400 and an [`crate::error::ErrorBody`] with
//! `ok: false`. Not every action can be refused that way: pausing or resuming, stopping a
//! room's answer, cancelling a check-in, stopping a watch or a job are done before the checked
//! part and answer 200 whatever came of them, and keeping or dropping a rule that isn't on
//! trial is a 200 too. A client that must know reads the state after: a server older than
//! `trialKeep` also answers 200 to it and leaves the rule on trial.

use serde::{Deserialize, Serialize};

use crate::answers::AnswerRequest;
use crate::builds::BuildReport;
use crate::facts::FactCategory;
use crate::ids::{
    ArtifactId, DeviceId, FactId, GoalId, JobId, ProjectId, RoomId, TaskId, TrialId, WatchId,
};
use crate::patch::Patch;
use crate::serde_util::is_false;
use crate::state::State;
use crate::tasks::ProjectStatus;
use crate::time::{Day, Timestamp};

/// One change, typed. See the module's docs for the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SettingsAction {
    /// Resume (`true`) or pause (`false`) the assistant: `{ on }`. Paused, it wakes for nothing
    /// and stops the turn it is in; it still answers the person in the chat.
    SetOn(bool),
    /// Stop what the assistant is answering in a room right now: `{ interrupt: true, room }`.
    /// Its background jobs keep running.
    Interrupt(RoomId),
    /// Cancel a check-in, by the time it was set for: `{ cancelCheckin }`.
    CancelCheckin(Timestamp),
    /// Stop a watch: `{ stopWatch }`.
    StopWatch(WatchId),
    /// Stop a background job: `{ stopTask }`.
    StopJob(JobId),
    /// Tick a step done, or untick it: `{ todoDone, done }`.
    TodoDone {
        /// The step.
        task: TaskId,
        /// Done, or back to to do.
        done: bool,
    },
    /// Park a step until a time (none: bring it back now): `{ todoPark, until }`.
    TodoPark {
        /// The step.
        task: TaskId,
        /// Until when.
        until: Option<Timestamp>,
    },
    /// Archive a task (its open subtasks with it): `{ taskArchive }`.
    TaskArchive(TaskId),
    /// Bring an archived task back to to do: `{ taskUnarchive }`.
    TaskUnarchive(TaskId),
    /// Answer a task's ask: `{ taskDecide, choice }`. It can't be taken back.
    TaskDecide(AnswerRequest),
    /// Add a task of the person's own: `{ taskCreate, goal?, project? }`.
    TaskCreate {
        /// Its title (the server trims it; a blank one is ignored).
        title: String,
        /// The goal it serves.
        goal: Option<GoalId>,
        /// The project it belongs to.
        project: Option<ProjectId>,
    },
    /// Change a task: `{ taskSet: { id, ... } }`.
    TaskSet(TaskChange),
    /// Stop a task's worker: `{ taskStop }`.
    TaskStop(TaskId),
    /// Change a project (or pause its room): `{ projectSet: { id, ... } }`.
    ProjectSet(ProjectChange),
    /// Correct a fact the assistant keeps: `{ factSet: { id, category, title, text } }`. The
    /// server notes the source as the person's correction and tells the assistant.
    FactSet(FactChange),
    /// Mark whether a person's messages wake the assistant at once:
    /// `{ personSet: { id, important } }`.
    PersonSet(PersonChange),
    /// Forget a fact: `{ factDrop }`.
    FactDrop(FactId),
    /// Drop a rule on trial: `{ trialDrop }`.
    TrialDrop(TrialId),
    /// Keep a rule on trial now, as a standing rule, without waiting for the weekly review:
    /// `{ trialKeep }`.
    TrialKeep(TrialId),
    /// Mark one of a post's drafted replies as posted: `{ xPosted, option }` (the option's
    /// index, from 0). Its task is done with it.
    XPosted {
        /// The drafted replies.
        artifact: ArtifactId,
        /// Which one was posted.
        option: u32,
    },
    /// Sign a device out: `{ authRevoke }`.
    AuthRevoke(DeviceId),
}

/// `taskSet`: the fields given change, the rest stay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct TaskChange {
    /// The task.
    #[cfg_attr(feature = "validate", garde(dive))]
    pub id: TaskId,
    /// Its deadline: a day, or `null` for none.
    #[serde(default, skip_serializing_if = "Patch::is_keep")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub deadline: Patch<Day>,
    /// Whether the deadline is hard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub hard: Option<bool>,
    /// Its size (as the server words sizes), or `null` for none.
    #[serde(default, skip_serializing_if = "Patch::is_keep")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub size: Patch<String>,
    /// Its project, or `null` for none.
    #[serde(default, skip_serializing_if = "Patch::is_keep")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub project: Patch<ProjectId>,
    /// Its description (the server trims it; empty clears it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(length(utf16, max = 100_000)))]
    pub body: Option<String>,
    /// Whether its next run splits the work across sub-agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub ultra: Option<bool>,
    /// The day it should be decided by, or `null` for none.
    #[serde(default, skip_serializing_if = "Patch::is_keep")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub decide_by: Patch<Day>,
}

impl TaskChange {
    /// A change of `task` that changes nothing yet.
    #[must_use]
    pub fn new(task: TaskId) -> Self {
        Self {
            id: task,
            deadline: Patch::Keep,
            hard: None,
            size: Patch::Keep,
            project: Patch::Keep,
            body: None,
            ultra: None,
            decide_by: Patch::Keep,
        }
    }
}

/// `projectSet`: the fields given change, the rest stay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct ProjectChange {
    /// The project.
    #[cfg_attr(feature = "validate", garde(dive))]
    pub id: ProjectId,
    /// Its priority: 0 none, 1 urgent, 2 high, 3 medium, 4 low.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(range(max = 4)))]
    pub priority: Option<u8>,
    /// Its deadline: a day, or `null` for none.
    #[serde(default, skip_serializing_if = "Patch::is_keep")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub deadline: Patch<Day>,
    /// Whether the deadline is hard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub hard: Option<bool>,
    /// Active, paused, archived or done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub status: Option<ProjectStatus>,
    /// With `status` paused: the day it comes back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub paused_until: Option<Day>,
}

impl ProjectChange {
    /// A change of `project` that changes nothing yet.
    #[must_use]
    pub fn new(project: ProjectId) -> Self {
        Self {
            id: project,
            priority: None,
            deadline: Patch::Keep,
            hard: None,
            status: None,
            paused_until: None,
        }
    }
}

/// `factSet`: a fact as the person corrected it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct FactChange {
    /// The fact.
    #[cfg_attr(feature = "validate", garde(skip))]
    pub id: FactId,
    /// Its category (moving it to another moves the fact).
    #[cfg_attr(feature = "validate", garde(skip))]
    pub category: FactCategory,
    /// Its title.
    #[cfg_attr(feature = "validate", garde(custom(crate::rules::not_blank)))]
    pub title: String,
    /// What is true, in the person's words.
    #[cfg_attr(feature = "validate", garde(custom(crate::rules::not_blank)))]
    pub text: String,
}

/// `personSet`: whether a person's messages wake the assistant at once (instead of with its
/// next look, every 15 minutes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct PersonChange {
    /// The person's fact (in `people`).
    #[cfg_attr(feature = "validate", garde(skip))]
    pub id: FactId,
    /// Wake the assistant when they write.
    #[cfg_attr(feature = "validate", garde(skip))]
    pub important: bool,
}

/// The body of `POST /api/settings` as the wire has it: every key the server reads, each left
/// out unless given. Build one from a [`SettingsAction`]; a server reads this and does every
/// action whose key it finds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct SettingsRequest {
    /// Resume or pause the assistant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub on: Option<bool>,
    /// Stop the turn in `room`.
    #[serde(default, skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub interrupt: bool,
    /// The room an interrupt is about, and whose state the answer carries (Main when left out).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub room: Option<RoomId>,
    /// What the app hides: the switches given change, the others stay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub ui: Option<DisplayUiChange>,
    /// The check-in to cancel, by its time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub cancel_checkin: Option<Timestamp>,
    /// The watch to stop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub stop_watch: Option<WatchId>,
    /// The background job to stop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub stop_task: Option<JobId>,
    /// The step to tick (or untick, with `done: false`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub todo_done: Option<TaskId>,
    /// With `todoDone`: done (the default) or back to to do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub done: Option<bool>,
    /// The step to park.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub todo_park: Option<TaskId>,
    /// With `todoPark`: until when (left out: brought back now).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub until: Option<Timestamp>,
    /// The task to archive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub task_archive: Option<TaskId>,
    /// The task to bring back from the archive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub task_unarchive: Option<TaskId>,
    /// The task whose ask is answered (with `choice`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub task_decide: Option<TaskId>,
    /// With `taskDecide`: the answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(length(min = 1)))]
    pub choice: Option<String>,
    /// A new task's title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(length(utf16, min = 1, max = 1000)))]
    pub task_create: Option<String>,
    /// With `taskCreate`: its goal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub goal: Option<GoalId>,
    /// With `taskCreate`: its project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub project: Option<ProjectId>,
    /// A change of a task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub task_set: Option<TaskChange>,
    /// The task whose worker to stop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub task_stop: Option<TaskId>,
    /// A change of a project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub project_set: Option<ProjectChange>,
    /// A corrected fact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub fact_set: Option<FactChange>,
    /// A person marked as waking the assistant, or not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub person_set: Option<PersonChange>,
    /// The fact to forget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub fact_drop: Option<FactId>,
    /// The rule on trial to drop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub trial_drop: Option<TrialId>,
    /// The rule on trial to keep now, as a standing rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub trial_keep: Option<TrialId>,
    /// The drafted replies one of which was posted (with `option`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub x_posted: Option<ArtifactId>,
    /// With `xPosted`: which reply, from 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub option: Option<u32>,
    /// The release step's report that a store has a build of the app ready.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub build: Option<BuildReport>,
    /// The device to sign out: its tokens stop working at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub auth_revoke: Option<DeviceId>,
    /// Pin a Focus item (or unpin it, with `pinned: false`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub pin_focus: Option<i64>,
    /// With `pinFocus`: `false` unpins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub pinned: Option<bool>,
    /// Drop a Focus item, pinned or not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub drop_focus: Option<i64>,
    /// Park a Focus item until `until` (`null`: bring it back).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub park_focus: Option<i64>,
    /// `true`: the body's room's next answer counts as its account running out (a test of
    /// the failover).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub test_limit: Option<bool>,
}

/// `ui`: what the app hides from now on. A switch left out (or not `true`/`false`) stays.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DisplayUiChange {
    /// Hide the goals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hide_goals: Option<bool>,
    /// Hide the projects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hide_projects: Option<bool>,
}

/// `POST /api/settings`'s answer: `{ ok: true }` and the state of the body's room.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SettingsReply {
    /// Always `true`.
    pub ok: bool,
    /// The state, as `GET /api/state` gives it.
    #[serde(flatten)]
    pub state: State,
}

impl From<SettingsAction> for SettingsRequest {
    fn from(action: SettingsAction) -> Self {
        let none = Self::default();
        match action {
            SettingsAction::SetOn(on) => Self {
                on: Some(on),
                ..none
            },
            SettingsAction::Interrupt(room) => Self {
                interrupt: true,
                room: Some(room),
                ..none
            },
            SettingsAction::CancelCheckin(at) => Self {
                cancel_checkin: Some(at),
                ..none
            },
            SettingsAction::StopWatch(watch) => Self {
                stop_watch: Some(watch),
                ..none
            },
            SettingsAction::StopJob(job) => Self {
                stop_task: Some(job),
                ..none
            },
            SettingsAction::TodoDone { task, done } => Self {
                todo_done: Some(task),
                done: Some(done),
                ..none
            },
            SettingsAction::TodoPark { task, until } => Self {
                todo_park: Some(task),
                until,
                ..none
            },
            SettingsAction::TaskArchive(task) => Self {
                task_archive: Some(task),
                ..none
            },
            SettingsAction::TaskUnarchive(task) => Self {
                task_unarchive: Some(task),
                ..none
            },
            SettingsAction::TaskDecide(answer) => Self {
                task_decide: Some(answer.task_decide),
                choice: Some(answer.choice),
                ..none
            },
            SettingsAction::TaskCreate {
                title,
                goal,
                project,
            } => Self {
                task_create: Some(title),
                goal,
                project,
                ..none
            },
            SettingsAction::TaskSet(change) => Self {
                task_set: Some(change),
                ..none
            },
            SettingsAction::TaskStop(task) => Self {
                task_stop: Some(task),
                ..none
            },
            SettingsAction::ProjectSet(change) => Self {
                project_set: Some(change),
                ..none
            },
            SettingsAction::FactSet(change) => Self {
                fact_set: Some(change),
                ..none
            },
            SettingsAction::PersonSet(change) => Self {
                person_set: Some(change),
                ..none
            },
            SettingsAction::FactDrop(fact) => Self {
                fact_drop: Some(fact),
                ..none
            },
            SettingsAction::TrialDrop(trial) => Self {
                trial_drop: Some(trial),
                ..none
            },
            SettingsAction::TrialKeep(trial) => Self {
                trial_keep: Some(trial),
                ..none
            },
            SettingsAction::XPosted { artifact, option } => Self {
                x_posted: Some(artifact),
                option: Some(option),
                ..none
            },
            SettingsAction::AuthRevoke(device) => Self {
                auth_revoke: Some(device),
                ..none
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(action: SettingsAction) -> String {
        serde_json::to_string(&SettingsRequest::from(action)).unwrap()
    }

    #[test]
    fn each_action_is_its_own_keys() {
        assert_eq!(wire(SettingsAction::SetOn(false)), r#"{"on":false}"#);
        assert_eq!(
            wire(SettingsAction::Interrupt(RoomId::new(2))),
            r#"{"interrupt":true,"room":2}"#
        );
        assert_eq!(
            wire(SettingsAction::TodoPark {
                task: TaskId::new(3),
                until: None
            }),
            r#"{"todoPark":3}"#
        );
        assert_eq!(
            wire(SettingsAction::TaskDecide(AnswerRequest::new(
                TaskId::new(4),
                "Yes"
            ))),
            r#"{"taskDecide":4,"choice":"Yes"}"#
        );
        assert_eq!(
            wire(SettingsAction::TrialDrop(TrialId::new(9))),
            r#"{"trialDrop":9}"#
        );
        assert_eq!(
            wire(SettingsAction::TrialKeep(TrialId::new(9))),
            r#"{"trialKeep":9}"#
        );
    }

    #[test]
    fn a_change_leaves_out_what_it_keeps() {
        let mut change = TaskChange::new(TaskId::new(5));
        change.deadline = Patch::Clear;
        change.ultra = Some(true);
        assert_eq!(
            wire(SettingsAction::TaskSet(change)),
            r#"{"taskSet":{"id":5,"deadline":null,"ultra":true}}"#
        );
        let project = ProjectChange::new(ProjectId::new(2));
        assert_eq!(
            wire(SettingsAction::ProjectSet(project)),
            r#"{"projectSet":{"id":2}}"#
        );
    }
}
