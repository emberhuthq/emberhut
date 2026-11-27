//! `dark_messages`: the off-the-record room's chat. This table lives in a database of its own
//! (`dark.db` beside the server's), made by [`super::DarkMigrator`], never by the server's
//! migrator: the room's file is left out of backups and no other table knows it.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer, big_pk_auto, string, text};

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
                    .table(DarkMessages::Table)
                    .col(big_pk_auto(DarkMessages::Id))
                    .col(string(DarkMessages::Role))
                    .col(text(DarkMessages::Text))
                    .col(big_integer(DarkMessages::At))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(DarkMessages::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum DarkMessages {
    Table,
    Id,
    Role,
    Text,
    At,
}
