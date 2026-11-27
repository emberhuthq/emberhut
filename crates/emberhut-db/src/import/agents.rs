//! What an import writes of agents and approvals (ADR 0027, "Importing an existing setup"),
//! inside the import's one transaction, and its reading back for the checks.
//!
//! Every row goes through the agent and approval stores' own writes (their checks included),
//! run on the import's transaction.

use emberhut_core::agents::{Agent, AgentAccount, AgentId, AgentTool, Channel, TrustedRequester};
use emberhut_core::approvals::{
    ApprovalRequest, ApproverId, ApproverRecord, ImportedApproval, Policy, RequestState,
};
use jiff::Timestamp;
use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};

use super::ImportStore;
use crate::agents::{
    create_approver_in, create_in, replace_accounts_in, replace_channels_in, replace_policies_in,
    replace_tools_in, replace_trusted_requesters_in, rows,
};
use crate::approvals::{import_in, rows::requests};
use crate::entity::{
    agent_account, agent_channel, agent_thread, agent_tool, approval_policy, approval_request,
    approver, configured_agent, trusted_requester,
};
use crate::{AgentStoreError, ApprovalStoreError, DbError};

/// Keys per read-back query.
const KEYS: usize = 500;

/// An agent's lists, as an import writes them whole.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentLists {
    /// Its channels, in order.
    pub channels: Vec<Channel>,
    /// Its tools, in order.
    pub tools: Vec<AgentTool>,
    /// Its accounts, in order.
    pub accounts: Vec<AgentAccount>,
    /// Its policies (read back by kind).
    pub policies: Vec<Policy>,
}

/// A thread an agent is in, as `agent_threads` holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedAgentThread {
    /// The agent.
    pub agent_id: String,
    /// The channel it is in.
    pub channel: String,
    /// The thread.
    pub thread: String,
    /// When a message in it last reached the agent, in milliseconds.
    pub last_seen: i64,
    /// Who said that message.
    pub last_speaker: Option<String>,
    /// The Slack team the conversation is in, when it is Slack's.
    pub workspace: Option<String>,
}

impl From<agent_thread::Model> for ImportedAgentThread {
    fn from(row: agent_thread::Model) -> Self {
        Self {
            agent_id: row.agent_id,
            channel: row.channel,
            thread: row.thread,
            last_seen: row.last_seen,
            last_speaker: row.last_speaker,
            workspace: row.workspace,
        }
    }
}

impl ImportStore {
    /// Stores an approver, made at `now`.
    ///
    /// # Errors
    ///
    /// [`AgentStoreError::Approver`] for one [`ApproverRecord::validate`] refuses,
    /// [`AgentStoreError::ApproverExists`], or the database's.
    pub async fn insert_approver(
        &self,
        record: &ApproverRecord,
        now: Timestamp,
    ) -> Result<(), AgentStoreError> {
        create_approver_in(&self.txn, record, now).await
    }

    /// The approver `id`, if there is one.
    ///
    /// # Errors
    ///
    /// [`DbError`], [`DbError::CorruptValue`] for a row that isn't an approver.
    pub async fn approver(&self, id: &ApproverId) -> Result<Option<ApproverRecord>, DbError> {
        approver::Entity::find_by_id(id.as_str())
            .one(&self.txn)
            .await?
            .map(rows::approver)
            .transpose()
    }

    /// Stores an agent, made at `now`. Its room and project must be in already.
    ///
    /// # Errors
    ///
    /// [`AgentStoreError::Invalid`] for an agent [`Agent::validate`] refuses,
    /// [`AgentStoreError::AgentExists`], [`AgentStoreError::NoRoom`],
    /// [`AgentStoreError::RoomTaken`], [`AgentStoreError::NoProject`], or the database's.
    pub async fn insert_agent(&self, agent: &Agent, now: Timestamp) -> Result<(), AgentStoreError> {
        create_in(&self.txn, agent, now).await
    }

    /// The agent `id`, if there is one.
    ///
    /// # Errors
    ///
    /// [`DbError`], [`DbError::CorruptValue`] for a row that isn't an agent.
    pub async fn agent(&self, id: &AgentId) -> Result<Option<Agent>, DbError> {
        configured_agent::Entity::find_by_id(id.as_str())
            .one(&self.txn)
            .await?
            .map(rows::agent)
            .transpose()
    }

    /// Stores the agent's lists, each checked as the agent store checks it; a policy's
    /// approver must be in already.
    ///
    /// # Errors
    ///
    /// [`AgentStoreError::Invalid`] or [`AgentStoreError::Policy`] for a list the rules
    /// refuse, [`AgentStoreError::NoAgent`], [`AgentStoreError::NoApprover`], or the database's.
    pub async fn insert_agent_lists(
        &self,
        agent: &AgentId,
        lists: &AgentLists,
    ) -> Result<(), AgentStoreError> {
        replace_channels_in(&self.txn, agent, &lists.channels).await?;
        replace_tools_in(&self.txn, agent, &lists.tools).await?;
        replace_accounts_in(&self.txn, agent, &lists.accounts).await?;
        replace_policies_in(&self.txn, agent, &lists.policies).await
    }

    /// The agent's lists: channels, tools and accounts in order, policies by kind.
    ///
    /// # Errors
    ///
    /// [`DbError`], [`DbError::CorruptValue`] for a row that isn't one of its list's.
    pub async fn agent_lists(&self, agent: &AgentId) -> Result<AgentLists, DbError> {
        let channels = agent_channel::Entity::find()
            .filter(agent_channel::Column::AgentId.eq(agent.as_str()))
            .order_by_asc(agent_channel::Column::Position)
            .all(&self.txn)
            .await?
            .into_iter()
            .map(rows::channel)
            .collect::<Result<_, _>>()?;
        let tools = agent_tool::Entity::find()
            .filter(agent_tool::Column::AgentId.eq(agent.as_str()))
            .order_by_asc(agent_tool::Column::Position)
            .all(&self.txn)
            .await?
            .into_iter()
            .map(rows::tool)
            .collect::<Result<_, _>>()?;
        let accounts = agent_account::Entity::find()
            .filter(agent_account::Column::AgentId.eq(agent.as_str()))
            .order_by_asc(agent_account::Column::Position)
            .all(&self.txn)
            .await?
            .into_iter()
            .map(rows::account)
            .collect::<Result<_, _>>()?;
        let mut policies = approval_policy::Entity::find()
            .filter(approval_policy::Column::AgentId.eq(agent.as_str()))
            .all(&self.txn)
            .await?
            .iter()
            .map(rows::policy)
            .collect::<Result<Vec<_>, _>>()?;
        policies.sort_by_key(|p| p.kind);
        Ok(AgentLists {
            channels,
            tools,
            accounts,
            policies,
        })
    }

    /// Stores the trusted requesters; an agent they are scoped to must be in already.
    ///
    /// # Errors
    ///
    /// [`AgentStoreError::Invalid`] for a list [`validate_requesters`](emberhut_core::agents::validate_requesters) refuses,
    /// [`AgentStoreError::NoAgent`], or the database's.
    pub async fn insert_trusted_requesters(
        &self,
        requesters: &[TrustedRequester],
    ) -> Result<(), AgentStoreError> {
        replace_trusted_requesters_in(&self.txn, requesters).await
    }

    /// Every trusted requester, in the order they were stored.
    ///
    /// # Errors
    ///
    /// [`DbError`], [`DbError::CorruptValue`] for a row whose agent isn't an agent's id.
    pub async fn trusted_requesters(&self) -> Result<Vec<TrustedRequester>, DbError> {
        trusted_requester::Entity::find()
            .order_by_asc(trusted_requester::Column::Id)
            .all(&self.txn)
            .await?
            .into_iter()
            .map(|row| {
                Ok(TrustedRequester {
                    agent: row.agent_id.as_deref().map(rows::agent_id).transpose()?,
                    requester: row.requester,
                })
            })
            .collect()
    }

    /// Inserts imported [`ImportedAgentThread`]s as they are.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn insert_agent_threads(
        &self,
        threads: &[ImportedAgentThread],
    ) -> Result<(), DbError> {
        for chunk in threads.chunks(KEYS) {
            agent_thread::Entity::insert_many(chunk.iter().map(|t| agent_thread::ActiveModel {
                agent_id: Set(t.agent_id.clone()),
                channel: Set(t.channel.clone()),
                thread: Set(t.thread.clone()),
                last_seen: Set(t.last_seen),
                last_speaker: Set(t.last_speaker.clone()),
                workspace: Set(t.workspace.clone()),
            }))
            .exec(&self.txn)
            .await?;
        }
        Ok(())
    }

    /// The threads of the agent `agent` among `channels`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn agent_threads(
        &self,
        agent: &str,
        channels: &[String],
    ) -> Result<Vec<ImportedAgentThread>, DbError> {
        let mut found = Vec::new();
        for chunk in channels.chunks(KEYS) {
            found.extend(
                agent_thread::Entity::find()
                    .filter(agent_thread::Column::AgentId.eq(agent))
                    .filter(agent_thread::Column::Channel.is_in(chunk.iter().cloned()))
                    .all(&self.txn)
                    .await?
                    .into_iter()
                    .map(ImportedAgentThread::from),
            );
        }
        Ok(found)
    }

    /// Stores requests from an earlier installation's history, each as
    /// [`crate::ApprovalStore::import`] stores one, in order.
    ///
    /// # Errors
    ///
    /// As [`crate::ApprovalStore::import`].
    pub async fn import_approvals(
        &self,
        imported: &[ImportedApproval],
    ) -> Result<(), ApprovalStoreError> {
        for one in imported {
            import_in(&self.txn, one).await?;
        }
        Ok(())
    }

    /// The requests whose `source_ref` is one of `refs`, with their files.
    ///
    /// # Errors
    ///
    /// [`DbError`], [`DbError::CorruptValue`] or [`DbError::CorruptHash`] for a corrupt row.
    pub async fn approvals_by_source(
        &self,
        refs: &[String],
    ) -> Result<Vec<ApprovalRequest>, DbError> {
        let mut found = Vec::with_capacity(refs.len());
        for chunk in refs.chunks(KEYS) {
            let rows = approval_request::Entity::find()
                .filter(approval_request::Column::SourceRef.is_in(chunk.iter().cloned()))
                .order_by_asc(approval_request::Column::Id)
                .all(&self.txn)
                .await?;
            found.extend(requests(&self.txn, rows).await?);
        }
        Ok(found)
    }

    /// How many of the agent's requests are in a state that waits or could be used (pending
    /// or approved): an import leaves none.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn open_approvals(&self, agent: &AgentId) -> Result<u64, DbError> {
        Ok(approval_request::Entity::find()
            .filter(approval_request::Column::AgentId.eq(agent.as_str()))
            .filter(
                approval_request::Column::State
                    .is_in(RequestState::WITHDRAWABLE.map(RequestState::as_str)),
            )
            .count(&self.txn)
            .await?)
    }
}
