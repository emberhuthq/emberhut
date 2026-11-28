//! `issues`: every task, the person's and the assistant's, and the findings filed under them.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]
#![expect(
    clippy::struct_field_names,
    reason = "`model` is the column of the task's model, not a name for the row"
)]
#![expect(
    clippy::struct_excessive_bools,
    reason = "each flag is a column of the table"
)]

use sea_orm::entity::prelude::*;

/// A task.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "issues")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its title.
    pub(crate) title: String,
    /// Its brief.
    pub(crate) body: Option<String>,
    /// Whose it is.
    pub(crate) owner: String,
    /// Where it is.
    pub(crate) status: String,
    /// Its own goal.
    pub(crate) goal: Option<i64>,
    /// Its project's name, as given.
    pub(crate) project: Option<String>,
    /// Its project.
    pub(crate) pid: Option<i64>,
    /// Its deadline is someone else's.
    pub(crate) hard: bool,
    /// Its rough size.
    pub(crate) size: Option<String>,
    /// 0 none, 1 urgent ... 4 low.
    pub(crate) priority: i32,
    /// Its own deadline, `YYYY-MM-DD`.
    pub(crate) due: Option<String>,
    /// Pinned to Today.
    pub(crate) today: bool,
    /// Out of sight until then.
    pub(crate) parked_until: Option<String>,
    /// An ask's day to decide by.
    pub(crate) decide_by: Option<String>,
    /// What it asks (JSON).
    pub(crate) decision: Option<String>,
    /// The history line of its latest ask (kept when the ask is answered or cleared).
    pub(crate) last_ask: Option<i64>,
    /// What the person should do.
    pub(crate) action: Option<String>,
    /// When it was made.
    pub(crate) created: i64,
    /// When it last changed.
    pub(crate) updated: i64,
    /// When it was done.
    pub(crate) done_at: Option<i64>,
    /// Its newest history line.
    pub(crate) last_note: Option<String>,
    /// Its parent.
    pub(crate) parent: Option<i64>,
    /// Its count so far.
    pub(crate) progress: Option<i64>,
    /// What its count goes to.
    pub(crate) total: Option<i64>,
    /// What it counts.
    pub(crate) count_unit: Option<String>,
    /// Done elsewhere.
    pub(crate) tracked: bool,
    /// When it is expected done.
    pub(crate) eta: Option<String>,
    /// Ultra mode.
    pub(crate) ultra: bool,
    /// Ultra: sub-agents at a time.
    pub(crate) ultra_parallel: Option<i32>,
    /// Ultra: the run's token cap.
    pub(crate) ultra_budget: Option<i64>,
    /// Its model.
    pub(crate) model: Option<String>,
    /// Its area.
    pub(crate) area: Option<i64>,
    /// `finding`, or none for a task.
    pub(crate) kind: Option<String>,
    /// A finding's state.
    pub(crate) landed: Option<String>,
    /// A finding's fixing task.
    pub(crate) fixed_by: Option<i64>,
    /// What does the work when it isn't a turn.
    pub(crate) runner: Option<String>,
    /// What its worker carries on with.
    pub(crate) next: Option<String>,
    /// Its worker's unit, while one runs.
    pub(crate) unit: Option<String>,
    /// Its worker's session.
    pub(crate) session: Option<String>,
    /// The person's own request for it (JSON).
    pub(crate) requested: Option<String>,
    /// The room that made it, when that isn't its project's.
    pub(crate) room: Option<i64>,
    /// An ultra task's run (JSON).
    pub(crate) ultra_run: Option<String>,
    /// Its worker's current stream file.
    pub(crate) turn_file: Option<String>,
    pub(crate) requested_by: Option<String>,
    /// When its plan last changed.
    pub(crate) plan_updated: Option<i64>,
    /// The highest step id its plan has given out.
    pub(crate) plan_last_step: Option<i32>,
    /// When the last turn of work on it ended with its plan empty.
    pub(crate) unplanned_turn: Option<i64>,
}

impl ActiveModelBehavior for ActiveModel {}
