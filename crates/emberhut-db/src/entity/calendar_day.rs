//! `calendar_days`: each calendar event on each day it covers, as last seen.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// An event on a day.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "calendar_days")]
pub(crate) struct Model {
    /// The day, `2026-10-19`.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) day: String,
    /// The event's id.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) id: String,
    /// The event, as JSON.
    pub(crate) event: String,
    /// When it was seen.
    pub(crate) seen: Option<i64>,
}

impl ActiveModelBehavior for ActiveModel {}
