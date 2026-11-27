//! `screentime`: screen-time totals per day and device.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A device's day.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "screentime")]
pub(crate) struct Model {
    /// The day, `2026-10-12`.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) date: String,
    /// The device (`iPhone`, `Mac`).
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) device: String,
    /// The totals, as JSON.
    pub(crate) data: String,
    /// When they arrived.
    pub(crate) at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
