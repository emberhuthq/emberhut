//! `projects`: what tasks are grouped by.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A project.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "projects")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its name.
    pub(crate) name: String,
    /// The goal it serves.
    pub(crate) goal: Option<i64>,
    /// 0 none, 1 urgent ... 4 low.
    pub(crate) priority: i32,
    /// `deliverable` or `ongoing`.
    pub(crate) kind: String,
    /// When it ends, `YYYY-MM-DD`.
    pub(crate) deadline: Option<String>,
    /// That deadline is someone else's.
    pub(crate) hard: bool,
    /// `active`, `paused`, `archived` or `done`.
    pub(crate) status: String,
    /// A paused project's day to come back.
    pub(crate) paused_until: Option<String>,
    /// When it was made.
    pub(crate) created: i64,
    /// When it last changed.
    pub(crate) updated: i64,
}

impl ActiveModelBehavior for ActiveModel {}
