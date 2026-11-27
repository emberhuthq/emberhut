//! `events`: what programs and the server tell the assistant, per room.

#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model derive writes async trait methods without an await for a table with no relations"
)]

use sea_orm::entity::prelude::*;

/// An event.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "events")]
pub(crate) struct Model {
    /// The event's id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// The room whose session reads it.
    pub(crate) room_id: i64,
    /// What kind of news it is.
    pub(crate) kind: String,
    /// The news.
    pub(crate) text: String,
    /// When it was stored.
    pub(crate) at: i64,
    /// Makes it happen once.
    #[sea_orm(unique)]
    pub(crate) key: Option<String>,
    /// Whether it has been read.
    pub(crate) handled: bool,
}

impl ActiveModelBehavior for ActiveModel {}
