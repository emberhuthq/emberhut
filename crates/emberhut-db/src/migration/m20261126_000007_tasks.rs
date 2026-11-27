//! Tasks and what hangs off them.
//!
//! - `issues`: every task, the person's and the assistant's, with its owner, status, project,
//!   area, place in the tree of subtasks (`parent`), its ask (`decision`, `action`) and its
//!   worker's columns; a finding is a row with `kind = 'finding'`. `last_note` keeps the newest
//!   history line, so a list of tasks reads no history.
//! - `issue_events`: each task's history, oldest first.
//! - `artifacts`: what a task produced: files on this machine, links, X replies (`data`).
//! - `projects`: what tasks are grouped by, with priority, deadline and status.
//! - `areas`: the parts of a project that run for a while on their own.
//! - `links`: the threads, tickets and chats that move a task.
//! - `runs`: every turn of work on a task (a thread's turn, or a worker session).
//!
//! Days (`due`, `decide_by`, `deadline`, `paused_until`) are `YYYY-MM-DD` text, as the
//! contract writes them and as they sort; `parked_until` is a day or an instant as given.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_integer_null, big_pk_auto, boolean, integer, integer_null, string,
    string_null, text, text_null,
};

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait]
#[expect(
    elided_lifetimes_in_paths,
    reason = "async_trait matches the trait's signature, which elides SchemaManager's lifetime"
)]
impl MigrationTrait for Migration {
    #[expect(
        clippy::too_many_lines,
        reason = "one table after another, as the schema reads"
    )]
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Issues::Table)
                    .col(big_pk_auto(Issues::Id))
                    .col(text(Issues::Title))
                    .col(text_null(Issues::Body))
                    .col(string(Issues::Owner))
                    .col(string(Issues::Status))
                    .col(big_integer_null(Issues::Goal))
                    .col(text_null(Issues::Project))
                    .col(big_integer_null(Issues::Pid))
                    .col(boolean(Issues::Hard).default(false))
                    .col(string_null(Issues::Size))
                    .col(integer(Issues::Priority).default(3))
                    .col(string_null(Issues::Due))
                    .col(boolean(Issues::Today).default(false))
                    .col(string_null(Issues::ParkedUntil))
                    .col(string_null(Issues::DecideBy))
                    .col(text_null(Issues::Decision))
                    .col(text_null(Issues::Action))
                    .col(big_integer(Issues::Created))
                    .col(big_integer(Issues::Updated))
                    .col(big_integer_null(Issues::DoneAt))
                    .col(text_null(Issues::LastNote))
                    .col(big_integer_null(Issues::Parent))
                    .col(big_integer_null(Issues::Progress))
                    .col(big_integer_null(Issues::Total))
                    .col(string_null(Issues::CountUnit))
                    .col(boolean(Issues::Tracked).default(false))
                    .col(text_null(Issues::Eta))
                    .col(boolean(Issues::Ultra).default(false))
                    .col(integer_null(Issues::UltraParallel))
                    .col(big_integer_null(Issues::UltraBudget))
                    .col(string_null(Issues::Model))
                    .col(big_integer_null(Issues::Area))
                    .col(string_null(Issues::Kind))
                    .col(string_null(Issues::Landed))
                    .col(big_integer_null(Issues::FixedBy))
                    .col(string_null(Issues::Runner))
                    .col(text_null(Issues::Next))
                    .col(string_null(Issues::Unit))
                    .col(string_null(Issues::Session))
                    .col(text_null(Issues::Requested))
                    .col(big_integer_null(Issues::Room))
                    .to_owned(),
            )
            .await?;
        for (name, column) in [
            ("issues_parent", Issues::Parent),
            ("issues_area", Issues::Area),
            ("issues_status", Issues::Status),
            ("issues_pid", Issues::Pid),
        ] {
            manager
                .create_index(
                    Index::create()
                        .name(name)
                        .table(Issues::Table)
                        .col(column)
                        .to_owned(),
                )
                .await?;
        }
        manager
            .create_table(
                Table::create()
                    .table(IssueEvents::Table)
                    .col(big_pk_auto(IssueEvents::Id))
                    .col(big_integer(IssueEvents::Issue))
                    .col(big_integer(IssueEvents::At))
                    .col(string(IssueEvents::Who))
                    .col(text(IssueEvents::Text))
                    .foreign_key(
                        ForeignKey::create()
                            .from(IssueEvents::Table, IssueEvents::Issue)
                            .to(Issues::Table, Issues::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("issue_events_issue")
                    .table(IssueEvents::Table)
                    .col(IssueEvents::Issue)
                    .col(IssueEvents::Id)
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Artifacts::Table)
                    .col(big_pk_auto(Artifacts::Id))
                    .col(big_integer(Artifacts::Issue))
                    .col(text(Artifacts::Ref))
                    .col(string(Artifacts::Kind))
                    .col(text_null(Artifacts::Caption))
                    .col(big_integer(Artifacts::At))
                    .col(text_null(Artifacts::Data))
                    .foreign_key(
                        ForeignKey::create()
                            .from(Artifacts::Table, Artifacts::Issue)
                            .to(Issues::Table, Issues::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("artifacts_issue_ref")
                    .table(Artifacts::Table)
                    .col(Artifacts::Issue)
                    .col(Artifacts::Ref)
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Projects::Table)
                    .col(big_pk_auto(Projects::Id))
                    .col(text(Projects::Name))
                    .col(big_integer_null(Projects::Goal))
                    .col(integer(Projects::Priority).default(0))
                    .col(string(Projects::Kind).default("deliverable"))
                    .col(string_null(Projects::Deadline))
                    .col(boolean(Projects::Hard).default(false))
                    .col(string(Projects::Status).default("active"))
                    .col(string_null(Projects::PausedUntil))
                    .col(big_integer(Projects::Created))
                    .col(big_integer(Projects::Updated))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Areas::Table)
                    .col(big_pk_auto(Areas::Id))
                    .col(big_integer(Areas::Pid))
                    .col(text(Areas::Name))
                    .col(string(Areas::Status).default("active"))
                    .col(text_null(Areas::Goal))
                    .col(text_null(Areas::About))
                    .col(big_integer(Areas::Created))
                    .col(big_integer(Areas::Updated))
                    .foreign_key(
                        ForeignKey::create()
                            .from(Areas::Table, Areas::Pid)
                            .to(Projects::Table, Projects::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("areas_pid")
                    .table(Areas::Table)
                    .col(Areas::Pid)
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Links::Table)
                    .col(big_integer(Links::Issue))
                    .col(string(Links::Kind))
                    .col(string(Links::Ref))
                    .col(string_null(Links::Machine))
                    .primary_key(
                        Index::create()
                            .col(Links::Issue)
                            .col(Links::Kind)
                            .col(Links::Ref),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(Links::Table, Links::Issue)
                            .to(Issues::Table, Issues::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("links_ref")
                    .table(Links::Table)
                    .col(Links::Kind)
                    .col(Links::Ref)
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Runs::Table)
                    .col(big_pk_auto(Runs::Id))
                    .col(big_integer_null(Runs::Task))
                    .col(string(Runs::Machine))
                    .col(string(Runs::Thread))
                    .col(string_null(Runs::Turn))
                    .col(string(Runs::Kind).default("thread"))
                    .col(text_null(Runs::Asked))
                    .col(big_integer_null(Runs::Requested))
                    .col(big_integer_null(Runs::Started))
                    .col(big_integer_null(Runs::Ended))
                    .col(string_null(Runs::Outcome))
                    .col(text_null(Runs::Last))
                    .col(text_null(Runs::StatusLine))
                    .col(boolean(Runs::Inferred).default(false))
                    .col(boolean(Runs::Applied).default(false))
                    .to_owned(),
            )
            .await?;
        for (name, first, second) in [
            ("runs_task", Runs::Task, Runs::Ended),
            ("runs_thread", Runs::Thread, Runs::Ended),
        ] {
            manager
                .create_index(
                    Index::create()
                        .name(name)
                        .table(Runs::Table)
                        .col(first)
                        .col(second)
                        .to_owned(),
                )
                .await?;
        }
        // One run per turn of a thread: a turn with no id yet (NULL) is never a duplicate.
        manager
            .create_index(
                Index::create()
                    .name("runs_turn")
                    .table(Runs::Table)
                    .col(Runs::Machine)
                    .col(Runs::Turn)
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            Runs::Table.into_iden(),
            Links::Table.into_iden(),
            Areas::Table.into_iden(),
            Projects::Table.into_iden(),
            Artifacts::Table.into_iden(),
            IssueEvents::Table.into_iden(),
            Issues::Table.into_iden(),
        ] {
            manager
                .drop_table(Table::drop().table(table).to_owned())
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Issues {
    Table,
    Id,
    Title,
    Body,
    Owner,
    Status,
    Goal,
    Project,
    Pid,
    Hard,
    Size,
    Priority,
    Due,
    Today,
    ParkedUntil,
    DecideBy,
    Decision,
    Action,
    Created,
    Updated,
    DoneAt,
    LastNote,
    Parent,
    Progress,
    Total,
    CountUnit,
    Tracked,
    Eta,
    Ultra,
    UltraParallel,
    UltraBudget,
    Model,
    Area,
    Kind,
    Landed,
    FixedBy,
    Runner,
    Next,
    Unit,
    Session,
    Requested,
    Room,
}

#[derive(DeriveIden)]
enum IssueEvents {
    Table,
    Id,
    Issue,
    At,
    Who,
    Text,
}

#[derive(DeriveIden)]
enum Artifacts {
    Table,
    Id,
    Issue,
    Ref,
    Kind,
    Caption,
    At,
    Data,
}

#[derive(DeriveIden)]
enum Projects {
    Table,
    Id,
    Name,
    Goal,
    Priority,
    Kind,
    Deadline,
    Hard,
    Status,
    PausedUntil,
    Created,
    Updated,
}

#[derive(DeriveIden)]
enum Areas {
    Table,
    Id,
    Pid,
    Name,
    Status,
    Goal,
    About,
    Created,
    Updated,
}

#[derive(DeriveIden)]
enum Links {
    Table,
    Issue,
    Kind,
    Ref,
    Machine,
}

#[derive(DeriveIden)]
enum Runs {
    Table,
    Id,
    Task,
    Machine,
    Thread,
    Turn,
    Kind,
    Asked,
    Requested,
    Started,
    Ended,
    Outcome,
    Last,
    StatusLine,
    Inferred,
    Applied,
}
