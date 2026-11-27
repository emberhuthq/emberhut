//! `secret_links`: one-time pages for typing secrets in. Only their tokens' hashes.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "sea-orm's model macro writes async methods with nothing to await for an entity \
              without relations"
)]

use sea_orm::entity::prelude::*;

/// A secret link that was made and not used yet.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "secret_links")]
pub(crate) struct Model {
    /// The link's id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// The hash of the link's token.
    #[sea_orm(unique)]
    pub(crate) token_hash: Vec<u8>,
    /// The names of the secrets it asks for, one per line (a name holds no line break).
    pub(crate) names: String,
    /// Why they are wanted.
    pub(crate) why: String,
    /// When it was made.
    pub(crate) created_at: i64,
    /// When it stops working.
    pub(crate) expires_at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
