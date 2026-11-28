//! Memory for shipping (ADR 0034):
//! - `corrections.triaged`: what the nightly review made of a candidate the server noticed
//!   (`noise`, or `correction` once it has a lesson); NULL until it is reviewed, and for every
//!   correction the assistant logged itself;
//! - FTS5's `secure-delete` on `docs_fts`, so a forgotten piece's words are overwritten in the
//!   full-text index rather than left in its pages. (Every connection also turns on SQLite's
//!   `secure_delete`, for the tables' own pages.)
//!
//! The full-text option is set with FTS5's own command, the one SQL here (ADR 0016's exception
//! for FTS5). It changes no existing column.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::text_null;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

/// FTS5's command that turns its secure delete on for the table (it is kept in the table).
const SECURE_DELETE_ON: &str = "INSERT INTO docs_fts(docs_fts, rank) VALUES('secure-delete', 1)";
const SECURE_DELETE_OFF: &str = "INSERT INTO docs_fts(docs_fts, rank) VALUES('secure-delete', 0)";

#[async_trait]
#[expect(
    elided_lifetimes_in_paths,
    reason = "async_trait matches the trait's signature, which elides SchemaManager's lifetime"
)]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Corrections::Table)
                    .add_column(text_null(Corrections::Triaged))
                    .to_owned(),
            )
            .await?;
        manager
            .get_connection()
            .execute_unprepared(SECURE_DELETE_ON)
            .await
            .map(drop)
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(SECURE_DELETE_OFF)
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Corrections::Table)
                    .drop_column(Corrections::Triaged)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Corrections {
    Table,
    Triaged,
}
