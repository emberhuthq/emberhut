//! `health`: the phone's health summary of each day.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A day's summary.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "health")]
pub(crate) struct Model {
    /// The day, `2026-10-12`.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) date: String,
    /// The summary, as JSON.
    pub(crate) data: String,
    /// When it arrived.
    pub(crate) at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
