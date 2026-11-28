//! Runners paired at run time, besides the ones the server's configuration names:
//!
//! - `paired_runners`: one row per runner, by its name: its API's address, its machine in words,
//!   its token, the system and version it said it runs, and when it first and last paired. The
//!   token is kept as text, not as a hash: the server presents it to the runner whenever it
//!   calls it, as it does a configured runner's.
//! - `runner_codes`: the one-time codes a runner pairs with, only their hashes, and the runner
//!   each one paired. They are device pairing codes' twins (`pairing_codes`), kept apart so a
//!   code made for one can never be traded for the other.
//!
//! It changes no existing table.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_integer_null, big_pk_auto, blob, string, text, text_null,
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
        manager
            .create_table(
                Table::create()
                    .table(PairedRunners::Table)
                    .col(string(PairedRunners::Name).primary_key())
                    .col(text(PairedRunners::Url))
                    .col(text_null(PairedRunners::Words))
                    .col(text(PairedRunners::Token))
                    .col(text_null(PairedRunners::Platform))
                    .col(text_null(PairedRunners::Version))
                    .col(big_integer(PairedRunners::CreatedAt))
                    .col(big_integer(PairedRunners::PairedAt))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(RunnerCodes::Table)
                    .col(big_pk_auto(RunnerCodes::Id))
                    .col(blob(RunnerCodes::CodeHash))
                    .col(big_integer(RunnerCodes::CreatedAt))
                    .col(big_integer(RunnerCodes::ExpiresAt))
                    .col(big_integer_null(RunnerCodes::UsedAt))
                    .col(text_null(RunnerCodes::Runner))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("runner_codes_code_hash")
                    .table(RunnerCodes::Table)
                    .col(RunnerCodes::CodeHash)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(RunnerCodes::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(PairedRunners::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum PairedRunners {
    Table,
    Name,
    Url,
    Words,
    Token,
    Platform,
    Version,
    CreatedAt,
    PairedAt,
}

#[derive(DeriveIden)]
enum RunnerCodes {
    Table,
    Id,
    CodeHash,
    CreatedAt,
    ExpiresAt,
    UsedAt,
    Runner,
}
