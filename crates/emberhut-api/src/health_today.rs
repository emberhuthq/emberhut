//! Today's numbers from the person's phone (a state's `health`): steps toward a daily target,
//! exercise minutes, and the walk tasks whose rows show today's walk toward their target.
//!
//! The server keeps the numbers as doubles; they are read as `f64`.

use serde::{Deserialize, Serialize};

use crate::ids::{ProjectId, TaskId};
use crate::time::Timestamp;

/// Today's numbers, on the phone's own day.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HealthToday {
    /// The phone's day, `YYYY-MM-DD`.
    pub date: String,
    /// Steps so far.
    pub steps: Option<f64>,
    /// Exercise minutes so far.
    pub exercise_min: Option<f64>,
    /// The day's walk in minutes (its longest walking workout, or its exercise minutes).
    pub move_min: Option<f64>,
    /// The daily steps target.
    pub steps_target: f64,
    /// When the phone last sent its days.
    pub at: Option<Timestamp>,
    /// The project the health tasks sit in.
    pub project: Option<ProjectId>,
    /// The walk tasks, each with its target in minutes.
    #[serde(default)]
    pub tasks: Vec<WalkTask>,
}

impl HealthToday {
    /// The walk target of `task`, in minutes, when it is a walk task.
    #[must_use]
    pub fn walk_target(&self, task: TaskId) -> Option<f64> {
        self.tasks
            .iter()
            .find(|walk| walk.id == task)
            .map(|walk| walk.target)
    }
}

/// A walk task and its target.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WalkTask {
    /// The task.
    pub id: TaskId,
    /// Its target, in minutes.
    pub target: f64,
}
