//! `chrome_tasks`: the browser a task works in.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model derive writes async trait methods that a table with no relations \
              leaves without an await"
)]

use sea_orm::entity::prelude::*;

/// A task's own browser.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "chrome_tasks")]
pub(crate) struct Model {
    /// The task.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) task: i64,
    /// The profile it works in, or 0 for the server's own browser.
    pub(crate) device: i64,
    /// When it was set, in ms.
    pub(crate) at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
