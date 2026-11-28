//! `plan_steps`: the steps of each task's plan, in order.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A step of a task's plan.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "plan_steps")]
pub(crate) struct Model {
    /// Its row's id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its task.
    pub(crate) issue: i64,
    /// Its id within its task's plan.
    pub(crate) step: i32,
    /// The top-level step it is a sub-step of.
    pub(crate) parent: Option<i32>,
    /// Its place in the plan's list.
    pub(crate) position: i32,
    /// What the step is.
    pub(crate) title: String,
    /// `todo`, `doing`, `done`, `blocked` or `skipped`.
    pub(crate) state: String,
    /// What it came to, or what blocks it.
    pub(crate) note: Option<String>,
    /// A link to what it produced.
    pub(crate) link: Option<String>,
    /// When it was first worked on.
    pub(crate) started: Option<i64>,
    /// When it was done or skipped.
    pub(crate) finished: Option<i64>,
    /// When it was added.
    pub(crate) created: i64,
    /// When it last changed.
    pub(crate) updated: i64,
}

impl ActiveModelBehavior for ActiveModel {}
