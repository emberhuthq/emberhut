//! `events`: every step of every session's transcript, in order.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A transcript step as stored; `data` holds the step's own fields as JSON.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "events")]
pub struct Model {
    /// Its position across all sessions, rising.
    #[sea_orm(primary_key)]
    pub seq: i64,
    /// Its session.
    #[sea_orm(indexed)]
    pub session: String,
    /// Its turn.
    pub turn: Option<String>,
    /// When it was written.
    pub at: Option<String>,
    /// Its kind (`user`, `text`, `tool`, `result`, `end`, `system`, `question`, `thinking`, `todos`).
    pub kind: Option<String>,
    /// Its fields, as JSON.
    pub data: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
