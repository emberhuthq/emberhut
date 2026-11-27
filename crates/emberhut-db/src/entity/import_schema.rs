//! `import_schema`: the source's `CREATE` text of every table, index and trigger.

#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model derive writes async trait methods without an await for a table with no relations"
)]

use sea_orm::entity::prelude::*;

/// One schema object.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "import_schema")]
pub(crate) struct Model {
    /// The source database.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) source: String,
    /// `table`, `index`, `trigger` or `view`.
    #[sea_orm(primary_key, auto_increment = false, column_name = "type")]
    pub(crate) kind: String,
    /// Its name.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) name: String,
    /// Its `CREATE` text (none for an automatic index).
    pub(crate) sql: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
