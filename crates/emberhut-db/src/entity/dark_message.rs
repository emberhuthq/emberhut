//! `dark_messages`: the off-the-record room's chat, in its own database.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// One line of the room.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "dark_messages")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// `user` or `assistant`.
    pub(crate) role: String,
    /// What was said.
    pub(crate) text: String,
    /// When.
    pub(crate) at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
