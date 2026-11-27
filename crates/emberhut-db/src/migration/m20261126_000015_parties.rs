//! Parties: `parties`, a few AI personas talking about a topic (their cast kept as JSON), and
//! `party_messages`, what was said in each, in order.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_pk_auto, boolean, double, integer, string, string_null, text, text_null,
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
                    .table(Parties::Table)
                    .col(big_pk_auto(Parties::Id))
                    .col(text(Parties::Title))
                    .col(text(Parties::Topic))
                    .col(string(Parties::Status))
                    .col(text_null(Parties::Why))
                    .col(text(Parties::Personas))
                    .col(integer(Parties::Turns).default(0))
                    .col(integer(Parties::MaxTurns))
                    .col(big_integer(Parties::Tokens).default(0))
                    .col(big_integer(Parties::MaxTokens))
                    .col(double(Parties::Cost).default(0.0))
                    .col(big_integer(Parties::Created))
                    .col(big_integer(Parties::Updated))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(PartyMessages::Table)
                    .col(big_pk_auto(PartyMessages::Id))
                    .col(big_integer(PartyMessages::Party))
                    .col(string(PartyMessages::Who))
                    .col(text(PartyMessages::Name))
                    .col(string(PartyMessages::Kind))
                    .col(text(PartyMessages::Text))
                    .col(string_null(PartyMessages::Model))
                    .col(big_integer(PartyMessages::Tokens).default(0))
                    .col(double(PartyMessages::Cost).default(0.0))
                    .col(boolean(PartyMessages::Cut).default(false))
                    .col(big_integer(PartyMessages::At))
                    .foreign_key(
                        ForeignKey::create()
                            .from(PartyMessages::Table, PartyMessages::Party)
                            .to(Parties::Table, Parties::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("party_messages_party")
                    .table(PartyMessages::Table)
                    .col(PartyMessages::Party)
                    .col(PartyMessages::Id)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [PartyMessages::Table.into_iden(), Parties::Table.into_iden()] {
            manager
                .drop_table(Table::drop().table(table).to_owned())
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Parties {
    Table,
    Id,
    Title,
    Topic,
    Status,
    Why,
    Personas,
    Turns,
    MaxTurns,
    Tokens,
    MaxTokens,
    Cost,
    Created,
    Updated,
}

#[derive(DeriveIden)]
enum PartyMessages {
    Table,
    Id,
    Party,
    Who,
    Name,
    Kind,
    Text,
    Model,
    Tokens,
    Cost,
    Cut,
    At,
}
