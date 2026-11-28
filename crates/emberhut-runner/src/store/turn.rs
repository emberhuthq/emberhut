//! `turns`: one message to a session and its answer.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A turn as stored.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "turns")]
pub struct Model {
    /// Its id: the session's id, `:` and its number.
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// Its session.
    pub session: String,
    /// Its number in the session.
    pub n: i64,
    /// What it asks.
    pub prompt: Option<String>,
    /// Who asked (`owner`, `assistant`, `self`, `runner`).
    pub by: Option<String>,
    /// When it was asked for.
    pub requested: Option<String>,
    /// When it started.
    pub started: Option<String>,
    /// When it ended.
    pub ended: Option<String>,
    /// How it ended (`completed`, `failed`, `interrupted`, `cancelled`).
    pub outcome: Option<String>,
    /// Its answer.
    pub last: Option<String>,
    /// The accounts it ran on, in order, as a JSON list.
    pub accounts: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
