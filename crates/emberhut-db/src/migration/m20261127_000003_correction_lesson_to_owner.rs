//! `corrections.lesson_to_owner`: a correction's lesson as one short line to the owner, in the
//! second person, which the app shows beside what the owner said. The lesson itself stays as
//! the assistant's rule for its prompt. NULL for rows made before it was recorded, and when the
//! assistant gives none.
//!
//! It changes no existing column.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::text_null;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

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
                    .add_column(text_null(Corrections::LessonToOwner))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Corrections::Table)
                    .drop_column(Corrections::LessonToOwner)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Corrections {
    Table,
    LessonToOwner,
}
