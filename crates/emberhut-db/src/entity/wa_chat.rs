//! `wa_chats`: the chats the assistant's own WhatsApp number knows, and the allow-list.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A chat.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "wa_chats")]
pub(crate) struct Model {
    /// The group's or the person's id.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) jid: String,
    /// `group` or `dm`.
    pub(crate) kind: String,
    /// Its name.
    pub(crate) name: Option<String>,
    /// `pending`, `allowed`, `denied` or `left`.
    pub(crate) status: String,
    /// The id a DM's messages come from.
    pub(crate) via: Option<String>,
    /// The task the owner was asked on.
    pub(crate) task: Option<i64>,
    /// The room its asks go to.
    pub(crate) room: Option<i64>,
    /// What the owner allowed there.
    pub(crate) topics: Option<String>,
    /// Who added the number.
    pub(crate) by: Option<String>,
    /// When it was first seen.
    pub(crate) added: i64,
    /// When it last changed.
    pub(crate) updated: i64,
}

impl ActiveModelBehavior for ActiveModel {}
