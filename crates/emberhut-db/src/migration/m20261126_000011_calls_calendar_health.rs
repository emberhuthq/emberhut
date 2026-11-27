//! Calls, the calendar's days, and the owner's health and screen time:
//!
//! - `calls`: a recorded or shared call, its audio tracks (JSON) and its transcript's file;
//! - `call_lines` and `call_hints`: a call's live transcript as it is heard, and the copilot's
//!   hints, removed with their call;
//! - `calendar_days`: each event the Mac's calendars gave, on each day it covers, as the Mac
//!   wrote it (JSON), so the days behind and a Mac that sleeps still show what was last seen;
//! - `health`: the phone's summary of each day (JSON), one row per day;
//! - `screentime`: the Mac's screen-time totals per day and device (JSON).

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_integer_null, big_pk_auto, boolean, double, double_null, string, string_null,
    text, text_null,
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
                    .table(Calls::Table)
                    .col(string(Calls::Id).primary_key())
                    .col(text(Calls::Title))
                    .col(big_integer_null(Calls::Task))
                    .col(big_integer(Calls::RoomId))
                    .col(string(Calls::Status))
                    .col(text(Calls::Tracks).default("[]"))
                    .col(text_null(Calls::Source))
                    .col(text_null(Calls::Transcript))
                    .col(text_null(Calls::Error))
                    .col(double_null(Calls::Seconds))
                    .col(big_integer(Calls::At))
                    .col(big_integer_null(Calls::DoneAt))
                    .col(boolean(Calls::AudioGone).default(false))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("calls_status_at")
                    .table(Calls::Table)
                    .col(Calls::Status)
                    .col(Calls::At)
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(CallLines::Table)
                    .col(big_pk_auto(CallLines::Id))
                    .col(string(CallLines::CallId))
                    .col(string(CallLines::Track))
                    .col(double(CallLines::T))
                    .col(text(CallLines::Text))
                    .col(string_null(CallLines::Lang))
                    .col(boolean(CallLines::Sure).default(false))
                    .col(big_integer(CallLines::At))
                    .foreign_key(
                        ForeignKey::create()
                            .from(CallLines::Table, CallLines::CallId)
                            .to(Calls::Table, Calls::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("call_lines_call_t")
                    .table(CallLines::Table)
                    .col(CallLines::CallId)
                    .col(CallLines::T)
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(CallHints::Table)
                    .col(big_pk_auto(CallHints::Id))
                    .col(string(CallHints::CallId))
                    .col(double_null(CallHints::T))
                    .col(text(CallHints::Text))
                    .col(big_integer(CallHints::At))
                    .foreign_key(
                        ForeignKey::create()
                            .from(CallHints::Table, CallHints::CallId)
                            .to(Calls::Table, Calls::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        days(manager).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            Screentime::Table.into_iden(),
            Health::Table.into_iden(),
            CalendarDays::Table.into_iden(),
            CallHints::Table.into_iden(),
            CallLines::Table.into_iden(),
            Calls::Table.into_iden(),
        ] {
            manager
                .drop_table(Table::drop().table(table).to_owned())
                .await?;
        }
        Ok(())
    }
}

/// The tables of days: `calendar_days`, `health` and `screentime`.
async fn days(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(CalendarDays::Table)
                .col(string(CalendarDays::Day))
                .col(string(CalendarDays::Id))
                .col(text(CalendarDays::Event))
                .col(big_integer_null(CalendarDays::Seen))
                .primary_key(Index::create().col(CalendarDays::Day).col(CalendarDays::Id))
                .to_owned(),
        )
        .await?;
    manager
        .create_table(
            Table::create()
                .table(Health::Table)
                .col(string(Health::Date).primary_key())
                .col(text(Health::Data))
                .col(big_integer(Health::At))
                .to_owned(),
        )
        .await?;
    manager
        .create_table(
            Table::create()
                .table(Screentime::Table)
                .col(string(Screentime::Date))
                .col(string(Screentime::Device))
                .col(text(Screentime::Data))
                .col(big_integer(Screentime::At))
                .primary_key(
                    Index::create()
                        .col(Screentime::Date)
                        .col(Screentime::Device),
                )
                .to_owned(),
        )
        .await
}

#[derive(DeriveIden)]
enum Calls {
    Table,
    Id,
    Title,
    Task,
    RoomId,
    Status,
    Tracks,
    Source,
    Transcript,
    Error,
    Seconds,
    At,
    DoneAt,
    AudioGone,
}

#[derive(DeriveIden)]
enum CallLines {
    Table,
    Id,
    CallId,
    Track,
    T,
    Text,
    Lang,
    Sure,
    At,
}

#[derive(DeriveIden)]
enum CallHints {
    Table,
    Id,
    CallId,
    T,
    Text,
    At,
}

#[derive(DeriveIden)]
enum CalendarDays {
    Table,
    Day,
    Id,
    Event,
    Seen,
}

#[derive(DeriveIden)]
enum Health {
    Table,
    Date,
    Data,
    At,
}

#[derive(DeriveIden)]
enum Screentime {
    Table,
    Date,
    Device,
    Data,
    At,
}
