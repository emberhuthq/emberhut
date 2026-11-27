//! What the assistant remembers: `notes` (the owner's quick notes and the assistant's diary),
//! `corrections` (what it was corrected on, and each lesson's trial), and recall's index:
//! `docs` (the pieces of text), `docs_fts` (their full-text index), `vecs` (their embedding
//! vectors), `marks` (how far each source has been indexed) and `transcript_offsets` (how far
//! each agent transcript has been read).
//!
//! `docs_fts` is an SQLite FTS5 virtual table, kept in step with `docs` by triggers, which
//! also drop a piece's vector when it changes or goes. Neither can be made with the schema
//! builder, so they are the statements below, as ADR 0016 allows; everything else is built.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_integer_null, big_pk_auto, blob, string, string_null, text, text_null,
};

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

/// The full-text index over the pieces' titles and texts, and the triggers that keep it (and
/// the vectors) in step with `docs`.
const FULL_TEXT: &[&str] = &[
    "CREATE VIRTUAL TABLE docs_fts USING fts5(title, text, content='docs', content_rowid='id', \
     tokenize='porter unicode61 remove_diacritics 2')",
    "CREATE TRIGGER docs_ai AFTER INSERT ON docs BEGIN \
     INSERT INTO docs_fts(rowid, title, text) VALUES (new.id, new.title, new.text); END",
    "CREATE TRIGGER docs_ad AFTER DELETE ON docs BEGIN \
     INSERT INTO docs_fts(docs_fts, rowid, title, text) VALUES ('delete', old.id, old.title, old.text); \
     DELETE FROM vecs WHERE id = old.id; END",
    "CREATE TRIGGER docs_au AFTER UPDATE OF title, text ON docs BEGIN \
     INSERT INTO docs_fts(docs_fts, rowid, title, text) VALUES ('delete', old.id, old.title, old.text); \
     INSERT INTO docs_fts(rowid, title, text) VALUES (new.id, new.title, new.text); \
     DELETE FROM vecs WHERE id = old.id; END",
];

/// What [`FULL_TEXT`] made, undone.
const FULL_TEXT_DOWN: &[&str] = &[
    "DROP TRIGGER IF EXISTS docs_au",
    "DROP TRIGGER IF EXISTS docs_ad",
    "DROP TRIGGER IF EXISTS docs_ai",
    "DROP TABLE IF EXISTS docs_fts",
];

#[async_trait]
#[expect(
    elided_lifetimes_in_paths,
    reason = "async_trait matches the trait's signature, which elides SchemaManager's lifetime"
)]
impl MigrationTrait for Migration {
    #[expect(
        clippy::too_many_lines,
        reason = "one table after another, each read on its own"
    )]
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Notes::Table)
                    .col(big_pk_auto(Notes::Id))
                    .col(string(Notes::Uid).unique_key())
                    .col(string(Notes::Who))
                    .col(string(Notes::Day))
                    .col(big_integer(Notes::At))
                    .col(text(Notes::Text))
                    .col(big_integer(Notes::Created))
                    .col(big_integer(Notes::Updated))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("notes_day")
                    .table(Notes::Table)
                    .col(Notes::Day)
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Corrections::Table)
                    .col(big_pk_auto(Corrections::Id))
                    .col(big_integer(Corrections::At))
                    .col(text(Corrections::Said))
                    .col(text(Corrections::Wrong))
                    .col(text_null(Corrections::Lesson))
                    .col(string(Corrections::Source))
                    .col(string_null(Corrections::Trial))
                    .col(string_null(Corrections::TrialBy))
                    .col(big_integer_null(Corrections::TrialAt))
                    .col(big_integer_null(Corrections::Rule))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("corrections_at")
                    .table(Corrections::Table)
                    .col(Corrections::At)
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Docs::Table)
                    .col(big_pk_auto(Docs::Id))
                    .col(string(Docs::Source))
                    .col(text(Docs::Reference))
                    .col(big_integer_null(Docs::At))
                    .col(text(Docs::Title))
                    .col(text(Docs::Text))
                    .col(text(Docs::Place))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("docs_source_reference")
                    .table(Docs::Table)
                    .col(Docs::Source)
                    .col(Docs::Reference)
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("docs_at")
                    .table(Docs::Table)
                    .col(Docs::At)
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Vecs::Table)
                    .col(big_integer(Vecs::Id).primary_key())
                    .col(blob(Vecs::V))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Marks::Table)
                    .col(string(Marks::Key).primary_key())
                    .col(text(Marks::Value))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(TranscriptOffsets::Table)
                    .col(text(TranscriptOffsets::Path).primary_key())
                    .col(big_integer(TranscriptOffsets::Offset))
                    .col(big_integer(TranscriptOffsets::Line))
                    .to_owned(),
            )
            .await?;
        let conn = manager.get_connection();
        for statement in FULL_TEXT {
            conn.execute_unprepared(statement).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        for statement in FULL_TEXT_DOWN {
            conn.execute_unprepared(statement).await?;
        }
        for table in [
            TranscriptOffsets::Table.into_iden(),
            Marks::Table.into_iden(),
            Vecs::Table.into_iden(),
            Docs::Table.into_iden(),
            Corrections::Table.into_iden(),
            Notes::Table.into_iden(),
        ] {
            manager
                .drop_table(Table::drop().table(table).to_owned())
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Notes {
    Table,
    Id,
    Uid,
    Who,
    Day,
    At,
    Text,
    Created,
    Updated,
}

#[derive(DeriveIden)]
enum Corrections {
    Table,
    Id,
    At,
    Said,
    Wrong,
    Lesson,
    Source,
    Trial,
    TrialBy,
    TrialAt,
    Rule,
}

#[derive(DeriveIden)]
enum Docs {
    Table,
    Id,
    Source,
    Reference,
    At,
    Title,
    Text,
    Place,
}

#[derive(DeriveIden)]
enum Vecs {
    Table,
    Id,
    V,
}

#[derive(DeriveIden)]
enum Marks {
    Table,
    Key,
    Value,
}

#[derive(DeriveIden)]
enum TranscriptOffsets {
    Table,
    Path,
    Offset,
    Line,
}
