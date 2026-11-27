//! What an import of another installation's archive keeps besides the typed tables:
//!
//! - `import_rows`: every row of every source table that is kept, verbatim, as JSON that keeps
//!   each value's storage class; `(source, tbl, ordinal)` is the row's place in its table's
//!   rowid order, and `rowid` the rowid it was read with;
//! - the manifest: `import_runs` (one row per import), `import_schema` (the source's `CREATE`
//!   text of every table, index and trigger), `import_tables` (per source table: rows read,
//!   archived, mapped and dropped, the checksum and the scrubbed columns), `import_files` (every
//!   file copied, with its SHA-256) and `import_mappings` (which mappings have run).
//!
//! A database that was never imported into keeps these tables empty.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_pk_auto, integer, string, string_null, text, text_null,
};

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait]
#[expect(
    elided_lifetimes_in_paths,
    reason = "async_trait matches the trait's signature, which elides SchemaManager's lifetime"
)]
impl MigrationTrait for Migration {
    #[expect(
        clippy::too_many_lines,
        reason = "one table after another, as the schema reads"
    )]
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ImportRows::Table)
                    .col(string(ImportRows::Source))
                    .col(string(ImportRows::Tbl))
                    .col(big_integer(ImportRows::Ordinal))
                    .col(big_integer(ImportRows::Rowid))
                    .col(text_null(ImportRows::Key))
                    .col(text(ImportRows::Row))
                    .primary_key(
                        Index::create()
                            .col(ImportRows::Source)
                            .col(ImportRows::Tbl)
                            .col(ImportRows::Ordinal),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("import_rows_rowid")
                    .table(ImportRows::Table)
                    .col(ImportRows::Source)
                    .col(ImportRows::Tbl)
                    .col(ImportRows::Rowid)
                    .unique()
                    .to_owned(),
            )
            .await?;
        // Not unique: SQLite lets a primary key that is not an integer be NULL more than once.
        manager
            .create_index(
                Index::create()
                    .name("import_rows_key")
                    .table(ImportRows::Table)
                    .col(ImportRows::Source)
                    .col(ImportRows::Tbl)
                    .col(ImportRows::Key)
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(ImportRuns::Table)
                    .col(big_pk_auto(ImportRuns::Id))
                    .col(big_integer(ImportRuns::RanAt))
                    .col(string(ImportRuns::Version))
                    .col(string(ImportRuns::Input))
                    .col(text(ImportRuns::Sources))
                    .col(text(ImportRuns::Report))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(ImportSchema::Table)
                    .col(string(ImportSchema::Source))
                    .col(string(ImportSchema::Type))
                    .col(string(ImportSchema::Name))
                    .col(text_null(ImportSchema::Sql))
                    .primary_key(
                        Index::create()
                            .col(ImportSchema::Source)
                            .col(ImportSchema::Type)
                            .col(ImportSchema::Name),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(ImportTables::Table)
                    .col(string(ImportTables::Source))
                    .col(string(ImportTables::Tbl))
                    .col(big_integer(ImportTables::RowsRead))
                    .col(big_integer(ImportTables::RowsArchived))
                    .col(big_integer(ImportTables::RowsMapped))
                    .col(big_integer(ImportTables::RowsDropped))
                    .col(string_null(ImportTables::Checksum))
                    .col(text(ImportTables::Scrubbed))
                    .col(text_null(ImportTables::Note))
                    .primary_key(
                        Index::create()
                            .col(ImportTables::Source)
                            .col(ImportTables::Tbl),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(ImportFiles::Table)
                    .col(string(ImportFiles::Tree))
                    .col(text(ImportFiles::Path))
                    .col(big_integer(ImportFiles::Bytes))
                    .col(string(ImportFiles::Sha256))
                    .primary_key(
                        Index::create()
                            .col(ImportFiles::Tree)
                            .col(ImportFiles::Path),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(ImportMappings::Table)
                    .col(string(ImportMappings::Name).primary_key())
                    .col(integer(ImportMappings::Version))
                    .col(string(ImportMappings::SourceTbl))
                    .col(big_integer(ImportMappings::Rows))
                    .col(big_integer(ImportMappings::RanAt))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            ImportMappings::Table.into_iden(),
            ImportFiles::Table.into_iden(),
            ImportTables::Table.into_iden(),
            ImportSchema::Table.into_iden(),
            ImportRuns::Table.into_iden(),
            ImportRows::Table.into_iden(),
        ] {
            manager
                .drop_table(Table::drop().table(table).to_owned())
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum ImportRows {
    Table,
    Source,
    Tbl,
    Ordinal,
    Rowid,
    Key,
    Row,
}

#[derive(DeriveIden)]
enum ImportRuns {
    Table,
    Id,
    RanAt,
    Version,
    Input,
    Sources,
    Report,
}

#[derive(DeriveIden)]
enum ImportSchema {
    Table,
    Source,
    Type,
    Name,
    Sql,
}

#[derive(DeriveIden)]
enum ImportTables {
    Table,
    Source,
    Tbl,
    RowsRead,
    RowsArchived,
    RowsMapped,
    RowsDropped,
    Checksum,
    Scrubbed,
    Note,
}

#[derive(DeriveIden)]
enum ImportFiles {
    Table,
    Tree,
    Path,
    Bytes,
    Sha256,
}

#[derive(DeriveIden)]
enum ImportMappings {
    Table,
    Name,
    Version,
    SourceTbl,
    Rows,
    RanAt,
}
