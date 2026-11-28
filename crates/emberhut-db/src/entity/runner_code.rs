//! `runner_codes`: one-time codes a new runner pairs with. Only their hashes.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A runner pairing code that was made.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "runner_codes")]
pub(crate) struct Model {
    /// The code's id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// The hash of the code, as typed without its dash.
    #[sea_orm(indexed)]
    pub(crate) code_hash: Vec<u8>,
    /// When it was made.
    pub(crate) created_at: i64,
    /// When it stops working.
    pub(crate) expires_at: i64,
    /// When it was used; `None` while it has not been.
    pub(crate) used_at: Option<i64>,
    /// The runner it paired.
    pub(crate) runner: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
