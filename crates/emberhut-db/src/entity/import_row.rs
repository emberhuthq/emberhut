//! `import_rows`: the archive of an imported installation's rows.

#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model derive writes async trait methods without an await for a table with no relations"
)]

use sea_orm::entity::prelude::*;

/// One archived row.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "import_rows")]
pub(crate) struct Model {
    /// The source database: the main, recall or runner database's name, as the importer spells
    /// it.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) source: String,
    /// The source table.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) tbl: String,
    /// The row's place in its table's rowid order.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) ordinal: i64,
    /// The rowid it was read with.
    pub(crate) rowid: i64,
    /// The declared primary key, as text (a JSON array for a composite key).
    pub(crate) key: Option<String>,
    /// The row, as JSON that keeps each value's storage class.
    pub(crate) row: String,
}

impl ActiveModelBehavior for ActiveModel {}
