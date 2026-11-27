//! Coding threads: `threads`, the latest look at every coding agent's thread on every machine
//! (one row per machine and thread id), and `followed_threads`, the ones the assistant follows.
//!
//! A followed thread is kept by its id alone, with no foreign key to `threads`: the assistant
//! follows a thread it has just started, before any look at it has been stored.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_integer_null, boolean, string, string_null, text, text_null,
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
                    .table(Threads::Table)
                    .col(string(Threads::Machine))
                    .col(string(Threads::Id))
                    .col(text(Threads::Title))
                    .col(text(Threads::Project))
                    .col(string(Threads::Status))
                    .col(big_integer(Threads::UpdatedAt))
                    .col(string_null(Threads::Parent))
                    .col(text_null(Threads::Ask))
                    .col(text_null(Threads::LastAsk))
                    .col(text_null(Threads::Last))
                    .col(text_null(Threads::StatusLine))
                    .col(text_null(Threads::Doing))
                    .col(text_null(Threads::Account))
                    .col(text_null(Threads::Cwd))
                    .col(text_null(Threads::Branch))
                    .col(big_integer_null(Threads::Since))
                    .col(boolean(Threads::Direct).default(false))
                    .col(text_null(Threads::Model))
                    .col(text_null(Threads::Effort))
                    .col(text_null(Threads::Question))
                    .primary_key(Index::create().col(Threads::Machine).col(Threads::Id))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("threads_updated_at")
                    .table(Threads::Table)
                    .col(Threads::UpdatedAt)
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(FollowedThreads::Table)
                    .col(string(FollowedThreads::Thread).primary_key())
                    .col(string(FollowedThreads::Machine))
                    .col(text(FollowedThreads::Why))
                    .col(big_integer(FollowedThreads::At))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            FollowedThreads::Table.into_iden(),
            Threads::Table.into_iden(),
        ] {
            manager
                .drop_table(Table::drop().table(table).to_owned())
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Threads {
    Table,
    Machine,
    Id,
    Title,
    Project,
    Status,
    UpdatedAt,
    Parent,
    Ask,
    LastAsk,
    Last,
    StatusLine,
    Doing,
    Account,
    Cwd,
    Branch,
    Since,
    Direct,
    Model,
    Effort,
    Question,
}

#[derive(DeriveIden)]
enum FollowedThreads {
    Table,
    Thread,
    Machine,
    Why,
    At,
}
