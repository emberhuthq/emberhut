//! `latest_builds`: the newest build of the app the release step reported, one row per
//! platform (`ios`, `mac`), for the state's `builds`.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer, string};

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
                    .table(LatestBuilds::Table)
                    .col(string(LatestBuilds::Platform).primary_key())
                    .col(big_integer(LatestBuilds::Number))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(LatestBuilds::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum LatestBuilds {
    Table,
    Platform,
    Number,
}
