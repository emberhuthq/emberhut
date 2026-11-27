//! The first schema: devices and their sessions, pairing codes, rooms (with Main) and messages.
//!
//! Instants are whole milliseconds since the Unix epoch (`BIGINT`); secrets are kept only as
//! their SHA-256 (`BLOB`).

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_integer_null, big_pk_auto, blob, blob_null, string, string_null, text,
};

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait]
#[expect(
    elided_lifetimes_in_paths,
    reason = "async_trait matches the trait's signature, which elides SchemaManager's lifetime"
)]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        create_devices(manager).await?;
        create_sessions(manager).await?;
        create_pairing_codes(manager).await?;
        create_rooms(manager).await?;
        create_messages(manager).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            Messages::Table.into_iden(),
            Rooms::Table.into_iden(),
            PairingCodes::Table.into_iden(),
            Sessions::Table.into_iden(),
            Devices::Table.into_iden(),
        ] {
            manager
                .drop_table(Table::drop().table(table).to_owned())
                .await?;
        }
        Ok(())
    }
}

/// One row per app install that has signed in.
async fn create_devices(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(Devices::Table)
                .col(big_pk_auto(Devices::Id))
                .col(string(Devices::Name))
                .col(string(Devices::Hardware))
                .col(string_null(Devices::Install).unique_key())
                .col(big_integer(Devices::CreatedAt))
                .col(big_integer(Devices::SeenAt))
                .col(big_integer_null(Devices::RevokedAt))
                .to_owned(),
        )
        .await
}

/// A device's tokens, at most one row per device: the current pair, and the pair they replaced
/// (whose access token works until its own expiry, and whose refresh token is kept to spot its
/// reuse).
async fn create_sessions(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(Sessions::Table)
                .col(big_integer(Sessions::DeviceId).primary_key())
                .col(blob(Sessions::AccessHash).unique_key())
                .col(big_integer(Sessions::AccessExpiresAt))
                .col(blob(Sessions::RefreshHash).unique_key())
                .col(big_integer(Sessions::IssuedAt))
                .col(blob_null(Sessions::PreviousAccessHash).unique_key())
                .col(big_integer_null(Sessions::PreviousAccessExpiresAt))
                .col(blob_null(Sessions::PreviousRefreshHash).unique_key())
                .col(big_integer_null(Sessions::RotatedAt))
                .foreign_key(
                    ForeignKey::create()
                        .from(Sessions::Table, Sessions::DeviceId)
                        .to(Devices::Table, Devices::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await
}

/// One-time codes, looked up by their hash.
async fn create_pairing_codes(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(PairingCodes::Table)
                .col(big_pk_auto(PairingCodes::Id))
                .col(blob(PairingCodes::CodeHash))
                .col(big_integer(PairingCodes::CreatedAt))
                .col(big_integer(PairingCodes::ExpiresAt))
                .col(big_integer_null(PairingCodes::UsedAt))
                .col(big_integer_null(PairingCodes::DeviceId))
                .foreign_key(
                    ForeignKey::create()
                        .from(PairingCodes::Table, PairingCodes::DeviceId)
                        .to(Devices::Table, Devices::Id)
                        .on_delete(ForeignKeyAction::SetNull),
                )
                .to_owned(),
        )
        .await?;
    manager
        .create_index(
            Index::create()
                .name("pairing_codes_code_hash")
                .table(PairingCodes::Table)
                .col(PairingCodes::CodeHash)
                .to_owned(),
        )
        .await
}

/// Conversations, starting with Main (id 1).
async fn create_rooms(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(Rooms::Table)
                .col(big_pk_auto(Rooms::Id))
                .col(string(Rooms::Name))
                .col(big_integer(Rooms::CreatedAt))
                .col(big_integer_null(Rooms::SeenAt))
                .to_owned(),
        )
        .await?;
    let now = jiff::Timestamp::now().as_millisecond();
    manager
        .exec_stmt(
            Query::insert()
                .into_table(Rooms::Table)
                .columns([Rooms::Id, Rooms::Name, Rooms::CreatedAt])
                .values([1.into(), "Main".into(), now.into()])
                .map_err(|error| DbErr::Migration(error.to_string()))?
                .to_owned(),
        )
        .await
}

/// What was said in each room.
async fn create_messages(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(Messages::Table)
                .col(big_pk_auto(Messages::Id))
                .col(big_integer(Messages::RoomId))
                .col(string(Messages::Author))
                .col(text(Messages::Text))
                .col(big_integer(Messages::CreatedAt))
                .col(big_integer_null(Messages::ReplyToId))
                .foreign_key(
                    ForeignKey::create()
                        .from(Messages::Table, Messages::RoomId)
                        .to(Rooms::Table, Rooms::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(Messages::Table, Messages::ReplyToId)
                        .to(Messages::Table, Messages::Id)
                        .on_delete(ForeignKeyAction::SetNull),
                )
                .to_owned(),
        )
        .await?;
    // A room's page of history and its unread count read its newest messages first.
    manager
        .create_index(
            Index::create()
                .name("messages_room_id_id")
                .table(Messages::Table)
                .col(Messages::RoomId)
                .col(Messages::Id)
                .to_owned(),
        )
        .await
}

#[derive(DeriveIden)]
enum Devices {
    Table,
    Id,
    Name,
    Hardware,
    Install,
    CreatedAt,
    SeenAt,
    RevokedAt,
}

#[derive(DeriveIden)]
enum Sessions {
    Table,
    DeviceId,
    AccessHash,
    AccessExpiresAt,
    RefreshHash,
    IssuedAt,
    PreviousAccessHash,
    PreviousAccessExpiresAt,
    PreviousRefreshHash,
    RotatedAt,
}

#[derive(DeriveIden)]
enum PairingCodes {
    Table,
    Id,
    CodeHash,
    CreatedAt,
    ExpiresAt,
    UsedAt,
    DeviceId,
}

#[derive(DeriveIden)]
enum Rooms {
    Table,
    Id,
    Name,
    CreatedAt,
    SeenAt,
}

#[derive(DeriveIden)]
enum Messages {
    Table,
    Id,
    RoomId,
    Author,
    Text,
    CreatedAt,
    ReplyToId,
}
