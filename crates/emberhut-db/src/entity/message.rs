//! `messages`: what was said in each room, oldest first by id.

use sea_orm::entity::prelude::*;

/// Who a message is from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
pub(crate) enum Author {
    /// The person using the app.
    #[sea_orm(string_value = "user")]
    User,
    /// The assistant's reply.
    #[sea_orm(string_value = "assistant")]
    Assistant,
    /// A note from the server itself (an event), not part of the conversation.
    #[sea_orm(string_value = "event")]
    Event,
}

/// A message.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "messages")]
pub(crate) struct Model {
    /// The message's id; ids grow, so they order a room's messages.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// The room it was said in.
    pub(crate) room_id: i64,
    /// Who said it.
    pub(crate) author: Author,
    /// What was said.
    pub(crate) text: String,
    /// When it was said.
    pub(crate) created_at: i64,
    /// The message this one replies to, if any.
    pub(crate) reply_to_id: Option<i64>,
    /// The room.
    #[sea_orm(belongs_to, from = "room_id", to = "id")]
    pub(crate) room: HasOne<super::room::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
