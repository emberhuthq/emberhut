//! `wa_msgs`: the allowed chats' recent messages, for context.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A message.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "wa_msgs")]
pub(crate) struct Model {
    /// The chat's id.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) chat: String,
    /// WhatsApp's id of it.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) id: String,
    /// The sender's id.
    pub(crate) sender: Option<String>,
    /// How the sender is named.
    pub(crate) who: Option<String>,
    /// What it says.
    pub(crate) text: Option<String>,
    /// When it was sent.
    pub(crate) at: i64,
    /// The assistant sent it.
    pub(crate) from_me: bool,
    /// It addressed the assistant.
    pub(crate) asked: bool,
    /// The assistant replied to it.
    pub(crate) answered: bool,
}

impl ActiveModelBehavior for ActiveModel {}
