//! `machine_latest`: each runner machine's latest sample of itself.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A machine's latest sample.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "machine_latest")]
pub(crate) struct Model {
    /// The machine, as its runner names itself.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) machine: String,
    /// When the sample came.
    pub(crate) at: i64,
    /// The sample, as JSON.
    pub(crate) data: String,
}

impl ActiveModelBehavior for ActiveModel {}
