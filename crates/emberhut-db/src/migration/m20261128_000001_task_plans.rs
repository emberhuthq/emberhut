//! A task's plan: the ordered steps its agent works through.
//!
//! - `plan_steps`: one row per step of a task's plan. `step` is the step's id within its task
//!   (stable for the life of the plan, never given to another of the task's steps), `parent`
//!   the `step` of the top-level step it is a sub-step of (steps nest one level), `position`
//!   its place in the task's list (each top-level step, then its sub-steps). `state` is
//!   `todo`, `doing`, `done`, `blocked` or `skipped`. Its rows go with their task.
//! - `issues.plan_updated`: when the task's plan last changed; NULL while it was never written.
//! - `issues.plan_last_step`: the highest step id the task has given out, so a removed step's
//!   id is never given to a new one; NULL before the first.
//! - `issues.unplanned_turn`: when the last turn of work on the task ended while its plan was
//!   empty; NULL once a plan is written.
//!
//! The plan's estimate is the task's own `eta`. Every new column is NULL for existing tasks,
//! whose plans start empty: nothing is filled in for them.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_integer_null, big_pk_auto, integer, integer_null, string, text, text_null,
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
                    .table(PlanSteps::Table)
                    .col(big_pk_auto(PlanSteps::Id))
                    .col(big_integer(PlanSteps::Issue))
                    .col(integer(PlanSteps::Step))
                    .col(integer_null(PlanSteps::Parent))
                    .col(integer(PlanSteps::Position))
                    .col(text(PlanSteps::Title))
                    .col(string(PlanSteps::State))
                    .col(text_null(PlanSteps::Note))
                    .col(text_null(PlanSteps::Link))
                    .col(big_integer_null(PlanSteps::Started))
                    .col(big_integer_null(PlanSteps::Finished))
                    .col(big_integer(PlanSteps::Created))
                    .col(big_integer(PlanSteps::Updated))
                    .foreign_key(
                        ForeignKey::create()
                            .from(PlanSteps::Table, PlanSteps::Issue)
                            .to(Issues::Table, Issues::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("plan_steps_issue_position")
                    .table(PlanSteps::Table)
                    .col(PlanSteps::Issue)
                    .col(PlanSteps::Position)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("plan_steps_issue_step")
                    .table(PlanSteps::Table)
                    .col(PlanSteps::Issue)
                    .col(PlanSteps::Step)
                    .unique()
                    .to_owned(),
            )
            .await?;
        for column in [
            big_integer_null(Issues::PlanUpdated),
            integer_null(Issues::PlanLastStep),
            big_integer_null(Issues::UnplannedTurn),
        ] {
            manager
                .alter_table(
                    Table::alter()
                        .table(Issues::Table)
                        .add_column(column)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in [
            Issues::UnplannedTurn,
            Issues::PlanLastStep,
            Issues::PlanUpdated,
        ] {
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
            .drop_table(Table::drop().table(PlanSteps::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Issues {
    Table,
    Id,
    PlanUpdated,
    PlanLastStep,
    UnplannedTurn,
}

#[derive(DeriveIden)]
enum PlanSteps {
    Table,
    Id,
    Issue,
    Step,
    Parent,
    Position,
    Title,
    State,
    Note,
    Link,
    Started,
    Finished,
    Created,
    Updated,
}
