//! The first schema: accounts, their machines, the accepted statements' signatures and each
//! machine's latest statement time.
//!
//! Instants are whole milliseconds since the Unix epoch (`BIGINT`); keys and signatures are
//! their raw bytes (`BLOB`).

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer, big_integer_null, blob, string, string_null};

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
                    .table(Accounts::Table)
                    .col(string(Accounts::UserId).primary_key())
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Machines::Table)
                    .col(string(Machines::Id).primary_key())
                    .col(blob(Machines::PublicKey))
                    .col(string(Machines::Name))
                    .col(string(Machines::Account))
                    .col(big_integer(Machines::LinkedAt))
                    .col(big_integer_null(Machines::LastSeen))
                    .col(string_null(Machines::HeldBy))
                    .foreign_key(
                        ForeignKey::create()
                            .from(Machines::Table, Machines::Account)
                            .to(Accounts::Table, Accounts::UserId)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("machines_account")
                    .table(Machines::Table)
                    .col(Machines::Account)
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(AcceptedSignatures::Table)
                    .col(blob(AcceptedSignatures::Signature).primary_key())
                    .col(big_integer(AcceptedSignatures::ExpiresAt))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("accepted_signatures_expires_at")
                    .table(AcceptedSignatures::Table)
                    .col(AcceptedSignatures::ExpiresAt)
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(LatestStatements::Table)
                    .col(string(LatestStatements::Machine).primary_key())
                    .col(big_integer(LatestStatements::Time))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("latest_statements_time")
                    .table(LatestStatements::Table)
                    .col(LatestStatements::Time)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            LatestStatements::Table.into_iden(),
            AcceptedSignatures::Table.into_iden(),
            Machines::Table.into_iden(),
            Accounts::Table.into_iden(),
        ] {
            manager
                .drop_table(Table::drop().table(table).to_owned())
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Accounts {
    Table,
    UserId,
}

#[derive(DeriveIden)]
enum Machines {
    Table,
    Id,
    PublicKey,
    Name,
    Account,
    LinkedAt,
    LastSeen,
    HeldBy,
}

#[derive(DeriveIden)]
enum AcceptedSignatures {
    Table,
    Signature,
    ExpiresAt,
}

#[derive(DeriveIden)]
enum LatestStatements {
    Table,
    Machine,
    Time,
}
