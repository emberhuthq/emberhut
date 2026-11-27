//! `latest_statements`: each machine's latest accepted link or unlink time, kept after an
//! unlink, so an older statement can't undo a newer one.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A machine's latest statement.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "latest_statements")]
pub(crate) struct Model {
    /// The machine's id.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) machine: String,
    /// The time its latest accepted link or unlink statement names.
    pub(crate) time: i64,
}

impl ActiveModelBehavior for ActiveModel {}
