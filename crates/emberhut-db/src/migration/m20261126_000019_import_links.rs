//! Two links an import carries and the server serves:
//!
//! - `rooms.project_id`: a project's room, which then shows its project's name, goal and status
//!   and the asks waiting in it; NULL for Main and every other room;
//! - `issues.requested_by`: a task asked for by someone other than the owner, as JSON
//!   (`who`, `at`, `how`).
//!
//! Neither changes an existing column.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer_null, text_null};

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
            .alter_table(
                Table::alter()
                    .table(Rooms::Table)
                    .add_column(big_integer_null(Rooms::ProjectId))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("rooms_project_id")
                    .table(Rooms::Table)
                    .col(Rooms::ProjectId)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Issues::Table)
                    .add_column(text_null(Issues::RequestedBy))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Issues::Table)
                    .drop_column(Issues::RequestedBy)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_index(
                Index::drop()
                    .name("rooms_project_id")
                    .table(Rooms::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Rooms::Table)
                    .drop_column(Rooms::ProjectId)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Rooms {
    Table,
    ProjectId,
}

#[derive(DeriveIden)]
enum Issues {
    Table,
    RequestedBy,
}
