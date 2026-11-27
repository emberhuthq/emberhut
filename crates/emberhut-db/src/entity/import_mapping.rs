//! `import_mappings`: which mappings have run.

#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model derive writes async trait methods without an await for a table with no relations"
)]

use sea_orm::entity::prelude::*;

/// One mapping that ran.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "import_mappings")]
pub(crate) struct Model {
    /// Its name.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) name: String,
    /// Its version.
    pub(crate) version: i32,
    /// The source table it maps from.
    pub(crate) source_tbl: String,
    /// The rows it wrote.
    pub(crate) rows: i64,
    /// When it ran.
    pub(crate) ran_at: i64,
}

impl ActiveModelBehavior for ActiveModel {}
