//! `message_refs`: an answer's chips, as the JSON list the chat shows.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes async methods that need no await for an entity without relations"
)]

use sea_orm::entity::prelude::*;

/// One answer's chips.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "message_refs")]
pub(crate) struct Model {
    /// The answer.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) message_id: i64,
    /// Its chips, as JSON.
    pub(crate) refs: String,
}

impl ActiveModelBehavior for ActiveModel {}
