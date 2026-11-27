//! `issue_events`: each task's history, oldest first.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A line of a task's history.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "issue_events")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its task.
    pub(crate) issue: i64,
    /// When.
    pub(crate) at: i64,
    /// Who wrote it.
    pub(crate) who: String,
    /// What happened.
    pub(crate) text: String,
    /// On the person's answer to an ask: the history line of the ask it answered.
    pub(crate) answers: Option<i64>,
}

impl ActiveModelBehavior for ActiveModel {}
