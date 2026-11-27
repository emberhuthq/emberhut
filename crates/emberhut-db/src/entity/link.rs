//! `links`: the threads, tickets and chats that move a task.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A task's link.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "links")]
pub(crate) struct Model {
    /// Its task.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) issue: i64,
    /// `thread`, `ticket` or `chat`.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) kind: String,
    /// The thread's id, the ticket or the chat.
    #[sea_orm(primary_key, auto_increment = false, column_name = "ref")]
    pub(crate) reference: String,
    /// The machine a thread runs on.
    pub(crate) machine: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
