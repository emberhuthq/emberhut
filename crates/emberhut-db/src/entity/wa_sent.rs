//! `wa_sent`: every message the assistant's own WhatsApp number sent.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A sent message.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "wa_sent")]
pub(crate) struct Model {
    /// Its row's id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// The chat's id.
    pub(crate) chat: String,
    /// The chat's name then.
    pub(crate) name: Option<String>,
    /// When it was sent.
    pub(crate) at: i64,
    /// What it said.
    pub(crate) text: String,
    /// WhatsApp's id of it.
    pub(crate) msg: Option<String>,
    /// The message it answered.
    pub(crate) reply_to: Option<String>,
    /// The room it was shown in.
    pub(crate) room: Option<i64>,
}

impl ActiveModelBehavior for ActiveModel {}
