//! `marks`: how far recall has indexed each of its sources.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A mark.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "marks")]
pub(crate) struct Model {
    /// The source.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) key: String,
    /// How far, as the source counts.
    pub(crate) value: String,
}

impl ActiveModelBehavior for ActiveModel {}
