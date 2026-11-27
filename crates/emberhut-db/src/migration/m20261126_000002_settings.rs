//! Settings, the activity log and events.
//!
//! - `settings`: one JSON value per key, for the small choices every area keeps (the theme,
//!   what the app hides, ...).
//! - `log`: what the assistant and the server did on their own, newest last, for the app's
//!   activity list.
//! - `events`: what programs and the server tell the assistant, per room, each read once
//!   (`handled`); a `key` makes an event happen at most once.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer, big_pk_auto, boolean, string, string_null, text};

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
                    .table(Settings::Table)
                    .col(string(Settings::Key).primary_key())
                    .col(text(Settings::Value))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Log::Table)
                    .col(big_pk_auto(Log::Id))
                    .col(big_integer(Log::At))
                    .col(text(Log::Text))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Events::Table)
                    .col(big_pk_auto(Events::Id))
                    .col(big_integer(Events::RoomId))
                    .col(string(Events::Kind))
                    .col(text(Events::Text))
                    .col(big_integer(Events::At))
                    .col(string_null(Events::Key).unique_key())
                    .col(boolean(Events::Handled).default(false))
                    .foreign_key(
                        ForeignKey::create()
                            .from(Events::Table, Events::RoomId)
                            .to(Rooms::Table, Rooms::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        // A room's waiting events are read, oldest first, every time the assistant wakes.
        manager
            .create_index(
                Index::create()
                    .name("events_handled_room_id")
                    .table(Events::Table)
                    .col(Events::Handled)
                    .col(Events::RoomId)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            Events::Table.into_iden(),
            Log::Table.into_iden(),
            Settings::Table.into_iden(),
        ] {
            manager
                .drop_table(Table::drop().table(table).to_owned())
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Settings {
    Table,
    Key,
    Value,
}

#[derive(DeriveIden)]
enum Log {
    Table,
    Id,
    At,
    Text,
}

#[derive(DeriveIden)]
enum Events {
    Table,
    Id,
    RoomId,
    Kind,
    Text,
    At,
    Key,
    Handled,
}

#[derive(DeriveIden)]
enum Rooms {
    Table,
    Id,
}
