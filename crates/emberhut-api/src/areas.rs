//! A project's areas, in the state's `areas`: each a part of a project with its own tasks, as
//! a tree (a task, its steps under it), what in it waits on the person, and its findings.
//!
//! The server sends every active and paused area with its tree; a done one comes bare (its
//! counts zero, no tree), folded on the project's page.

use serde::{Deserialize, Serialize};

use crate::ids::{AreaId, ProjectId, TaskId};
use crate::open_enum::open_enum;
use crate::tasks::Landed;
use crate::time::{Day, Timestamp};

open_enum! {
    /// Where an area is, as the person set it.
    pub enum AreaStatus {
        /// Worked on.
        Active = "active",
        /// Put aside for now.
        Paused = "paused",
        /// Finished.
        Done = "done",
    }
}

open_enum! {
    /// Where a task (or an area) stands, rolled up from the tasks under it.
    pub enum WorkStage {
        /// Every task under it is done.
        Done = "done",
        /// Something under it is being worked on.
        Working = "working",
        /// Something under it waits on the person.
        NeedsYou = "needs_you",
        /// Nothing under it started yet.
        NotStarted = "not_started",
    }
}

/// An area with its tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Area {
    /// Its id.
    pub id: AreaId,
    /// Its project.
    pub pid: ProjectId,
    /// Its name.
    pub name: String,
    /// Where the person set it.
    pub status: AreaStatus,
    /// What done looks like for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// A short line on what it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// Where it stands, rolled up from its tasks.
    pub state: WorkStage,
    /// Its tasks' leaves by where they stand.
    #[serde(default)]
    pub counts: AreaCounts,
    /// What waits on the person in it, at any depth.
    #[serde(default)]
    pub questions: Vec<TreeNode>,
    /// Its findings at a glance.
    #[serde(default)]
    pub findings: FindingsSummary,
    /// Its tasks, each with its steps under it.
    #[serde(default)]
    pub tree: Vec<TreeNode>,
}

impl Area {
    /// Its tasks not done yet (the leaves): what its sidebar row counts.
    #[must_use]
    pub fn open(&self) -> u32 {
        self.counts.total.saturating_sub(self.counts.done)
    }
}

/// An area's leaves by where they stand.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AreaCounts {
    /// Done.
    pub done: u32,
    /// Being worked on.
    pub working: u32,
    /// Waiting on the person.
    pub needs_you: u32,
    /// Not started.
    pub not_started: u32,
    /// All of them.
    pub total: u32,
}

/// A task in an area's tree: its own status, where it stands rolled up from its leaves, and its
/// steps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TreeNode {
    /// The task.
    pub id: TaskId,
    /// Its title.
    pub title: String,
    /// Who does it (an [`Issue::owner`](crate::tasks::Issue::owner) value).
    pub owner: String,
    /// Its own status (an [`Issue::status`](crate::tasks::Issue::status) word).
    pub status: String,
    /// Where it stands, from its leaves.
    pub state: WorkStage,
    /// What the person should do, when it waits on them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// When it is due.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<Day>,
    /// When it was done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_at: Option<Timestamp>,
    /// A scan's findings, counted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub findings: Option<FindingCount>,
    /// Its steps.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(no_recursion))]
    pub kids: Vec<TreeNode>,
}

/// A scan's findings, counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FindingCount {
    /// All of them.
    pub total: u32,
    /// Those whose fix hasn't landed (not landed, partly, unchecked).
    pub open: u32,
}

/// An area's findings at a glance, and the ones still open.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FindingsSummary {
    /// All of them.
    pub total: u32,
    /// Landed.
    pub landed: u32,
    /// Partly landed.
    pub partly: u32,
    /// Not landed.
    pub not_landed: u32,
    /// Replaced by something else.
    pub superseded: u32,
    /// Not checked yet.
    pub unchecked: u32,
    /// Not landed, partly or unchecked.
    pub open: u32,
    /// The open ones: not landed first, then partly, then unchecked.
    #[serde(default)]
    pub unlanded: Vec<FindingItem>,
}

/// An open finding as its area lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FindingItem {
    /// The finding.
    pub id: TaskId,
    /// Its title.
    pub title: String,
    /// Whether its fix landed.
    pub landed: Landed,
    /// The scan that found it.
    pub scan: TaskId,
    /// That scan's title.
    pub scan_title: String,
    /// The task that fixes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_by: Option<TaskId>,
    /// That task's title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_by_title: Option<String>,
    /// That task's status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_by_status: Option<String>,
}
