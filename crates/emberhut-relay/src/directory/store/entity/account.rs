//! `accounts`: the accounts that have machines linked, by their Clerk user id and nothing more.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// An account.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "accounts")]
pub(crate) struct Model {
    /// The account provider's user id (`sub`).
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) user_id: String,
}

impl ActiveModelBehavior for ActiveModel {}
