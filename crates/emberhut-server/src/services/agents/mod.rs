//! Agents as the owner configures them, from a signed-in device only: each agent with its room,
//! channels, tools, accounts and policies; the external approvers; and the trusted requesters.
//! No agent tool reaches this service's writes.
//!
//! The rules are `emberhut_core`'s, checked again by the store: the default agent can't be deleted,
//! renamed, given a persona or narrowed; a tool that acts as the owner is no other agent's; a
//! fixed-core kind is the owner's whatever a policy says. A new agent gets a room of its own,
//! made with it in one transaction. An agent with approval history is disabled, not deleted.

pub mod conversations;
mod inbound;
pub mod scope;
pub mod slack;
mod wire;

use std::collections::HashMap;
use std::sync::Arc;

use emberhut_api::agents::{
    AccountsRequest, AgentChange, AgentPage, AgentRow, AgentsPage, Approver, ApproverChange,
    ApproversPage, ChannelsRequest, NewAgent, PoliciesRequest, ToolsRequest, TrustedRequestersPage,
};
use emberhut_api::patch::Patch;
use emberhut_core::Clock;
use emberhut_core::agents::{Agent, AgentId};
use emberhut_core::approvals::{ApproverId, ApproverRecord};
use emberhut_core::task::ProjectId;
use emberhut_db::{AgentStore, AgentStoreError, ApprovalStore, DbError};

pub use self::conversations::{AgentConversations, ConversationError, ConversationParts, Reply};
pub use self::inbound::InboundError;

/// Why an agent, or something configured around one, was not read or written.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AgentServiceError {
    /// The body asks for something an agent or its lists may not be (400).
    #[error("{0}")]
    Refused(String),
    /// There is no such agent or approver (404).
    #[error("{0}")]
    NotFound(String),
    /// It clashes with what is there (409): an id taken, a room another agent's, an agent with
    /// approval history, an approver a policy names.
    #[error("{0}")]
    Conflict(String),
    /// The database failed (500).
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<AgentStoreError> for AgentServiceError {
    fn from(error: AgentStoreError) -> Self {
        let words = error.to_string();
        match error {
            AgentStoreError::Invalid(_)
            | AgentStoreError::Policy(_)
            | AgentStoreError::Approver(_)
            | AgentStoreError::NoRoom(_)
            | AgentStoreError::NoProject(_)
            | AgentStoreError::NoApprover(_) => Self::Refused(words),
            AgentStoreError::NoAgent(_) => Self::NotFound(words),
            AgentStoreError::Db(error) => Self::Db(error),
            _ => Self::Conflict(words),
        }
    }
}

/// Reads and changes agents, approvers and trusted requesters. Cheap to clone.
#[derive(Clone, Debug)]
pub struct AgentService {
    agents: AgentStore,
    approvals: ApprovalStore,
    clock: Arc<dyn Clock>,
}

impl AgentService {
    /// The service over the stores, at `clock`'s time.
    #[must_use]
    pub fn new(agents: AgentStore, approvals: ApprovalStore, clock: Arc<dyn Clock>) -> Self {
        Self {
            agents,
            approvals,
            clock,
        }
    }

    /// The agents' store, for the areas that read an agent's lists as they act.
    #[must_use]
    pub fn store(&self) -> &AgentStore {
        &self.agents
    }

    /// How many requests wait on an answer, per agent id.
    async fn pending(&self) -> Result<HashMap<AgentId, u32>, DbError> {
        Ok(self.approvals.pending_counts().await?.into_iter().collect())
    }

    /// Every agent, the default agent first.
    ///
    /// # Errors
    ///
    /// The database's.
    pub async fn list(&self) -> Result<AgentsPage, DbError> {
        let pending = self.pending().await?;
        Ok(AgentsPage {
            agents: self
                .agents
                .list()
                .await?
                .iter()
                .map(|agent| wire::agent(agent, pending.get(&agent.id).copied().unwrap_or(0)))
                .collect(),
        })
    }

    /// The state's `agents`: every agent with its pending count, the default agent first, or
    /// none at all when the default agent is the only one (an installation without agents
    /// shows nothing new).
    ///
    /// # Errors
    ///
    /// The database's.
    pub async fn state_rows(&self) -> Result<Vec<AgentRow>, DbError> {
        let agents = self.agents.list().await?;
        if agents.iter().all(|agent| agent.id.is_main()) {
            return Ok(Vec::new());
        }
        let pending = self.pending().await?;
        Ok(agents
            .iter()
            .map(|agent| wire::agent_row(agent, pending.get(&agent.id).copied().unwrap_or(0)))
            .collect())
    }

    /// The agent `id` with its lists.
    ///
    /// # Errors
    ///
    /// [`AgentServiceError::NotFound`], or the database's.
    pub async fn page(&self, id: &str) -> Result<AgentPage, AgentServiceError> {
        let id = known_agent(id)?;
        let agent = self.existing(&id).await?;
        self.page_of(&agent).await
    }

    async fn existing(&self, id: &AgentId) -> Result<Agent, AgentServiceError> {
        self.agents
            .get(id)
            .await?
            .ok_or_else(|| AgentServiceError::NotFound(format!("no agent {id}")))
    }

    async fn page_of(&self, agent: &Agent) -> Result<AgentPage, AgentServiceError> {
        let pending = self.pending().await?;
        let id = &agent.id;
        Ok(AgentPage {
            agent: wire::agent(agent, pending.get(id).copied().unwrap_or(0)),
            channels: self
                .agents
                .channels(id)
                .await?
                .iter()
                .map(wire::channel_to_api)
                .collect(),
            tools: self
                .agents
                .tools(id)
                .await?
                .iter()
                .map(wire::tool_to_api)
                .collect(),
            accounts: self
                .agents
                .accounts(id)
                .await?
                .iter()
                .map(wire::account_to_api)
                .collect(),
            policies: self
                .agents
                .policies(id)
                .await?
                .iter()
                .map(wire::policy_to_api)
                .collect(),
        })
    }

    /// Makes the agent `new`, in the room it names or (none named) in a new room of its own.
    ///
    /// # Errors
    ///
    /// [`AgentServiceError::Refused`] for an agent that can't be one (Main as its room, a room
    /// or project that isn't there), [`AgentServiceError::Conflict`] for an id taken or a room
    /// another agent's, or the database's.
    pub async fn create(&self, new: &NewAgent) -> Result<AgentPage, AgentServiceError> {
        let agent = wire::new_agent(new).map_err(AgentServiceError::Refused)?;
        if agent.id.is_main() {
            return Err(AgentServiceError::Conflict(
                "main is the default agent's id".to_owned(),
            ));
        }
        let now = self.clock.now();
        let agent = if new.room.is_some() {
            self.agents.create(&agent, now).await?;
            agent
        } else {
            self.agents.create_in_new_room(&agent, now).await?
        };
        self.page_of(&agent).await
    }

    /// Changes the agent `id` as `change` says.
    ///
    /// # Errors
    ///
    /// [`AgentServiceError::Refused`] for a change the rules refuse (the default agent's name,
    /// persona or principal), [`AgentServiceError::NotFound`], or the database's.
    pub async fn change(
        &self,
        id: &str,
        change: &AgentChange,
    ) -> Result<AgentPage, AgentServiceError> {
        let id = known_agent(id)?;
        let mut agent = self.existing(&id).await?;
        if let Some(name) = &change.name {
            agent.name.clone_from(name);
        }
        if let Some(principal) = &change.principal {
            agent.principal.clone_from(principal);
        }
        patch(&mut agent.persona, &change.persona, Clone::clone);
        patch(&mut agent.project, &change.project, |project| {
            ProjectId::new(project.get())
        });
        match &change.spend_cap {
            Patch::Keep => {}
            Patch::Clear => agent.spend_cap = None,
            Patch::Set(cap) => {
                agent.spend_cap = Some(wire::spend_cap(cap).map_err(AgentServiceError::Refused)?);
            }
        }
        if let Some(enabled) = change.enabled {
            agent.enabled = enabled;
        }
        if let Some(shell) = change.worker_shell {
            agent.worker_shell = shell;
        }
        self.agents.update(&agent, self.clock.now()).await?;
        self.page_of(&agent).await
    }

    /// Deletes the agent `id`, what is configured around it and its empty room.
    ///
    /// # Errors
    ///
    /// [`AgentServiceError::Refused`] for the default agent, [`AgentServiceError::NotFound`],
    /// [`AgentServiceError::Conflict`] for an agent with approval history (disable it
    /// instead), or the database's.
    pub async fn delete(&self, id: &str) -> Result<AgentsPage, AgentServiceError> {
        let id = known_agent(id)?;
        if !self.agents.delete(&id).await? {
            return Err(AgentServiceError::NotFound(format!("no agent {id}")));
        }
        Ok(self.list().await?)
    }

    /// Replaces the agent `id`'s channels.
    ///
    /// # Errors
    ///
    /// [`AgentServiceError::Refused`] for a list the rules refuse,
    /// [`AgentServiceError::NotFound`], or the database's.
    pub async fn replace_channels(
        &self,
        id: &str,
        body: &ChannelsRequest,
    ) -> Result<AgentPage, AgentServiceError> {
        let agent = self.for_list(id).await?;
        let channels = wire::channels(&body.channels).map_err(AgentServiceError::Refused)?;
        self.agents.replace_channels(&agent.id, &channels).await?;
        self.page_of(&agent).await
    }

    /// Replaces the agent `id`'s tools. A tool that acts as the owner is refused for any agent
    /// but the default, whose tools are never narrowed.
    ///
    /// # Errors
    ///
    /// As [`AgentService::replace_channels`].
    pub async fn replace_tools(
        &self,
        id: &str,
        body: &ToolsRequest,
    ) -> Result<AgentPage, AgentServiceError> {
        let agent = self.for_list(id).await?;
        let tools = wire::tools(&body.tools).map_err(AgentServiceError::Refused)?;
        self.agents.replace_tools(&agent.id, &tools).await?;
        self.page_of(&agent).await
    }

    /// Replaces the agent `id`'s accounts.
    ///
    /// # Errors
    ///
    /// As [`AgentService::replace_channels`].
    pub async fn replace_accounts(
        &self,
        id: &str,
        body: &AccountsRequest,
    ) -> Result<AgentPage, AgentServiceError> {
        let agent = self.for_list(id).await?;
        let accounts = wire::accounts(&body.accounts).map_err(AgentServiceError::Refused)?;
        self.agents.replace_accounts(&agent.id, &accounts).await?;
        self.page_of(&agent).await
    }

    /// Replaces the agent `id`'s policies. A fixed-core kind given to anyone but the owner is
    /// refused, and so is any policy of the default agent's but the owner's.
    ///
    /// # Errors
    ///
    /// As [`AgentService::replace_channels`]; an approver that isn't there is refused.
    pub async fn replace_policies(
        &self,
        id: &str,
        body: &PoliciesRequest,
    ) -> Result<AgentPage, AgentServiceError> {
        let agent = self.for_list(id).await?;
        let policies = wire::policies(&body.policies).map_err(AgentServiceError::Refused)?;
        self.agents.replace_policies(&agent.id, &policies).await?;
        self.page_of(&agent).await
    }

    async fn for_list(&self, id: &str) -> Result<Agent, AgentServiceError> {
        let id = known_agent(id)?;
        self.existing(&id).await
    }

    /// Every external approver.
    ///
    /// # Errors
    ///
    /// The database's.
    pub async fn approvers(&self) -> Result<ApproversPage, DbError> {
        Ok(ApproversPage {
            approvers: self
                .agents
                .approvers()
                .await?
                .iter()
                .map(wire::approver_to_api)
                .collect(),
        })
    }

    /// Adds the approver `body`.
    ///
    /// # Errors
    ///
    /// [`AgentServiceError::Refused`] for an approver that can't be one,
    /// [`AgentServiceError::Conflict`] for an id taken, or the database's.
    pub async fn create_approver(&self, body: &Approver) -> Result<Approver, AgentServiceError> {
        let record = wire::approver(body).map_err(AgentServiceError::Refused)?;
        self.agents
            .create_approver(&record, self.clock.now())
            .await?;
        Ok(wire::approver_to_api(&record))
    }

    /// Changes the approver `id` as `change` says.
    ///
    /// # Errors
    ///
    /// [`AgentServiceError::Refused`] for a change that leaves no approver,
    /// [`AgentServiceError::NotFound`], or the database's.
    pub async fn change_approver(
        &self,
        id: &str,
        change: &ApproverChange,
    ) -> Result<Approver, AgentServiceError> {
        let id = known_approver(id)?;
        let mut record = self.existing_approver(&id).await?;
        if let Some(name) = &change.name {
            record.name.clone_from(name);
        }
        let (mut workspace, mut user) = record
            .slack
            .take()
            .map_or((None, None), |s| (Some(s.workspace), Some(s.user)));
        patch(&mut workspace, &change.slack_workspace, Clone::clone);
        patch(&mut user, &change.slack_user, Clone::clone);
        let (mut url, mut secret) = record.webhook.take().map_or((None, None), |w| {
            (Some(w.url.to_string()), Some(w.secret.as_str().to_owned()))
        });
        patch(&mut url, &change.webhook_url, Clone::clone);
        patch(&mut secret, &change.webhook_secret, Clone::clone);
        let changed = wire::approver(&Approver {
            id: emberhut_api::ids::ApproverId::new(id.as_str()),
            name: record.name.clone(),
            slack_workspace: workspace,
            slack_user: user,
            webhook_url: url,
            webhook_secret: secret,
            link: change.link.unwrap_or(record.link),
        })
        .map_err(AgentServiceError::Refused)?;
        self.agents
            .update_approver(&changed, self.clock.now())
            .await?;
        Ok(wire::approver_to_api(&changed))
    }

    async fn existing_approver(
        &self,
        id: &ApproverId,
    ) -> Result<ApproverRecord, AgentServiceError> {
        self.agents
            .approver(id)
            .await?
            .ok_or_else(|| AgentServiceError::NotFound(format!("no approver {id}")))
    }

    /// Deletes the approver `id`.
    ///
    /// # Errors
    ///
    /// [`AgentServiceError::NotFound`], [`AgentServiceError::Conflict`] while a policy names
    /// it, or the database's.
    pub async fn delete_approver(&self, id: &str) -> Result<ApproversPage, AgentServiceError> {
        let id = known_approver(id)?;
        if !self.agents.delete_approver(&id).await? {
            return Err(AgentServiceError::NotFound(format!("no approver {id}")));
        }
        Ok(self.approvers().await?)
    }

    /// The trusted requesters.
    ///
    /// # Errors
    ///
    /// The database's.
    pub async fn trusted_requesters(&self) -> Result<TrustedRequestersPage, DbError> {
        Ok(TrustedRequestersPage {
            requesters: self
                .agents
                .trusted_requesters()
                .await?
                .iter()
                .map(wire::requester_to_api)
                .collect(),
        })
    }

    /// Replaces the trusted requesters.
    ///
    /// # Errors
    ///
    /// [`AgentServiceError::Refused`] for a list the rules refuse (or naming an agent that
    /// isn't there), or the database's.
    pub async fn replace_trusted_requesters(
        &self,
        body: &TrustedRequestersPage,
    ) -> Result<TrustedRequestersPage, AgentServiceError> {
        let requesters = wire::requesters(&body.requesters).map_err(AgentServiceError::Refused)?;
        for requester in &requesters {
            if let Some(agent) = &requester.agent
                && self.agents.get(agent).await?.is_none()
            {
                return Err(AgentServiceError::Refused(format!("no agent {agent}")));
            }
        }
        self.agents.replace_trusted_requesters(&requesters).await?;
        Ok(self.trusted_requesters().await?)
    }
}

/// The agent `id` names: one that can't be an agent's id is no agent.
fn known_agent(id: &str) -> Result<AgentId, AgentServiceError> {
    wire::agent_id(id).map_err(|_| AgentServiceError::NotFound(format!("no agent {id:?}")))
}

/// The approver `id` names: one that can't be an approver's id is no approver.
fn known_approver(id: &str) -> Result<ApproverId, AgentServiceError> {
    wire::approver_id(id).map_err(|_| AgentServiceError::NotFound(format!("no approver {id:?}")))
}

/// Applies a `Patch` to an optional field, converting a value with `convert`.
fn patch<T, U>(field: &mut Option<T>, change: &Patch<U>, convert: impl FnOnce(&U) -> T) {
    match change {
        Patch::Keep => {}
        Patch::Clear => *field = None,
        Patch::Set(value) => *field = Some(convert(value)),
    }
}
