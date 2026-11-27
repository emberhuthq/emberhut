//! Where to push, and the one-time secret links.
//!
//! - `push_devices`: every iPhone's APNs device token, which of Apple's services it is for
//!   (`sandbox` for a development build, `production` for a release), and the signed-in device
//!   that registered it.
//! - `fcm_devices`: every Android phone's FCM registration token, and the signed-in device that
//!   registered it.
//! - `secret_links`: the one-time pages for typing secrets in, by their token's SHA-256, with
//!   the names of the secrets each asks for (one per line) and why.
//!
//! A push token is the phone's own address, not a secret, so it is kept as sent and is its
//! row's key: a phone registering again updates its row. A push row belongs to its device: it
//! goes when the device is signed out (in the sign-out's transaction) or deleted (the foreign
//! key's cascade), so a signed-out phone is sent nothing more.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer, big_pk_auto, blob, string, text};

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
                    .table(PushDevices::Table)
                    .col(string(PushDevices::Token).primary_key())
                    .col(string(PushDevices::Env))
                    .col(big_integer(PushDevices::DeviceId))
                    .col(big_integer(PushDevices::RegisteredAt))
                    .foreign_key(
                        ForeignKey::create()
                            .from(PushDevices::Table, PushDevices::DeviceId)
                            .to(Devices::Table, Devices::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(FcmDevices::Table)
                    .col(string(FcmDevices::Token).primary_key())
                    .col(big_integer(FcmDevices::DeviceId))
                    .col(big_integer(FcmDevices::RegisteredAt))
                    .foreign_key(
                        ForeignKey::create()
                            .from(FcmDevices::Table, FcmDevices::DeviceId)
                            .to(Devices::Table, Devices::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(SecretLinks::Table)
                    .col(big_pk_auto(SecretLinks::Id))
                    .col(blob(SecretLinks::TokenHash).unique_key())
                    .col(text(SecretLinks::Names))
                    .col(text(SecretLinks::Why))
                    .col(big_integer(SecretLinks::CreatedAt))
                    .col(big_integer(SecretLinks::ExpiresAt))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            SecretLinks::Table.into_iden(),
            FcmDevices::Table.into_iden(),
            PushDevices::Table.into_iden(),
        ] {
            manager
                .drop_table(Table::drop().table(table).to_owned())
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Devices {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum PushDevices {
    Table,
    Token,
    Env,
    DeviceId,
    RegisteredAt,
}

#[derive(DeriveIden)]
enum FcmDevices {
    Table,
    Token,
    DeviceId,
    RegisteredAt,
}

#[derive(DeriveIden)]
enum SecretLinks {
    Table,
    Id,
    TokenHash,
    Names,
    Why,
    CreatedAt,
    ExpiresAt,
}
