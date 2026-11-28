//! A task's plan: its steps (`plan_steps`), when it last changed, the last step id it gave out
//! and when a turn of work last ended with it empty (columns of `issues`); its estimate is the
//! task's `eta`. A change goes through `emberhut-core`'s rules ([`emberhut_core::task::plan`])
//! and is written, with its one history line, in one transaction: the steps are written again
//! as the rules leave them.

#![expect(
    clippy::option_option,
    reason = "an estimate's change: left out is unchanged, none clears it"
)]

use std::collections::BTreeMap;

use emberhut_core::task::plan::{
    self, Edit, NewStep, Outcome, PlanRefusal, Step, StepId, StepState, UNPLANNED_TURN_LINE,
};
use emberhut_core::task::{TaskId, TaskRefusal, Who};
use jiff::Timestamp;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
};

use super::{TaskStore, TaskStoreError, note};
use crate::entity::{issue, plan_step};
use crate::time::{instant, millis};
use crate::{DbError, begin_write};

/// A task's plan as stored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoredPlan {
    /// Every step, in order: each top-level step, then its sub-steps.
    pub steps: Vec<Step>,
    /// The highest step id the task has given out.
    pub last_id: StepId,
    /// The estimate of what is left (the task's `eta`).
    pub eta: Option<String>,
    /// When the plan last changed; none while it was never written.
    pub updated: Option<Timestamp>,
    /// When the last turn of work on the task ended while its plan was empty.
    pub unplanned_turn: Option<Timestamp>,
}

/// A task's own columns of its plan: `(id, eta, plan_updated, plan_last_step, unplanned_turn)`.
type PlanColumns = (i64, Option<String>, Option<i64>, Option<i32>, Option<i64>);

impl TaskStore {
    /// The plan of the task `id`; none when there is no such task.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn plan(&self, id: TaskId) -> Result<Option<StoredPlan>, DbError> {
        Ok(self.plans(&[id]).await?.remove(&id))
    }

    /// The plans of the tasks `ids` that exist (an empty one for a task with no steps), in two
    /// queries whatever their number.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn plans(&self, ids: &[TaskId]) -> Result<BTreeMap<TaskId, StoredPlan>, DbError> {
        plans(&self.conn, ids).await
    }

    /// Replaces the plan of the task `id` with `steps` ([`plan::set`]), and its estimate when
    /// `eta` is given (`Some(None)` clears it), as `by` wrote it: one history line, and no
    /// unplanned turn any more.
    ///
    /// # Errors
    ///
    /// [`TaskStoreError::Refused`] when there is no such task, [`TaskStoreError::Plan`] when a
    /// plan rule says no, [`TaskStoreError::Db`].
    pub async fn set_plan(
        &self,
        id: TaskId,
        steps: &[NewStep],
        eta: Option<Option<String>>,
        by: Who,
        now: Timestamp,
    ) -> Result<StoredPlan, TaskStoreError> {
        let txn = begin_write(&self.conn).await?;
        let was = held(&txn, id).await?;
        let outcome = plan::set(&was.steps, was.last_id, steps, now)?;
        let eta = eta.map(|eta| checked_eta(eta.as_deref())).transpose()?;
        write(&txn, id, &outcome, eta, by, now).await?;
        let stored = held(&txn, id).await?;
        txn.commit().await?;
        Ok(stored)
    }

    /// Changes one step of the plan of the task `id` ([`plan::edit`]), or adds one, and its
    /// estimate when `eta` is given (`Some(None)` clears it), as `by` asked: one history line.
    ///
    /// # Errors
    ///
    /// [`TaskStoreError::Refused`] when there is no such task, [`TaskStoreError::Plan`] when a
    /// plan rule says no (nothing would change, too), [`TaskStoreError::Db`].
    pub async fn edit_plan(
        &self,
        id: TaskId,
        edit: &Edit,
        eta: Option<Option<String>>,
        by: Who,
        now: Timestamp,
    ) -> Result<StoredPlan, TaskStoreError> {
        let txn = begin_write(&self.conn).await?;
        let was = held(&txn, id).await?;
        let eta_given = eta.is_some();
        let eta = eta.map(|eta| checked_eta(eta.as_deref())).transpose()?;
        let eta = eta.filter(|eta| *eta != was.eta);
        let changed = if edit.step.is_none() && edit.add.is_none() && eta_given {
            None
        } else {
            match plan::edit(&was.steps, was.last_id, edit, now) {
                Ok(outcome) => Some(outcome),
                Err(PlanRefusal::Nothing) if eta.is_some() => None,
                Err(refusal) => return Err(refusal.into()),
            }
        };
        let outcome = match (changed, &eta) {
            (Some(outcome), None) => outcome,
            (Some(mut outcome), Some(eta)) => {
                outcome.line = format!("{}; {}", outcome.line, plan::eta_line(eta.as_deref()));
                outcome
            }
            (None, Some(eta)) => Outcome {
                steps: was.steps,
                last_id: was.last_id,
                line: plan::eta_line(eta.as_deref()),
            },
            (None, None) => return Err(PlanRefusal::Nothing.into()),
        };
        write(&txn, id, &outcome, eta, by, now).await?;
        let stored = held(&txn, id).await?;
        txn.commit().await?;
        Ok(stored)
    }

    /// A turn of work on the task `id` ended at `now`: when its plan is empty, that is kept,
    /// and the first of such turns in a row writes [`UNPLANNED_TURN_LINE`] to its history.
    /// Whether the line was written (false too when there is no such task, or it has a plan).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn turn_unplanned(&self, id: TaskId, now: Timestamp) -> Result<bool, DbError> {
        let txn = begin_write(&self.conn).await?;
        let Some(row) = issue::Entity::find_by_id(id.get()).one(&txn).await? else {
            return Ok(false);
        };
        let steps = plan_step::Entity::find()
            .filter(plan_step::Column::Issue.eq(id.get()))
            .count(&txn)
            .await?;
        if steps > 0 {
            return Ok(false);
        }
        issue::Entity::update_many()
            .col_expr(issue::Column::UnplannedTurn, Expr::value(millis(now)))
            .filter(issue::Column::Id.eq(id.get()))
            .exec(&txn)
            .await?;
        let first = row.unplanned_turn.is_none();
        if first {
            note(&txn, id, Who::Assistant, UNPLANNED_TURN_LINE, now).await?;
        }
        txn.commit().await?;
        Ok(first)
    }
}

/// `eta` checked ([`plan::eta`]).
fn checked_eta(eta: Option<&str>) -> Result<Option<String>, PlanRefusal> {
    eta.map_or(Ok(None), plan::eta)
}

/// The plan of the task `id`, which must exist.
async fn held(conn: &impl ConnectionTrait, id: TaskId) -> Result<StoredPlan, TaskStoreError> {
    plans(conn, &[id])
        .await?
        .remove(&id)
        .ok_or_else(|| TaskRefusal::NoTask(id).into())
}

/// Writes `outcome` as the task's plan, and its estimate when one is given; with its history
/// line, and the task's plan marked changed.
async fn write(
    conn: &impl ConnectionTrait,
    id: TaskId,
    outcome: &Outcome,
    eta: Option<Option<String>>,
    by: Who,
    now: Timestamp,
) -> Result<(), DbError> {
    plan_step::Entity::delete_many()
        .filter(plan_step::Column::Issue.eq(id.get()))
        .exec(conn)
        .await?;
    let rows: Vec<plan_step::ActiveModel> = outcome
        .steps
        .iter()
        .enumerate()
        .map(|(position, step)| plan_step::ActiveModel {
            issue: Set(id.get()),
            step: Set(to_column(step.id)),
            parent: Set(step.parent.map(to_column)),
            position: Set(i32::try_from(position).unwrap_or(i32::MAX)),
            title: Set(step.title.clone()),
            state: Set(step.state.as_str().to_owned()),
            note: Set(step.note.clone()),
            link: Set(step.link.clone()),
            started: Set(step.started.map(millis)),
            finished: Set(step.finished.map(millis)),
            created: Set(millis(step.created)),
            updated: Set(millis(step.updated)),
            ..Default::default()
        })
        .collect();
    if !rows.is_empty() {
        plan_step::Entity::insert_many(rows).exec(conn).await?;
    }
    let mut update = issue::Entity::update_many()
        .col_expr(issue::Column::PlanUpdated, Expr::value(millis(now)))
        .col_expr(
            issue::Column::PlanLastStep,
            Expr::value(to_column(outcome.last_id)),
        )
        .col_expr(
            issue::Column::UnplannedTurn,
            Expr::value(Option::<i64>::None),
        );
    if let Some(eta) = eta {
        update = update.col_expr(issue::Column::Eta, Expr::value(eta));
    }
    update
        .filter(issue::Column::Id.eq(id.get()))
        .exec(conn)
        .await?;
    note(conn, id, by, &outcome.line, now).await?;
    Ok(())
}

/// A step id as its column keeps it.
fn to_column(id: StepId) -> i32 {
    i32::try_from(id).unwrap_or(i32::MAX)
}

/// A step id from its column; a negative one (never written) reads as 0.
fn from_column(id: i32) -> StepId {
    StepId::try_from(id).unwrap_or_default()
}

async fn plans(
    conn: &impl ConnectionTrait,
    ids: &[TaskId],
) -> Result<BTreeMap<TaskId, StoredPlan>, DbError> {
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let numbers: Vec<i64> = ids.iter().map(|id| id.get()).collect();
    let tasks: Vec<PlanColumns> = issue::Entity::find()
        .select_only()
        .column(issue::Column::Id)
        .column(issue::Column::Eta)
        .column(issue::Column::PlanUpdated)
        .column(issue::Column::PlanLastStep)
        .column(issue::Column::UnplannedTurn)
        .filter(issue::Column::Id.is_in(numbers.clone()))
        .into_tuple()
        .all(conn)
        .await?;
    let mut by_task: BTreeMap<TaskId, StoredPlan> = BTreeMap::new();
    for (id, eta, updated, last_id, unplanned) in tasks {
        by_task.insert(
            TaskId::new(id),
            StoredPlan {
                steps: Vec::new(),
                last_id: last_id.map_or(0, from_column),
                eta,
                updated: updated.map(instant).transpose()?,
                unplanned_turn: unplanned.map(instant).transpose()?,
            },
        );
    }
    let rows = plan_step::Entity::find()
        .filter(plan_step::Column::Issue.is_in(numbers))
        .order_by_asc(plan_step::Column::Issue)
        .order_by_asc(plan_step::Column::Position)
        .all(conn)
        .await?;
    for row in rows {
        let Some(stored) = by_task.get_mut(&TaskId::new(row.issue)) else {
            continue;
        };
        let step = from_row(row)?;
        stored.last_id = stored.last_id.max(step.id);
        stored.steps.push(step);
    }
    Ok(by_task)
}

fn from_row(row: plan_step::Model) -> Result<Step, DbError> {
    Ok(Step {
        id: from_column(row.step),
        parent: row.parent.map(from_column),
        state: StepState::parse(&row.state).ok_or_else(|| DbError::CorruptValue {
            column: "plan_steps.state",
            value: row.state.clone(),
        })?,
        title: row.title,
        note: row.note,
        link: row.link,
        started: row.started.map(instant).transpose()?,
        finished: row.finished.map(instant).transpose()?,
        created: instant(row.created)?,
        updated: instant(row.updated)?,
    })
}
