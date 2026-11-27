//! `import_runs`: one row per import.

#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model derive writes async trait methods without an await for a table with no relations"
)]

use sea_orm::entity::prelude::*;

/// One import.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "import_runs")]
pub(crate) struct Model {
    /// Its number.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// When it ran.
    pub(crate) ran_at: i64,
    /// The importing program's version.
    pub(crate) version: String,
    /// The kind of input.
    pub(crate) input: String,
    /// Per source database: its bytes, SHA-256 and `user_version`, as JSON.
    pub(crate) sources: String,
    /// The report, as printed.
    pub(crate) report: String,
}

impl ActiveModelBehavior for ActiveModel {}
