//! Family members' own rooms and phones: `family_rooms` names the person each of their rooms
//! belongs to, and `family_devices` keeps their phones' push addresses apart from the owner's.

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
                    .table(FamilyRooms::Table)
                    .col(big_integer(FamilyRooms::RoomId).primary_key())
                    .col(string(FamilyRooms::Person))
                    .foreign_key(
                        ForeignKey::create()
                            .from(FamilyRooms::Table, FamilyRooms::RoomId)
                            .to(Rooms::Table, Rooms::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("family_rooms_person")
                    .table(FamilyRooms::Table)
                    .col(FamilyRooms::Person)
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(FamilyDevices::Table)
                    .col(string(FamilyDevices::Token).primary_key())
                    .col(string(FamilyDevices::Env))
                    .col(string(FamilyDevices::Person))
                    .col(big_integer(FamilyDevices::RegisteredAt))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            FamilyDevices::Table.into_iden(),
            FamilyRooms::Table.into_iden(),
        ] {
            manager
                .drop_table(Table::drop().table(table).to_owned())
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum FamilyRooms {
    Table,
    RoomId,
    Person,
}

#[derive(DeriveIden)]
enum FamilyDevices {
    Table,
    Token,
    Env,
    Person,
    RegisteredAt,
}

#[derive(DeriveIden)]
enum Rooms {
    Table,
    Id,
}
