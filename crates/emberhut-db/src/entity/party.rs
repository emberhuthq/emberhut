//! `parties`: AI personas talking about a topic, with their caps and what they used.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A party.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "parties")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its title.
    pub(crate) title: String,
    /// Its topic.
    pub(crate) topic: String,
    /// `running`, `paused`, `done` or `stopped`.
    pub(crate) status: String,
    /// Why it paused or ended.
    pub(crate) why: Option<String>,
    /// Its cast, a JSON array of `{ id, name, role, model, color }`.
    pub(crate) personas: String,
    /// Turns so far.
    pub(crate) turns: i32,
    /// The turn cap.
    pub(crate) max_turns: i32,
    /// Tokens so far.
    pub(crate) tokens: i64,
    /// The token cap.
    pub(crate) max_tokens: i64,
    /// USD so far.
    pub(crate) cost: f64,
    /// When it was made.
    pub(crate) created: i64,
    /// When it last changed.
    pub(crate) updated: i64,
}

impl ActiveModelBehavior for ActiveModel {}
