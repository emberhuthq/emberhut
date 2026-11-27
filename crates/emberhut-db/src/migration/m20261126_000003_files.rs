//! The chat's files: `files`, one row per file stored in the uploads folder, and
//! `message_files`, which files each message came with, in order.
//!
//! A file's bytes are on disk, in the uploads folder beside the database; the row names it
//! (`stored_name`) and keeps what the chat shows of it. A file may be sent with more than one
//! message, so messages reach their files through `message_files`.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer, big_pk_auto, integer, string};

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
            .create_table(
                Table::create()
                    .table(Files::Table)
                    .col(big_pk_auto(Files::Id))
                    .col(string(Files::StoredName).unique_key())
                    .col(string(Files::Name))
                    .col(string(Files::Kind))
                    .col(big_integer(Files::Size))
                    .col(big_integer(Files::CreatedAt))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(MessageFiles::Table)
                    .col(big_integer(MessageFiles::MessageId))
                    .col(integer(MessageFiles::Position))
                    .col(big_integer(MessageFiles::FileId))
                    .primary_key(
                        Index::create()
                            .col(MessageFiles::MessageId)
                            .col(MessageFiles::Position),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(MessageFiles::Table, MessageFiles::MessageId)
                            .to(Messages::Table, Messages::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(MessageFiles::Table, MessageFiles::FileId)
                            .to(Files::Table, Files::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [MessageFiles::Table.into_iden(), Files::Table.into_iden()] {
            manager
                .drop_table(Table::drop().table(table).to_owned())
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Files {
    Table,
    Id,
    StoredName,
    Name,
    Kind,
    Size,
    CreatedAt,
}

#[derive(DeriveIden)]
enum MessageFiles {
    Table,
    MessageId,
    Position,
    FileId,
}

#[derive(DeriveIden)]
enum Messages {
    Table,
    Id,
}
