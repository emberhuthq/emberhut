//! `log`: the activity log, one line per thing done.

#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model derive writes async trait methods without an await for a table with no relations"
)]

use sea_orm::entity::prelude::*;

/// A line of the log.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "log")]
pub(crate) struct Model {
    /// The line's id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// When it was written.
    pub(crate) at: i64,
    /// What was done.
    pub(crate) text: String,
}

impl ActiveModelBehavior for ActiveModel {}
