//! Tasks made for runner sessions.
//!
//! - `issues.made_for_session`: the runner session a task was made for automatically (a session
//!   started without a task), so the server can tell such a task from one someone made; NULL
//!   for every other task, existing ones included. A session moved to another task leaves its
//!   own task behind, and the server puts that task away only when it was made for that session
//!   and holds nothing else.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::text_null;

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
                    .table(Issues::Table)
                    .add_column(text_null(Issues::MadeForSession))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Issues::Table)
                    .drop_column(Issues::MadeForSession)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Issues {
    Table,
    MadeForSession,
}
