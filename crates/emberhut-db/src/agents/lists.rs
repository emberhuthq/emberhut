//! An agent's lists (channels, tools, accounts and policies), each read in order and replaced
//! whole in one transaction.

use emberhut_core::agents::{
    AgentAccount, AgentId, AgentTool, Channel, validate_accounts, validate_channels, validate_tools,
};
use emberhut_core::approvals::fixed_core::{effective_approver, validate_policies};
use emberhut_core::approvals::{ActionKind, Approver, Policy};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};

use super::{AgentStore, AgentStoreError, check_agent, rows};
use crate::entity::{agent_account, agent_channel, agent_tool, approval_policy, approver};
use crate::{DbError, begin_write};

/// A list's position as a column stores it.
fn position(index: usize) -> i32 {
    i32::try_from(index).unwrap_or(i32::MAX)
}

impl AgentStore {
    /// The agent's channels, in order.
    ///
    /// # Errors
    ///
    /// [`DbError`], [`DbError::CorruptValue`] for a row that isn't a channel.
    pub async fn channels(&self, agent: &AgentId) -> Result<Vec<Channel>, DbError> {
        agent_channel::Entity::find()
            .filter(agent_channel::Column::AgentId.eq(agent.as_str()))
            .order_by_asc(agent_channel::Column::Position)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(rows::channel)
            .collect()
    }

    /// Replaces the agent's channels with `channels`.
    ///
    /// # Errors
    ///
    /// [`AgentStoreError::Invalid`] for a list [`validate_channels`] refuses,
    /// [`AgentStoreError::NoAgent`], or the database's.
    pub async fn replace_channels(
        &self,
        agent: &AgentId,
        channels: &[Channel],
    ) -> Result<(), AgentStoreError> {
        let txn = begin_write(&self.conn).await?;
        replace_channels_in(&txn, agent, channels).await?;
        txn.commit().await?;
        Ok(())
    }

    /// The agent's own tools, in order (the default agent has none listed: it has every tool).
    ///
    /// # Errors
    ///
    /// [`DbError`], [`DbError::CorruptValue`] for a row that isn't a tool.
    pub async fn tools(&self, agent: &AgentId) -> Result<Vec<AgentTool>, DbError> {
        agent_tool::Entity::find()
            .filter(agent_tool::Column::AgentId.eq(agent.as_str()))
            .order_by_asc(agent_tool::Column::Position)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(rows::tool)
            .collect()
    }

    /// Replaces the agent's tools with `tools`.
    ///
    /// # Errors
    ///
    /// [`AgentStoreError::Invalid`] for a list [`validate_tools`] refuses (a tool that acts as
    /// the owner, or any list for the default agent), [`AgentStoreError::NoAgent`], or the
    /// database's.
    pub async fn replace_tools(
        &self,
        agent: &AgentId,
        tools: &[AgentTool],
    ) -> Result<(), AgentStoreError> {
        let txn = begin_write(&self.conn).await?;
        replace_tools_in(&txn, agent, tools).await?;
        txn.commit().await?;
        Ok(())
    }

    /// The agent's accounts, in order.
    ///
    /// # Errors
    ///
    /// [`DbError`], [`DbError::CorruptValue`] for a row that isn't an account.
    pub async fn accounts(&self, agent: &AgentId) -> Result<Vec<AgentAccount>, DbError> {
        agent_account::Entity::find()
            .filter(agent_account::Column::AgentId.eq(agent.as_str()))
            .order_by_asc(agent_account::Column::Position)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(rows::account)
            .collect()
    }

    /// Replaces the agent's accounts with `accounts`.
    ///
    /// # Errors
    ///
    /// [`AgentStoreError::Invalid`] for a list [`validate_accounts`] refuses,
    /// [`AgentStoreError::NoAgent`], or the database's.
    pub async fn replace_accounts(
        &self,
        agent: &AgentId,
        accounts: &[AgentAccount],
    ) -> Result<(), AgentStoreError> {
        let txn = begin_write(&self.conn).await?;
        replace_accounts_in(&txn, agent, accounts).await?;
        txn.commit().await?;
        Ok(())
    }

    /// The agent's policies, by kind, as stored (see [`AgentStore::approver_for`] for who
    /// approves now).
    ///
    /// # Errors
    ///
    /// [`DbError`], [`DbError::CorruptValue`] for a row that isn't a policy.
    pub async fn policies(&self, agent: &AgentId) -> Result<Vec<Policy>, DbError> {
        let mut policies = approval_policy::Entity::find()
            .filter(approval_policy::Column::AgentId.eq(agent.as_str()))
            .all(&self.conn)
            .await?
            .iter()
            .map(rows::policy)
            .collect::<Result<Vec<_>, _>>()?;
        policies.sort_by_key(|p| p.kind);
        Ok(policies)
    }

    /// Replaces the agent's policies with `policies`.
    ///
    /// # Errors
    ///
    /// [`AgentStoreError::Policy`] for a list the fixed core refuses
    /// ([`validate_policies`]), [`AgentStoreError::NoApprover`] for an approver that isn't
    /// there, [`AgentStoreError::NoAgent`], or the database's.
    pub async fn replace_policies(
        &self,
        agent: &AgentId,
        policies: &[Policy],
    ) -> Result<(), AgentStoreError> {
        let txn = begin_write(&self.conn).await?;
        replace_policies_in(&txn, agent, policies).await?;
        txn.commit().await?;
        Ok(())
    }

    /// The agent's policy row for `kind`, as stored; none: no row.
    ///
    /// # Errors
    ///
    /// [`DbError`], [`DbError::CorruptValue`] for a row that isn't a policy.
    pub async fn policy(
        &self,
        agent: &AgentId,
        kind: ActionKind,
    ) -> Result<Option<Approver>, DbError> {
        approval_policy::Entity::find()
            .filter(approval_policy::Column::AgentId.eq(agent.as_str()))
            .filter(approval_policy::Column::Kind.eq(kind.as_str()))
            .one(&self.conn)
            .await?
            .map(|row| rows::policy(&row).map(|p| p.approver))
            .transpose()
    }

    /// Who approves `kind` for the agent now: its policy as the fixed core reads it
    /// ([`effective_approver`]), so a fixed-core kind, the default agent and no row are all
    /// the owner's.
    ///
    /// # Errors
    ///
    /// As [`AgentStore::policy`].
    pub async fn approver_for(
        &self,
        agent: &AgentId,
        kind: ActionKind,
    ) -> Result<Approver, DbError> {
        let policy = self.policy(agent, kind).await?;
        Ok(effective_approver(agent, kind, policy.as_ref()))
    }
}

/// [`AgentStore::replace_channels`]'s write, on `conn`: checked, then the list replaced. The
/// importer runs it inside its own one transaction.
pub(crate) async fn replace_channels_in(
    conn: &impl ConnectionTrait,
    agent: &AgentId,
    channels: &[Channel],
) -> Result<(), AgentStoreError> {
    validate_channels(channels)?;
    check_agent(conn, agent).await?;
    agent_channel::Entity::delete_many()
        .filter(agent_channel::Column::AgentId.eq(agent.as_str()))
        .exec(conn)
        .await?;
    if !channels.is_empty() {
        agent_channel::Entity::insert_many(
            channels
                .iter()
                .enumerate()
                .map(|(i, c)| rows::channel_row(agent, position(i), c)),
        )
        .exec(conn)
        .await?;
    }
    Ok(())
}

/// [`AgentStore::replace_tools`]'s write, on `conn`: checked, then the list replaced. The
/// importer runs it inside its own one transaction.
pub(crate) async fn replace_tools_in(
    conn: &impl ConnectionTrait,
    agent: &AgentId,
    tools: &[AgentTool],
) -> Result<(), AgentStoreError> {
    validate_tools(agent, tools)?;
    check_agent(conn, agent).await?;
    agent_tool::Entity::delete_many()
        .filter(agent_tool::Column::AgentId.eq(agent.as_str()))
        .exec(conn)
        .await?;
    if !tools.is_empty() {
        agent_tool::Entity::insert_many(
            tools
                .iter()
                .enumerate()
                .map(|(i, t)| rows::tool_row(agent, position(i), t)),
        )
        .exec(conn)
        .await?;
    }
    Ok(())
}

/// [`AgentStore::replace_accounts`]'s write, on `conn`: checked, then the list replaced. The
/// importer runs it inside its own one transaction.
pub(crate) async fn replace_accounts_in(
    conn: &impl ConnectionTrait,
    agent: &AgentId,
    accounts: &[AgentAccount],
) -> Result<(), AgentStoreError> {
    validate_accounts(accounts)?;
    check_agent(conn, agent).await?;
    agent_account::Entity::delete_many()
        .filter(agent_account::Column::AgentId.eq(agent.as_str()))
        .exec(conn)
        .await?;
    if !accounts.is_empty() {
        agent_account::Entity::insert_many(
            accounts
                .iter()
                .enumerate()
                .map(|(i, a)| rows::account_row(agent, position(i), a)),
        )
        .exec(conn)
        .await?;
    }
    Ok(())
}

/// [`AgentStore::replace_policies`]'s write, on `conn`: checked, then the list replaced. The
/// importer runs it inside its own one transaction.
pub(crate) async fn replace_policies_in(
    conn: &impl ConnectionTrait,
    agent: &AgentId,
    policies: &[Policy],
) -> Result<(), AgentStoreError> {
    validate_policies(agent, policies)?;
    check_agent(conn, agent).await?;
    for policy in policies {
        if let Approver::External(id) = &policy.approver
            && approver::Entity::find_by_id(id.as_str())
                .one(conn)
                .await?
                .is_none()
        {
            return Err(AgentStoreError::NoApprover(id.clone()));
        }
    }
    approval_policy::Entity::delete_many()
        .filter(approval_policy::Column::AgentId.eq(agent.as_str()))
        .exec(conn)
        .await?;
    if !policies.is_empty() {
        approval_policy::Entity::insert_many(policies.iter().map(|p| rows::policy_row(agent, p)))
            .exec(conn)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use emberhut_core::agents::Agent;
    use emberhut_core::approvals::{ApproverId, ApproverRecord, SlackIdentity};
    use jiff::Timestamp;
    use sea_orm::ActiveModelTrait as _;
    use sea_orm::ActiveValue::Set;

    use super::*;
    use crate::{Database, DatabaseUrl};

    /// A policy giving a fixed-core kind to an external approver can't be written through the
    /// store, but one in the table anyway (an older build, an import, a hand edit) is read as
    /// the owner's.
    #[tokio::test]
    async fn a_fixed_core_policy_in_the_table_anyway_is_the_owners() {
        let dir = tempfile::tempdir().unwrap();
        let url: DatabaseUrl = DatabaseUrl::from_path(&dir.path().join("emberhut.db")).unwrap();
        let db = Database::connect(&url).await.unwrap();
        let agents = db.agents();
        let now = Timestamp::UNIX_EPOCH;
        let agent = agents
            .create_in_new_room(
                &Agent {
                    id: AgentId::parse("client-a").unwrap(),
                    name: "Example helper".to_owned(),
                    persona: None,
                    principal: "Example Ltd".to_owned(),
                    room: emberhut_core::RoomId::MAIN,
                    project: None,
                    spend_cap: None,
                    enabled: true,
                    worker_shell: false,
                },
                now,
            )
            .await
            .unwrap();
        let lead = ApproverId::parse("example-lead").unwrap();
        agents
            .create_approver(
                &ApproverRecord {
                    id: lead.clone(),
                    name: "Example lead".to_owned(),
                    slack: Some(SlackIdentity {
                        workspace: "T0EXAMPLE".to_owned(),
                        user: "U0APPROVER".to_owned(),
                    }),
                    webhook: None,
                    link: false,
                },
                now,
            )
            .await
            .unwrap();
        approval_policy::ActiveModel {
            agent_id: Set(agent.id.as_str().to_owned()),
            kind: Set(ActionKind::Payment.as_str().to_owned()),
            approver: Set(lead.as_str().to_owned()),
            ..Default::default()
        }
        .insert(&db.conn)
        .await
        .unwrap();
        assert_eq!(
            agents.policy(&agent.id, ActionKind::Payment).await.unwrap(),
            Some(Approver::External(lead)),
            "the row is there"
        );
        assert_eq!(
            agents
                .approver_for(&agent.id, ActionKind::Payment)
                .await
                .unwrap(),
            Approver::Owner,
            "and ignored"
        );
    }
}
