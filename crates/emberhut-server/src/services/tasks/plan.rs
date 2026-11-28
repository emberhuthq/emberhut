//! A task's plan, as its agents keep it (the plan tools) and the apps read it (every task the
//! state lists, and its page), on a server that keeps plans (`--task-plans`).
//!
//! Every change writes one line of the task's history and changes the task's `updated`, so the
//! state's fingerprint changes and an open task list or page fetches it again. A turn of work
//! that ends while the task's plan is empty is noted ([`TaskService::turn_ended`]).

use std::collections::BTreeMap;

use emberhut_api as api;
use emberhut_core::guard::key_mask;
use emberhut_core::task::plan::{self, Edit, NewStep, Step};
use emberhut_core::task::{TaskId, TaskRefusal, Who};
use emberhut_db::StoredPlan;

use super::{TaskError, TaskService};
use crate::convert::timestamp_to_api;

impl TaskService {
    /// Whether this server keeps tasks' plans.
    #[must_use]
    pub fn keeps_plans(&self) -> bool {
        self.task_plans
    }

    /// The plans at a glance of `ids` (none when the server keeps no plans), in two queries.
    pub(super) async fn plan_summaries(
        &self,
        ids: &[TaskId],
    ) -> Result<BTreeMap<TaskId, api::task_plan::PlanSummary>, TaskError> {
        if !self.task_plans {
            return Ok(BTreeMap::new());
        }
        Ok(self
            .tasks
            .plans(ids)
            .await?
            .iter()
            .map(|(id, stored)| (*id, summary(stored)))
            .collect())
    }

    /// The task `id`'s whole plan as its page shows it; none when the server keeps no plans,
    /// or there is no such task.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn plan_of(&self, id: TaskId) -> Result<Option<api::task_plan::Plan>, TaskError> {
        if !self.task_plans {
            return Ok(None);
        }
        Ok(self.tasks.plan(id).await?.as_ref().map(whole))
    }

    /// The task `id`'s plan, as stored.
    ///
    /// # Errors
    ///
    /// [`TaskError::Refused`] when there is no such task, [`TaskError::Db`].
    pub async fn stored_plan(&self, id: TaskId) -> Result<StoredPlan, TaskError> {
        self.tasks
            .plan(id)
            .await?
            .ok_or_else(|| TaskRefusal::NoTask(id).into())
    }

    /// Replaces the task `id`'s plan with `steps`, as `who` wrote it ([`plan::set`]); its
    /// estimate too when `eta` is given (`Some(None)` clears it). A key or token in a title or
    /// note is masked: the plan is shown in the apps.
    ///
    /// # Errors
    ///
    /// [`TaskError::Refused`] with the rule's reason, [`TaskError::Db`].
    pub async fn set_plan(
        &self,
        id: TaskId,
        mut steps: Vec<NewStep>,
        eta: Option<Option<String>>,
        who: Who,
    ) -> Result<StoredPlan, TaskError> {
        for step in &mut steps {
            mask_new(step);
        }
        Ok(self
            .tasks
            .set_plan(id, &steps, eta, who, self.now())
            .await?)
    }

    /// Changes one step of the task `id`'s plan, or adds one, as `who` asked ([`plan::edit`]);
    /// its estimate too when `eta` is given (`Some(None)` clears it). Keys are masked as in
    /// [`Self::set_plan`].
    ///
    /// # Errors
    ///
    /// [`TaskError::Refused`] with the rule's reason, [`TaskError::Db`].
    pub async fn edit_plan(
        &self,
        id: TaskId,
        mut edit: Edit,
        eta: Option<Option<String>>,
        who: Who,
    ) -> Result<StoredPlan, TaskError> {
        if let Some(add) = &mut edit.add {
            add.title = masked(&add.title);
        }
        edit.title = edit.title.as_deref().map(masked);
        edit.note = edit.note.as_deref().map(masked);
        Ok(self
            .tasks
            .edit_plan(id, &edit, eta, who, self.now())
            .await?)
    }

    /// A turn of work on the task `id` ended (a worker's session, a runner session's turn):
    /// on a server that keeps plans, a task whose plan is still empty is marked, and the first
    /// such turn in a row noted in its history.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn turn_ended(&self, id: TaskId) -> Result<(), TaskError> {
        if self.task_plans && self.tasks.turn_unplanned(id, self.now()).await? {
            tracing::info!(task = %id.key(), "a turn of work ended with no plan");
        }
        Ok(())
    }
}

/// `text` with any key or token in it masked.
fn masked(text: &str) -> String {
    key_mask::mask(text, &[], None)
}

/// `step` and its sub-steps with their titles and notes masked.
fn mask_new(step: &mut NewStep) {
    step.title = masked(&step.title);
    step.note = step.note.as_deref().map(masked);
    for sub in &mut step.steps {
        mask_new(sub);
    }
}

/// A plan at a glance as the contract writes it.
pub(crate) fn summary(stored: &StoredPlan) -> api::task_plan::PlanSummary {
    let glance = plan::summary(&stored.steps);
    api::task_plan::PlanSummary {
        done: glance.done,
        total: glance.total,
        current: glance.current.map(|step| api::task_plan::CurrentStep {
            id: step.id,
            title: step.title.clone(),
            state: api::task_plan::StepState::from(step.state.as_str()),
        }),
        blocker: glance.blocker,
        eta: stored.eta.clone(),
        updated: stored.updated.map(timestamp_to_api),
        unplanned_turn: stored.unplanned_turn.map(timestamp_to_api),
    }
}

/// A whole plan as the contract writes it.
pub(crate) fn whole(stored: &StoredPlan) -> api::task_plan::Plan {
    api::task_plan::Plan {
        steps: stored.steps.iter().map(step).collect(),
        summary: summary(stored),
    }
}

/// A step as the contract writes it.
fn step(step: &Step) -> api::task_plan::PlanStep {
    api::task_plan::PlanStep {
        id: step.id,
        parent: step.parent,
        title: step.title.clone(),
        state: api::task_plan::StepState::from(step.state.as_str()),
        note: step.note.clone(),
        link: step.link.clone(),
        started: step.started.map(timestamp_to_api),
        finished: step.finished.map(timestamp_to_api),
        updated: timestamp_to_api(step.updated),
    }
}
