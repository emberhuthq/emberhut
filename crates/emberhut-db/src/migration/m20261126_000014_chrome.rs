//! `chrome_devices`: the owner's Chrome profiles paired with the server through the browser
//! extension, each with its own token (stored hashed); and `chrome_tasks`: the browser a task
//! works in (a profile, or 0 for the server's own browser).

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer, big_integer_null, pk_auto, string, string_null};

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
                    .table(ChromeDevices::Table)
                    .col(pk_auto(ChromeDevices::Id))
                    .col(string(ChromeDevices::Name))
                    .col(string(ChromeDevices::Token))
                    .col(big_integer(ChromeDevices::CreatedAt))
                    .col(big_integer_null(ChromeDevices::SeenAt))
                    .col(big_integer_null(ChromeDevices::RevokedAt))
                    .col(string_null(ChromeDevices::Label))
                    .col(string_null(ChromeDevices::Email))
                    .col(string_null(ChromeDevices::Machine))
                    .col(string_null(ChromeDevices::Os))
                    .col(string_null(ChromeDevices::Version))
                    .col(string_null(ChromeDevices::Agent))
                    .col(string_null(ChromeDevices::Ext))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(ChromeTasks::Table)
                    .col(big_integer(ChromeTasks::Task).primary_key())
                    .col(big_integer(ChromeTasks::Device))
                    .col(big_integer(ChromeTasks::At))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ChromeTasks::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(ChromeDevices::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum ChromeDevices {
    Table,
    Id,
    Name,
    Token,
    CreatedAt,
    SeenAt,
    RevokedAt,
    Label,
    Email,
    Machine,
    Os,
    Version,
    Agent,
    Ext,
}

#[derive(DeriveIden)]
enum ChromeTasks {
    Table,
    Task,
    Device,
    At,
}
