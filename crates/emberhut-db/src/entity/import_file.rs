//! `import_files`: every file an import copied.

#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model derive writes async trait methods without an await for a table with no relations"
)]

use sea_orm::entity::prelude::*;

/// One copied file.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "import_files")]
pub(crate) struct Model {
    /// Its tree (`uploads`, `context`, ...).
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) tree: String,
    /// Its path in the tree, as the source wrote it.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) path: String,
    /// Its size.
    pub(crate) bytes: i64,
    /// Its SHA-256, in lower-case hex.
    pub(crate) sha256: String,
}

impl ActiveModelBehavior for ActiveModel {}
