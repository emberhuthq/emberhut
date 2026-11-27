//! Runs: every turn of work on a task. A thread's turn is opened when it is sent and closed
//! when it ends; a worker's session is one run from its start to its stop.

use std::collections::BTreeMap;

use emberhut_core::task::live::{NewRun, Run, RunKind};
use emberhut_core::task::{RunId, TaskId};
use jiff::Timestamp;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect,
};

use crate::DbError;
use crate::entity::run;
use crate::time::{instant, millis};

/// The longest `asked` a run keeps.
const ASKED: usize = 400;

/// Reads and writes runs.
#[derive(Clone, Debug)]
pub struct RunStore {
    conn: DatabaseConnection,
}

impl RunStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Opens a run: sent now (and started now, for a worker's session).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn open(&self, new: NewRun, now: Timestamp) -> Result<Run, DbError> {
        let asked = new
            .asked
            .map(|asked| asked.split_whitespace().collect::<Vec<_>>().join(" "))
            .map(|asked| asked.chars().take(ASKED).collect::<String>())
            .filter(|asked| !asked.is_empty());
        let row = run::ActiveModel {
            task: Set(Some(new.task.get())),
            machine: Set(new.machine),
            thread: Set(new.thread),
            turn: Set(None),
            kind: Set(new.kind.as_str().to_owned()),
            asked: Set(asked),
            requested: Set(Some(millis(now))),
            started: Set(new.started.then(|| millis(now))),
            ended: Set(None),
            outcome: Set(None),
            last: Set(None),
            status_line: Set(None),
            inferred: Set(false),
            applied: Set(false),
            ..Default::default()
        }
        .insert(&self.conn)
        .await?;
        from_row(row)
    }

    /// Closes a run with how it ended and its last answer; `false` when it was closed already.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn close(
        &self,
        id: RunId,
        outcome: &str,
        last: Option<&str>,
        now: Timestamp,
    ) -> Result<bool, DbError> {
        let Some(row) = run::Entity::find_by_id(id.get()).one(&self.conn).await? else {
            return Ok(false);
        };
        if row.ended.is_some() {
            return Ok(false);
        }
        let mut active = row.into_active_model();
        active.ended = Set(Some(millis(now)));
        active.outcome = Set(Some(outcome.to_owned()));
        active.last = Set(last.map(str::to_owned));
        active.applied = Set(true);
        active.update(&self.conn).await?;
        Ok(true)
    }

    /// The open worker run of `unit`, if there is one.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn open_worker(&self, unit: &str) -> Result<Option<Run>, DbError> {
        run::Entity::find()
            .filter(run::Column::Kind.eq(RunKind::Worker.as_str()))
            .filter(run::Column::Thread.eq(unit))
            .filter(run::Column::Ended.is_null())
            .one(&self.conn)
            .await?
            .map(from_row)
            .transpose()
    }

    /// A task's runs, newest first, `limit` at most.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn of_task(&self, task: TaskId, limit: u64) -> Result<Vec<Run>, DbError> {
        let mut runs = run::Entity::find()
            .filter(run::Column::Task.eq(task.get()))
            .order_by_desc(run::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect::<Result<Vec<_>, _>>()?;
        runs.sort_by(|a, b| {
            b.started
                .or(b.requested)
                .cmp(&a.started.or(a.requested))
                .then_with(|| b.id.cmp(&a.id))
        });
        runs.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        Ok(runs)
    }

    /// For each of `tasks`: its open run (a started one before a queued one, the newest first)
    /// and its last ended one.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn latest(
        &self,
        tasks: &[TaskId],
    ) -> Result<BTreeMap<TaskId, (Option<Run>, Option<Run>)>, DbError> {
        let mut by_task: BTreeMap<TaskId, (Option<Run>, Option<Run>)> = BTreeMap::new();
        if tasks.is_empty() {
            return Ok(by_task);
        }
        let open = run::Entity::find()
            .filter(run::Column::Task.is_in(tasks.iter().map(|t| t.get())))
            .filter(run::Column::Ended.is_null())
            .order_by_desc(run::Column::Id)
            .all(&self.conn)
            .await?;
        for row in open {
            let found = from_row(row)?;
            let Some(task) = found.task else { continue };
            let slot = &mut by_task.entry(task).or_default().0;
            let better = match slot {
                None => true,
                Some(kept) => kept.started.is_none() && found.started.is_some(),
            };
            if better {
                *slot = Some(found);
            }
        }
        for task in tasks {
            let last = run::Entity::find()
                .filter(run::Column::Task.eq(task.get()))
                .filter(run::Column::Ended.is_not_null())
                .order_by_desc(run::Column::Ended)
                .limit(1)
                .one(&self.conn)
                .await?;
            if let Some(last) = last {
                by_task.entry(*task).or_default().1 = Some(from_row(last)?);
            }
        }
        Ok(by_task)
    }
}

fn from_row(row: run::Model) -> Result<Run, DbError> {
    Ok(Run {
        id: RunId::new(row.id),
        task: row.task.map(TaskId::new),
        kind: RunKind::parse(&row.kind),
        requested: row.requested.map(instant).transpose()?,
        started: row.started.map(instant).transpose()?,
        ended: row.ended.map(instant).transpose()?,
        machine: row.machine,
        thread: row.thread,
        asked: row.asked,
        outcome: row.outcome,
        last: row.last,
        status_line: row.status_line,
        inferred: row.inferred,
        applied: row.applied,
    })
}
