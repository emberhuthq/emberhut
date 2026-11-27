//! `pulls`: the files already brought from a runner's machine.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A file brought over.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "pulls")]
pub(crate) struct Model {
    /// The machine it came from.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) machine: String,
    /// Its path there.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) path: String,
    /// Its size, in bytes.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) size: i64,
    /// When it last changed there, as the runner said it.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) mtime: String,
    /// Where it is here, relative to the uploads folder.
    pub(crate) local: String,
    /// When it was brought.
    pub(crate) at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
