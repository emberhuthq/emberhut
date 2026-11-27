//! The workers and ultra runs:
//!
//! - `agents`: one row per sub-agent of a task's run: `run` is the session that started it (a
//!   worker's session, or a runner session), `key` the CLI's own id for it (a workflow's
//!   agent: `<task id>/<agent id>`; a lane: its session's id); its kind, part, state, last line,
//!   tokens and tool calls, where it runs, and when it started and ended.
//! - `streams`: how far each worker's stream (its turn file) has been read, kept across
//!   restarts so a stream is never read twice, with the lead's step and tokens so far.
//! - `issues.ultra_run`: an ultra task's run (its plan, state, caps and spend), as JSON;
//!   `issues.turn_file`: the stream file of its worker's current turn.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_integer_null, double_null, pk_auto, string, string_null, text, text_null,
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
                    .table(Agents::Table)
                    .col(pk_auto(Agents::Id))
                    .col(big_integer(Agents::Task))
                    .col(text(Agents::Run))
                    .col(text(Agents::Key))
                    .col(text_null(Agents::ToolUse))
                    .col(string_null(Agents::Source))
                    .col(string_null(Agents::Kind))
                    .col(text_null(Agents::Name))
                    .col(double_null(Agents::Part))
                    .col(string(Agents::State).default("running"))
                    .col(text_null(Agents::Last))
                    .col(big_integer_null(Agents::Tokens))
                    .col(big_integer_null(Agents::Tools))
                    .col(text_null(Agents::Account))
                    .col(text_null(Agents::Machine))
                    .col(big_integer_null(Agents::Started))
                    .col(big_integer_null(Agents::Ended))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("agents_run_key")
                    .table(Agents::Table)
                    .col(Agents::Run)
                    .col(Agents::Key)
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("agents_task")
                    .table(Agents::Table)
                    .col(Agents::Task)
                    .col(Agents::Id)
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Streams::Table)
                    .col(text(Streams::File).primary_key())
                    .col(big_integer(Streams::Offset))
                    .col(big_integer_null(Streams::Task))
                    .col(text_null(Streams::Doing))
                    .col(double_null(Streams::Tokens))
                    .to_owned(),
            )
            .await?;
        for column in [Issues::UltraRun, Issues::TurnFile] {
            manager
                .alter_table(
                    Table::alter()
                        .table(Issues::Table)
                        .add_column(text_null(column))
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in [Issues::TurnFile, Issues::UltraRun] {
            manager
                .alter_table(
                    Table::alter()
                        .table(Issues::Table)
                        .drop_column(column)
                        .to_owned(),
                )
                .await?;
        }
        manager
            .drop_table(Table::drop().table(Streams::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Agents::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Agents {
    Table,
    Id,
    Task,
    Run,
    Key,
    ToolUse,
    Source,
    Kind,
    Name,
    Part,
    State,
    Last,
    Tokens,
    Tools,
    Account,
    Machine,
    Started,
    Ended,
}

#[derive(DeriveIden)]
enum Streams {
    Table,
    File,
    Offset,
    Task,
    Doing,
    Tokens,
}

#[derive(Clone, Copy, DeriveIden)]
enum Issues {
    Table,
    UltraRun,
    TurnFile,
}
