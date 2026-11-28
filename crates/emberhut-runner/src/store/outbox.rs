//! `outbox`: events for the server it hasn't taken yet, oldest first.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// An event waiting for the server.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "outbox")]
pub struct Model {
    /// Its place in the queue.
    #[sea_orm(primary_key)]
    pub id: i64,
    /// Its session.
    pub session: Option<String>,
    /// Its kind.
    pub kind: Option<String>,
    /// The event's body, as JSON.
    pub body: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
