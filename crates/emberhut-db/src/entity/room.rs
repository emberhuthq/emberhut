//! `rooms`: conversations. Main is made by the first migration.

use sea_orm::entity::prelude::*;

/// A room.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "rooms")]
pub(crate) struct Model {
    /// The room's id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its name, as the room switcher shows it.
    pub(crate) name: String,
    /// When the room was made.
    pub(crate) created_at: i64,
    /// When the user last looked at it.
    pub(crate) seen_at: Option<i64>,
    /// The newest message there when the user last looked: replies after it are unread.
    pub(crate) seen_through: Option<i64>,
    /// The project whose room it is; none for Main and every other room.
    pub(crate) project_id: Option<i64>,
    /// What was said in it.
    #[sea_orm(has_many)]
    pub(crate) messages: HasMany<super::message::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
