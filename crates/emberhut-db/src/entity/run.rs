//! `runs`: every turn of work on a task.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A run.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "runs")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its task.
    pub(crate) task: Option<i64>,
    /// The machine it ran on.
    pub(crate) machine: String,
    /// Its thread, session or unit.
    pub(crate) thread: String,
    /// The thread's own id for the turn.
    pub(crate) turn: Option<String>,
    /// `thread` or `worker`.
    pub(crate) kind: String,
    /// What it was asked.
    pub(crate) asked: Option<String>,
    /// When it was sent.
    pub(crate) requested: Option<i64>,
    /// When it started.
    pub(crate) started: Option<i64>,
    /// When it ended.
    pub(crate) ended: Option<i64>,
    /// How it ended.
    pub(crate) outcome: Option<String>,
    /// Its last answer.
    pub(crate) last: Option<String>,
    /// Its status line.
    pub(crate) status_line: Option<String>,
    /// Matched to its task by a guess.
    pub(crate) inferred: bool,
    /// Its end was applied to its task.
    pub(crate) applied: bool,
}

impl ActiveModelBehavior for ActiveModel {}
