//! `vecs`: a piece's embedding vector, normalised, as little-endian `f32`s.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A vector.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "vecs")]
pub(crate) struct Model {
    /// The piece's id.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) id: i64,
    /// The vector.
    pub(crate) v: Vec<u8>,
}

impl ActiveModelBehavior for ActiveModel {}
