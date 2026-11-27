//! `docs`: recall's pieces of text. `docs_fts` indexes their titles and texts (kept in
//! step by triggers) and `vecs` holds their embedding vectors.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A piece.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "docs")]
pub(crate) struct Model {
    /// Its id (`docs_fts`'s rowid, `vecs`' id).
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its kind.
    pub(crate) source: String,
    /// Unique within its kind.
    pub(crate) reference: String,
    /// When it was said.
    pub(crate) at: Option<i64>,
    /// What it is.
    pub(crate) title: String,
    /// What it says.
    pub(crate) text: String,
    /// How to open it.
    pub(crate) place: String,
}

impl ActiveModelBehavior for ActiveModel {}
