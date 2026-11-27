//! Three columns of the agents' tables, added after they shipped:
//!
//! - `configured_agents.worker_shell`: whether an agent's tasks' workers may run shell
//!   commands (as the server's user, for an agent the owner trusts); off by default;
//! - `configured_agents.made_room`: whether the agent's room was made with it (rather than an
//!   existing room given to it), so deleting the agent deletes that room and no other; off for
//!   rows made before it was recorded, whose rooms are then kept;
//! - `agent_threads.workspace`: the Slack team a conversation an agent is in belongs to, so its
//!   answer there is posted with that workspace's bot token; NULL for WhatsApp and for rows made
//!   before it was recorded.
//!
//! Neither changes an existing column.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{boolean, text_null};

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
                    .table(ConfiguredAgents::Table)
                    .add_column(boolean(ConfiguredAgents::WorkerShell).default(false))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(ConfiguredAgents::Table)
                    .add_column(boolean(ConfiguredAgents::MadeRoom).default(false))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(AgentThreads::Table)
                    .add_column(text_null(AgentThreads::Workspace))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(AgentThreads::Table)
                    .drop_column(AgentThreads::Workspace)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(ConfiguredAgents::Table)
                    .drop_column(ConfiguredAgents::MadeRoom)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(ConfiguredAgents::Table)
                    .drop_column(ConfiguredAgents::WorkerShell)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum ConfiguredAgents {
    Table,
    WorkerShell,
    MadeRoom,
}

#[derive(DeriveIden)]
enum AgentThreads {
    Table,
    Workspace,
}
