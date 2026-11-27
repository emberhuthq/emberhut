//! `mail`: the owner's mail, one row per message.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A mail message.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "mail")]
pub(crate) struct Model {
    /// `<account>:<Gmail's id>`.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) id: String,
    /// The account.
    pub(crate) account: String,
    /// The chat id.
    pub(crate) chat: String,
    /// The correspondent's name.
    pub(crate) who: String,
    /// Who wrote it.
    pub(crate) sender: String,
    /// The owner wrote it.
    pub(crate) from_me: bool,
    /// Its subject.
    pub(crate) subject: String,
    /// Its text.
    pub(crate) body: String,
    /// When it was sent or received.
    pub(crate) at: i64,
    /// When the server got it.
    pub(crate) got: i64,
    /// Worth waking for.
    pub(crate) loud: bool,
    /// Gmail's thread id.
    pub(crate) thread: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
