//! `message_files`: which files a message came with, in the order they were attached.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes async methods that need no await for an entity without relations"
)]

use sea_orm::entity::prelude::*;

/// One file of a message.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "message_files")]
pub(crate) struct Model {
    /// The message.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) message_id: i64,
    /// Where the file is among the message's files, from 0.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) position: i32,
    /// The file.
    pub(crate) file_id: i64,
}

impl ActiveModelBehavior for ActiveModel {}
