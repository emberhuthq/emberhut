//! `machine_points`: a day of 5-minute points per runner machine.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// One machine's point.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "machine_points")]
pub(crate) struct Model {
    /// The machine.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) machine: String,
    /// The start of its 5 minutes, in milliseconds since the epoch.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) bucket: i64,
    /// The point, as JSON.
    pub(crate) data: String,
}

impl ActiveModelBehavior for ActiveModel {}
