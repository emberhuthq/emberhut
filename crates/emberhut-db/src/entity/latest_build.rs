//! `latest_builds`: the newest build reported per platform.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model derive writes async trait methods that a table with no relations \
              leaves without an await"
)]

use sea_orm::entity::prelude::*;

/// A platform's newest build.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "latest_builds")]
pub(crate) struct Model {
    /// The platform: `ios` or `mac`.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) platform: String,
    /// Its newest build.
    pub(crate) number: i64,
}

impl ActiveModelBehavior for ActiveModel {}
