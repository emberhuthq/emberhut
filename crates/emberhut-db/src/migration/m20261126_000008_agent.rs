//! The assistant's answers' chips: `message_refs` keeps, per answer, what it is about (its
//! pull requests, tickets and reports) as the JSON list the chat shows under it.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer, text};

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
                    .table(MessageRefs::Table)
                    .col(big_integer(MessageRefs::MessageId).primary_key())
                    .col(text(MessageRefs::Refs))
                    .foreign_key(
                        ForeignKey::create()
                            .from(MessageRefs::Table, MessageRefs::MessageId)
                            .to(Messages::Table, Messages::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(MessageRefs::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum MessageRefs {
    Table,
    MessageId,
    Refs,
}

#[derive(DeriveIden)]
enum Messages {
    Table,
    Id,
}
