//! Agents and approvals.
//!
//! - `configured_agents`: configured assistants, with the default agent's row (`main`, in
//!   Main). Named apart from `agents`, which holds the sub-agents of tasks' runs.
//! - `agent_channels`, `agent_tools`, `agent_accounts`: each agent's lists, in order.
//! - `agent_threads`: the conversations an agent is in, one row per agent, channel and thread.
//! - `approvers`: external approvers and how they are reached.
//! - `approval_policies`: who approves each kind of an agent's actions, one row per kind.
//! - `trusted_requesters`: people whose requests count as the owner's own.
//! - `approval_requests` and `approval_files`: every approval, its payload and its life.
//! - `agent_spend`: one row per used payment, summed by month against an agent's cap.
//!
//! Deleting an agent deletes its lists, threads, policies and trusted requesters; its approval
//! requests and spending keep it (a restricting foreign key), so an agent with approval history
//! is disabled rather than deleted. Times are milliseconds since the epoch; hashes are their 32
//! bytes.

use sea_orm_migration::async_trait::async_trait;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_integer_null, big_pk_auto, blob, blob_null, boolean, integer, string,
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
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        create_agents(manager).await?;
        create_agent_lists(manager).await?;
        create_approvers(manager).await?;
        create_requests(manager).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            AgentSpend::Table.into_iden(),
            ApprovalFiles::Table.into_iden(),
            ApprovalRequests::Table.into_iden(),
            TrustedRequesters::Table.into_iden(),
            ApprovalPolicies::Table.into_iden(),
            Approvers::Table.into_iden(),
            AgentThreads::Table.into_iden(),
            AgentAccounts::Table.into_iden(),
            AgentTools::Table.into_iden(),
            AgentChannels::Table.into_iden(),
            ConfiguredAgents::Table.into_iden(),
        ] {
            manager
                .drop_table(Table::drop().table(table).to_owned())
                .await?;
        }
        Ok(())
    }
}

/// A foreign key from `table`'s `agent_id` to the agent, deleted with it.
fn to_agent<T: IntoIden + Copy>(table: T, column: T) -> ForeignKeyCreateStatement {
    ForeignKey::create()
        .from(table, column)
        .to(ConfiguredAgents::Table, ConfiguredAgents::Id)
        .on_delete(ForeignKeyAction::Cascade)
        .to_owned()
}

/// A foreign key from `table`'s `agent_id` to the agent that keeps the agent while a row
/// names it: approval history outlives nothing it refers to, so an agent with any is disabled,
/// never deleted.
fn kept_by_agent<T: IntoIden + Copy>(table: T, column: T) -> ForeignKeyCreateStatement {
    ForeignKey::create()
        .from(table, column)
        .to(ConfiguredAgents::Table, ConfiguredAgents::Id)
        .on_delete(ForeignKeyAction::Restrict)
        .to_owned()
}

/// `configured_agents`, with the default agent's row.
async fn create_agents(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(ConfiguredAgents::Table)
                .col(string(ConfiguredAgents::Id).primary_key())
                .col(text(ConfiguredAgents::Name))
                .col(text_null(ConfiguredAgents::Persona))
                .col(text(ConfiguredAgents::Principal))
                .col(big_integer(ConfiguredAgents::RoomId).unique_key())
                .col(big_integer_null(ConfiguredAgents::ProjectId))
                .col(big_integer_null(ConfiguredAgents::SpendCapCents))
                .col(string_null(ConfiguredAgents::SpendCurrency))
                .col(boolean(ConfiguredAgents::Enabled).default(true))
                .col(big_integer(ConfiguredAgents::CreatedAt))
                .col(big_integer(ConfiguredAgents::UpdatedAt))
                .foreign_key(
                    ForeignKey::create()
                        .from(ConfiguredAgents::Table, ConfiguredAgents::RoomId)
                        .to(Rooms::Table, Rooms::Id),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(ConfiguredAgents::Table, ConfiguredAgents::ProjectId)
                        .to(Projects::Table, Projects::Id)
                        .on_delete(ForeignKeyAction::SetNull),
                )
                .to_owned(),
        )
        .await?;
    let now = jiff::Timestamp::now().as_millisecond();
    manager
        .exec_stmt(
            Query::insert()
                .into_table(ConfiguredAgents::Table)
                .columns([
                    ConfiguredAgents::Id,
                    ConfiguredAgents::Name,
                    ConfiguredAgents::Principal,
                    ConfiguredAgents::RoomId,
                    ConfiguredAgents::Enabled,
                    ConfiguredAgents::CreatedAt,
                    ConfiguredAgents::UpdatedAt,
                ])
                .values([
                    "main".into(),
                    "Main".into(),
                    "the owner".into(),
                    1.into(),
                    true.into(),
                    now.into(),
                    now.into(),
                ])
                .map_err(|error| DbErr::Migration(error.to_string()))?
                .to_owned(),
        )
        .await
}

/// An agent's channels, tools, accounts and threads.
async fn create_agent_lists(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(AgentChannels::Table)
                .col(big_pk_auto(AgentChannels::Id))
                .col(string(AgentChannels::AgentId))
                .col(integer(AgentChannels::Position))
                .col(string(AgentChannels::Kind))
                .col(string_null(AgentChannels::Workspace))
                .col(string(AgentChannels::Address))
                .col(string_null(AgentChannels::TokenSecret))
                .foreign_key(&mut to_agent(AgentChannels::Table, AgentChannels::AgentId))
                .to_owned(),
        )
        .await?;
    manager
        .create_index(
            Index::create()
                .name("agent_channels_address")
                .table(AgentChannels::Table)
                .col(AgentChannels::Kind)
                .col(AgentChannels::Address)
                .to_owned(),
        )
        .await?;
    manager
        .create_table(
            Table::create()
                .table(AgentTools::Table)
                .col(big_pk_auto(AgentTools::Id))
                .col(string(AgentTools::AgentId))
                .col(integer(AgentTools::Position))
                .col(string(AgentTools::Kind))
                .col(string(AgentTools::Name))
                .col(text_null(AgentTools::Config))
                .foreign_key(&mut to_agent(AgentTools::Table, AgentTools::AgentId))
                .to_owned(),
        )
        .await?;
    manager
        .create_table(
            Table::create()
                .table(AgentAccounts::Table)
                .col(big_pk_auto(AgentAccounts::Id))
                .col(string(AgentAccounts::AgentId))
                .col(integer(AgentAccounts::Position))
                .col(string(AgentAccounts::Label))
                .col(string(AgentAccounts::Secret))
                .foreign_key(&mut to_agent(AgentAccounts::Table, AgentAccounts::AgentId))
                .to_owned(),
        )
        .await?;
    manager
        .create_index(
            Index::create()
                .name("agent_accounts_label")
                .table(AgentAccounts::Table)
                .col(AgentAccounts::AgentId)
                .col(AgentAccounts::Label)
                .unique()
                .to_owned(),
        )
        .await?;
    manager
        .create_table(
            Table::create()
                .table(AgentThreads::Table)
                .col(string(AgentThreads::AgentId))
                .col(string(AgentThreads::Channel))
                .col(string(AgentThreads::Thread))
                .col(big_integer(AgentThreads::LastSeen))
                .col(text_null(AgentThreads::LastSpeaker))
                .primary_key(
                    Index::create()
                        .col(AgentThreads::AgentId)
                        .col(AgentThreads::Channel)
                        .col(AgentThreads::Thread),
                )
                .foreign_key(&mut to_agent(AgentThreads::Table, AgentThreads::AgentId))
                .to_owned(),
        )
        .await
}

/// Approvers, policies and trusted requesters.
async fn create_approvers(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(Approvers::Table)
                .col(string(Approvers::Id).primary_key())
                .col(text(Approvers::Name))
                .col(string_null(Approvers::SlackWorkspace))
                .col(string_null(Approvers::SlackUser))
                .col(text_null(Approvers::WebhookUrl))
                .col(string_null(Approvers::WebhookSecret))
                .col(boolean(Approvers::Link).default(false))
                .col(big_integer(Approvers::CreatedAt))
                .col(big_integer(Approvers::UpdatedAt))
                .to_owned(),
        )
        .await?;
    manager
        .create_table(
            Table::create()
                .table(ApprovalPolicies::Table)
                .col(big_pk_auto(ApprovalPolicies::Id))
                .col(string(ApprovalPolicies::AgentId))
                .col(string(ApprovalPolicies::Kind))
                .col(string(ApprovalPolicies::Approver))
                .foreign_key(&mut to_agent(
                    ApprovalPolicies::Table,
                    ApprovalPolicies::AgentId,
                ))
                .to_owned(),
        )
        .await?;
    manager
        .create_index(
            Index::create()
                .name("approval_policies_agent_kind")
                .table(ApprovalPolicies::Table)
                .col(ApprovalPolicies::AgentId)
                .col(ApprovalPolicies::Kind)
                .unique()
                .to_owned(),
        )
        .await?;
    manager
        .create_table(
            Table::create()
                .table(TrustedRequesters::Table)
                .col(big_pk_auto(TrustedRequesters::Id))
                .col(text(TrustedRequesters::Requester))
                .col(string_null(TrustedRequesters::AgentId))
                .foreign_key(&mut to_agent(
                    TrustedRequesters::Table,
                    TrustedRequesters::AgentId,
                ))
                .to_owned(),
        )
        .await
}

/// Approval requests, their files, and the spending they allowed.
async fn create_requests(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(ApprovalRequests::Table)
                .col(big_pk_auto(ApprovalRequests::Id))
                .col(string(ApprovalRequests::AgentId))
                .col(string(ApprovalRequests::Kind))
                .col(text(ApprovalRequests::Target))
                .col(text(ApprovalRequests::Summary))
                .col(text_null(ApprovalRequests::Text))
                .col(blob(ApprovalRequests::PayloadHash))
                .col(string(ApprovalRequests::Approver))
                .col(big_integer_null(ApprovalRequests::AmountCents))
                .col(big_integer_null(ApprovalRequests::TaskId))
                .col(string(ApprovalRequests::State))
                .col(text_null(ApprovalRequests::StateReason))
                .col(text_null(ApprovalRequests::SourceRef))
                .col(big_integer(ApprovalRequests::CreatedAt))
                .col(string_null(ApprovalRequests::Decision))
                .col(big_integer_null(ApprovalRequests::DecidedAt))
                .col(text_null(ApprovalRequests::DecidedBy))
                .col(string_null(ApprovalRequests::DecidedVia))
                .col(big_integer_null(ApprovalRequests::UsedAt))
                .col(text_null(ApprovalRequests::UsedBy))
                .col(big_integer_null(ApprovalRequests::ClosedAt))
                .col(blob_null(ApprovalRequests::LinkNonceHash).unique_key())
                .col(string_null(ApprovalRequests::SlackChannel))
                .col(string_null(ApprovalRequests::SlackTs))
                .foreign_key(&mut kept_by_agent(
                    ApprovalRequests::Table,
                    ApprovalRequests::AgentId,
                ))
                .foreign_key(
                    ForeignKey::create()
                        .from(ApprovalRequests::Table, ApprovalRequests::TaskId)
                        .to(Issues::Table, Issues::Id)
                        .on_delete(ForeignKeyAction::SetNull),
                )
                .to_owned(),
        )
        .await?;
    manager
        .create_index(
            Index::create()
                .name("approval_requests_state")
                .table(ApprovalRequests::Table)
                .col(ApprovalRequests::State)
                .to_owned(),
        )
        .await?;
    manager
        .create_index(
            Index::create()
                .name("approval_requests_payload")
                .table(ApprovalRequests::Table)
                .col(ApprovalRequests::AgentId)
                .col(ApprovalRequests::Kind)
                .col(ApprovalRequests::PayloadHash)
                .to_owned(),
        )
        .await?;
    create_files_and_spend(manager).await
}

/// A request's files, and the spending used payments add up to.
async fn create_files_and_spend(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table(ApprovalFiles::Table)
                .col(big_pk_auto(ApprovalFiles::Id))
                .col(big_integer(ApprovalFiles::RequestId))
                .col(integer(ApprovalFiles::Position))
                .col(text(ApprovalFiles::Name))
                .col(big_integer_null(ApprovalFiles::Size))
                .col(blob(ApprovalFiles::Sha256))
                .foreign_key(
                    ForeignKey::create()
                        .from(ApprovalFiles::Table, ApprovalFiles::RequestId)
                        .to(ApprovalRequests::Table, ApprovalRequests::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
    manager
        .create_index(
            Index::create()
                .name("approval_files_request")
                .table(ApprovalFiles::Table)
                .col(ApprovalFiles::RequestId)
                .to_owned(),
        )
        .await?;
    manager
        .create_table(
            Table::create()
                .table(AgentSpend::Table)
                .col(big_pk_auto(AgentSpend::Id))
                .col(string(AgentSpend::AgentId))
                .col(big_integer(AgentSpend::RequestId).unique_key())
                .col(big_integer(AgentSpend::AmountCents))
                .col(string(AgentSpend::Month))
                .col(big_integer(AgentSpend::At))
                .foreign_key(&mut kept_by_agent(AgentSpend::Table, AgentSpend::AgentId))
                .foreign_key(
                    ForeignKey::create()
                        .from(AgentSpend::Table, AgentSpend::RequestId)
                        .to(ApprovalRequests::Table, ApprovalRequests::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
    manager
        .create_index(
            Index::create()
                .name("agent_spend_month")
                .table(AgentSpend::Table)
                .col(AgentSpend::AgentId)
                .col(AgentSpend::Month)
                .to_owned(),
        )
        .await
}

#[derive(DeriveIden, Clone, Copy)]
enum ConfiguredAgents {
    Table,
    Id,
    Name,
    Persona,
    Principal,
    RoomId,
    ProjectId,
    SpendCapCents,
    SpendCurrency,
    Enabled,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden, Clone, Copy)]
enum AgentChannels {
    Table,
    Id,
    AgentId,
    Position,
    Kind,
    Workspace,
    Address,
    TokenSecret,
}

#[derive(DeriveIden, Clone, Copy)]
enum AgentTools {
    Table,
    Id,
    AgentId,
    Position,
    Kind,
    Name,
    Config,
}

#[derive(DeriveIden, Clone, Copy)]
enum AgentAccounts {
    Table,
    Id,
    AgentId,
    Position,
    Label,
    Secret,
}

#[derive(DeriveIden, Clone, Copy)]
enum AgentThreads {
    Table,
    AgentId,
    Channel,
    Thread,
    LastSeen,
    LastSpeaker,
}

#[derive(DeriveIden, Clone, Copy)]
enum Approvers {
    Table,
    Id,
    Name,
    SlackWorkspace,
    SlackUser,
    WebhookUrl,
    WebhookSecret,
    Link,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden, Clone, Copy)]
enum ApprovalPolicies {
    Table,
    Id,
    AgentId,
    Kind,
    Approver,
}

#[derive(DeriveIden, Clone, Copy)]
enum TrustedRequesters {
    Table,
    Id,
    Requester,
    AgentId,
}

#[derive(DeriveIden, Clone, Copy)]
enum ApprovalRequests {
    Table,
    Id,
    AgentId,
    Kind,
    Target,
    Summary,
    Text,
    PayloadHash,
    Approver,
    AmountCents,
    TaskId,
    State,
    StateReason,
    SourceRef,
    CreatedAt,
    Decision,
    DecidedAt,
    DecidedBy,
    DecidedVia,
    UsedAt,
    UsedBy,
    ClosedAt,
    LinkNonceHash,
    SlackChannel,
    SlackTs,
}

#[derive(DeriveIden, Clone, Copy)]
enum ApprovalFiles {
    Table,
    Id,
    RequestId,
    Position,
    Name,
    Size,
    Sha256,
}

#[derive(DeriveIden, Clone, Copy)]
enum AgentSpend {
    Table,
    Id,
    AgentId,
    RequestId,
    AmountCents,
    Month,
    At,
}

#[derive(DeriveIden)]
enum Rooms {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Projects {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Issues {
    Table,
    Id,
}
