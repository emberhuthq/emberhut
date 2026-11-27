//! `family_rooms`: the family member each of their rooms belongs to.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A family member's room.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "family_rooms")]
pub(crate) struct Model {
    /// The room.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) room_id: i64,
    /// Whose it is.
    pub(crate) person: String,
}

impl ActiveModelBehavior for ActiveModel {}
