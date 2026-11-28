//! A task's plan: the ordered steps its agent works through, each ticked as it goes, so a task
//! shows its own progress.
//!
//! - [`Plan`]: every step, in order, on the task's page (`GET /api/tasks/{id}`'s `plan`).
//! - [`PlanSummary`]: the plan at a glance, on every task the state lists (`Issue::plan`): how
//!   many steps are done of how many, the step being worked on, what blocks it, the agent's
//!   estimate and when it last changed.
//!
//! Steps nest one level: a step with a `parent` is a sub-step of that top-level step, listed
//! right after it. A top-level step with sub-steps counts as its sub-steps; a skipped step
//! counts in neither `done` nor `total`.
//!
//! A task the server keeps plans for always has one, empty until its agent writes it: a client
//! shows an empty plan (`total` 0) as "No plan yet". Servers that keep no plans, or have them
//! turned off, send no `plan` at all.

use serde::{Deserialize, Serialize};

use crate::open_enum::open_enum;
use crate::time::Timestamp;

/// A step's id: stable for the life of its plan, unique within its task.
pub type PlanStepId = u32;

open_enum! {
    /// Where a step is.
    pub enum StepState {
        /// Not started.
        Todo = "todo",
        /// Being worked on now.
        Doing = "doing",
        /// Done: ticked.
        Done = "done",
        /// Stuck: its `note` says on what.
        Blocked = "blocked",
        /// Left out: not needed after all.
        Skipped = "skipped",
    }
}

/// One step of a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PlanStep {
    /// Its id.
    pub id: PlanStepId,
    /// The top-level step it is a sub-step of; none for a top-level step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<PlanStepId>,
    /// What the step is, one short line (`Build the Mac app`).
    pub title: String,
    /// Where it is.
    pub state: StepState,
    /// A note: what it came to, or what blocks it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// A link to what it produced (a pull request, a page, a file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// When it was first worked on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<Timestamp>,
    /// When it was done or skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished: Option<Timestamp>,
    /// When it last changed.
    pub updated: Timestamp,
}

/// The step a task is on: the first one being worked on, else the first not started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CurrentStep {
    /// Its id.
    pub id: PlanStepId,
    /// Its title.
    pub title: String,
    /// Its state: `doing`, `todo` or `blocked`.
    pub state: StepState,
}

/// A plan at a glance.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PlanSummary {
    /// Steps done (sub-steps in place of the step they make up; skipped ones not counted).
    pub done: u32,
    /// Steps in all, counted the same way. 0: no plan yet.
    pub total: u32,
    /// The step being worked on, or next.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<CurrentStep>,
    /// What blocks the plan: the first blocked step's note, else its title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    /// The agent's estimate of what is left, in words (`about 2 hours`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eta: Option<String>,
    /// When the plan last changed; none while it is empty and never written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<Timestamp>,
    /// A turn of work on the task ended while its plan was still empty: when the last one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unplanned_turn: Option<Timestamp>,
}

impl PlanSummary {
    /// Whether the plan has no steps yet.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.total == 0
    }
}

/// A task's whole plan.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Plan {
    /// Every step in order: each top-level step, then its sub-steps.
    #[serde(default)]
    pub steps: Vec<PlanStep>,
    /// The plan at a glance, as the task's row shows it.
    pub summary: PlanSummary,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_reads_and_writes_its_wire_words() {
        let json = serde_json::json!({
            "id": 3, "parent": 1, "title": "Build the Mac app", "state": "doing",
            "updated": "2026-11-28T06:50:17.291Z"
        });
        let step: PlanStep = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(step.state, StepState::Doing);
        assert_eq!(step.parent, Some(1));
        assert_eq!(serde_json::to_value(&step).unwrap(), json);
    }

    #[test]
    fn an_empty_summary_is_no_plan_yet() {
        let summary: PlanSummary = serde_json::from_str(r#"{"done":0,"total":0}"#).unwrap();
        assert!(summary.is_empty());
        assert_eq!(
            serde_json::to_string(&summary).unwrap(),
            r#"{"done":0,"total":0}"#
        );
    }
}
