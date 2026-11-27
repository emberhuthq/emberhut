//! `import_tables`: what happened to each source table.

#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model derive writes async trait methods without an await for a table with no relations"
)]

use sea_orm::entity::prelude::*;

/// One source table.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "import_tables")]
pub(crate) struct Model {
    /// The source database.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) source: String,
    /// The table.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) tbl: String,
    /// Rows read.
    pub(crate) rows_read: i64,
    /// Rows archived.
    pub(crate) rows_archived: i64,
    /// Rows mapped into typed tables.
    pub(crate) rows_mapped: i64,
    /// Rows dropped.
    pub(crate) rows_dropped: i64,
    /// The archived rows' checksum.
    pub(crate) checksum: Option<String>,
    /// The scrubbed columns, as a JSON array.
    pub(crate) scrubbed: String,
    /// Why it was dropped, or what else the report says of it.
    pub(crate) note: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
