//! What the runners on other machines report, and what the server keeps for them:
//!
//! - `machine_latest`: each runner machine's latest sample of itself (one row per machine, the
//!   sample as JSON), so a restart still shows the last one;
//! - `machine_points`: a day of 5-minute points per machine (each the worst of its samples);
//! - `usage`: each assistant account's use as its runner last reported it (one row per machine
//!   and account id);
//! - `prefs`: the person's small account choices, one text value per key (the pinned account);
//! - `pulls`: the files the server already brought from a runner's machine, so the same file,
//!   unchanged, isn't fetched twice.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_integer_null, boolean, double_null, string, string_null, text, text_null,
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
                    .table(MachineLatest::Table)
                    .col(string(MachineLatest::Machine).primary_key())
                    .col(big_integer(MachineLatest::At))
                    .col(text(MachineLatest::Data))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(MachinePoints::Table)
                    .col(string(MachinePoints::Machine))
                    .col(big_integer(MachinePoints::Bucket))
                    .col(text(MachinePoints::Data))
                    .primary_key(
                        Index::create()
                            .col(MachinePoints::Machine)
                            .col(MachinePoints::Bucket),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Usage::Table)
                    .col(string(Usage::Machine))
                    .col(string(Usage::Id))
                    .col(text(Usage::Name))
                    .col(string(Usage::Scope))
                    .col(boolean(Usage::Enabled))
                    .col(boolean(Usage::LoggedIn))
                    .col(big_integer_null(Usage::Rank))
                    .col(text_null(Usage::Email))
                    .col(string_null(Usage::Plan))
                    .col(double_null(Usage::Used5h))
                    .col(double_null(Usage::Used7d))
                    .col(big_integer_null(Usage::Resets5h))
                    .col(big_integer_null(Usage::Resets7d))
                    .col(big_integer_null(Usage::LimitedUntil))
                    .col(text_null(Usage::LimitedWhy))
                    .col(big_integer_null(Usage::Checked))
                    .col(text_null(Usage::Error))
                    .col(big_integer(Usage::At))
                    .primary_key(Index::create().col(Usage::Machine).col(Usage::Id))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Prefs::Table)
                    .col(string(Prefs::Key).primary_key())
                    .col(text(Prefs::Value))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Pulls::Table)
                    .col(string(Pulls::Machine))
                    .col(text(Pulls::Path))
                    .col(big_integer(Pulls::Size))
                    .col(string(Pulls::Mtime))
                    .col(text(Pulls::Local))
                    .col(big_integer(Pulls::At))
                    .primary_key(
                        Index::create()
                            .col(Pulls::Machine)
                            .col(Pulls::Path)
                            .col(Pulls::Size)
                            .col(Pulls::Mtime),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Pulls::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Prefs::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Usage::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(MachinePoints::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(MachineLatest::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum MachineLatest {
    Table,
    Machine,
    At,
    Data,
}

#[derive(DeriveIden)]
enum MachinePoints {
    Table,
    Machine,
    Bucket,
    Data,
}

#[derive(DeriveIden)]
enum Usage {
    Table,
    Machine,
    Id,
    Name,
    Scope,
    Enabled,
    LoggedIn,
    Rank,
    Email,
    Plan,
    Used5h,
    Used7d,
    Resets5h,
    Resets7d,
    LimitedUntil,
    LimitedWhy,
    Checked,
    Error,
    At,
}

#[derive(DeriveIden)]
enum Prefs {
    Table,
    Key,
    Value,
}

#[derive(DeriveIden)]
enum Pulls {
    Table,
    Machine,
    Path,
    Size,
    Mtime,
    Local,
    At,
}
