//! `settings`: one JSON value per key.

#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model derive writes async trait methods without an await for a table with no relations"
)]

use sea_orm::entity::prelude::*;

/// A setting.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "settings")]
pub(crate) struct Model {
    /// Its key.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) key: String,
    /// Its value, as JSON text.
    pub(crate) value: String,
}

impl ActiveModelBehavior for ActiveModel {}
