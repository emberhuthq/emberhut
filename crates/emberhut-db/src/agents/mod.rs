//! Agents and what is configured around them: their rows ([`AgentStore::create`] and the rest
//! here), their lists (`lists`), approvers, policies and trusted requesters (`approvers`), and
//! the threads they are in (`threads`). Every list is replaced whole, in one transaction, and
//! checked by `emberhut_core::agents` and `emberhut_core::approvals::fixed_core` before it is written.

mod approvers;
mod lists;
pub(crate) mod rows;
mod threads;

use emberhut_core::RoomId;
use emberhut_core::agents::{Agent, AgentError, AgentId, check_delete};
use emberhut_core::approvals::{ApproverError, ApproverId, PolicyError};
use emberhut_core::task::ProjectId;
use jiff::Timestamp;
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait,
    QueryFilter, QueryOrder,
};

pub(crate) use approvers::{create_approver_in, replace_trusted_requesters_in};
pub(crate) use lists::{
    replace_accounts_in, replace_channels_in, replace_policies_in, replace_tools_in,
};
pub use threads::{AgentThread, ThreadPlace};

use crate::entity::{approval_request, configured_agent, issue, message, project, room};
use crate::time::millis;
use crate::{DbError, begin_write};

/// Why an agent, or something configured around it, could not be written.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AgentStoreError {
    /// The agent, or one of its lists, is not what an agent may be.
    #[error(transparent)]
    Invalid(#[from] AgentError),
    /// A policy the fixed core refuses.
    #[error(transparent)]
    Policy(#[from] PolicyError),
    /// An approver that can't be one.
    #[error(transparent)]
    Approver(#[from] ApproverError),
    /// There is no such agent.
    #[error("no agent {0}")]
    NoAgent(AgentId),
    /// An agent with this id is there already.
    #[error("there is an agent {0} already")]
    AgentExists(AgentId),
    /// There is no such room.
    #[error("no room {0}")]
    NoRoom(RoomId),
    /// The room is another agent's.
    #[error("room {room} is agent {agent}'s")]
    RoomTaken {
        /// The room.
        room: RoomId,
        /// The agent it belongs to.
        agent: AgentId,
    },
    /// There is no such project.
    #[error("no project {0}")]
    NoProject(ProjectId),
    /// There is no such approver.
    #[error("no approver {0}")]
    NoApprover(ApproverId),
    /// An approver with this id is there already.
    #[error("there is an approver {0} already")]
    ApproverExists(ApproverId),
    /// An approver an open request (pending, or approved and unused) still names.
    #[error("approver {0} is named by open requests; withdraw them, or let them end, first")]
    ApproverHasRequests(ApproverId),
    /// An agent with approval history: it is disabled, never deleted.
    #[error("agent {0} has approval history; disable it instead")]
    HasHistory(AgentId),
    /// An agent with tasks: they are its, and would become the owner's; it is disabled instead.
    #[error("agent {0} has tasks; disable it instead")]
    HasTasks(AgentId),
    /// An agent whose room has a conversation: it would become the owner's; it is disabled
    /// instead.
    #[error("agent {0}'s room has a conversation; disable it instead")]
    HasConversation(AgentId),
    /// An approver a policy still names.
    #[error("approver {0} is named by a policy; change the policy first")]
    ApproverInUse(ApproverId),
    /// The database failed.
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<DbErr> for AgentStoreError {
    fn from(error: DbErr) -> Self {
        Self::Db(DbError::Query(error))
    }
}

/// Reads and writes agents, their lists, approvers, policies, trusted requesters and threads.
#[derive(Clone, Debug)]
pub struct AgentStore {
    conn: DatabaseConnection,
}

impl AgentStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Every agent: the default agent first, then by name.
    ///
    /// # Errors
    ///
    /// [`DbError`], [`DbError::CorruptValue`] for a row that isn't an agent.
    pub async fn list(&self) -> Result<Vec<Agent>, DbError> {
        let mut agents = configured_agent::Entity::find()
            .order_by_asc(configured_agent::Column::Name)
            .order_by_asc(configured_agent::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(rows::agent)
            .collect::<Result<Vec<_>, _>>()?;
        agents.sort_by_key(|agent| !agent.id.is_main());
        Ok(agents)
    }

    /// The agent `id`, if there is one.
    ///
    /// # Errors
    ///
    /// As [`AgentStore::list`].
    pub async fn get(&self, id: &AgentId) -> Result<Option<Agent>, DbError> {
        configured_agent::Entity::find_by_id(id.as_str())
            .one(&self.conn)
            .await?
            .map(rows::agent)
            .transpose()
    }

    /// The agent whose room `room` is: its own agent's, or the default agent for a room no
    /// other agent owns.
    ///
    /// # Errors
    ///
    /// As [`AgentStore::list`]; [`DbError::Query`] when the default agent's row is missing.
    pub async fn by_room(&self, room: RoomId) -> Result<Agent, DbError> {
        let owner = configured_agent::Entity::find()
            .filter(configured_agent::Column::RoomId.eq(room.get()))
            .one(&self.conn)
            .await?;
        let row = match owner {
            Some(row) => row,
            None => configured_agent::Entity::find_by_id(AgentId::MAIN.as_str())
                .one(&self.conn)
                .await?
                .ok_or_else(|| DbErr::RecordNotFound("the default agent".to_owned()))?,
        };
        rows::agent(row)
    }

    /// Stores a new agent.
    ///
    /// # Errors
    ///
    /// [`AgentStoreError::Invalid`] for an agent [`Agent::validate`] refuses,
    /// [`AgentStoreError::AgentExists`], [`AgentStoreError::NoRoom`],
    /// [`AgentStoreError::RoomTaken`], [`AgentStoreError::NoProject`], or the database's.
    pub async fn create(&self, agent: &Agent, now: Timestamp) -> Result<(), AgentStoreError> {
        let txn = begin_write(&self.conn).await?;
        create_in(&txn, agent, now).await?;
        txn.commit().await?;
        Ok(())
    }

    /// Stores a new agent in a new room of its own, named as the agent, in one transaction:
    /// `agent.room` is ignored. The agent as stored, with its room.
    ///
    /// # Errors
    ///
    /// As [`AgentStore::create`].
    pub async fn create_in_new_room(
        &self,
        agent: &Agent,
        now: Timestamp,
    ) -> Result<Agent, AgentStoreError> {
        let txn = begin_write(&self.conn).await?;
        if configured_agent::Entity::find_by_id(agent.id.as_str())
            .one(&txn)
            .await?
            .is_some()
        {
            return Err(AgentStoreError::AgentExists(agent.id.clone()));
        }
        let room = room::ActiveModel {
            name: Set(agent.name.clone()),
            created_at: Set(millis(now)),
            ..Default::default()
        }
        .insert(&txn)
        .await?;
        let agent = Agent {
            room: RoomId::new(room.id),
            ..agent.clone()
        };
        agent.validate()?;
        check_room_and_project(&txn, &agent).await?;
        let mut row = rows::agent_row(&agent, millis(now), now);
        row.made_room = Set(true);
        row.insert(&txn).await?;
        txn.commit().await?;
        Ok(agent)
    }

    /// Changes the agent `agent.id` to `agent`.
    ///
    /// # Errors
    ///
    /// As [`AgentStore::create`], with [`AgentStoreError::NoAgent`] in place of
    /// [`AgentStoreError::AgentExists`].
    pub async fn update(&self, agent: &Agent, now: Timestamp) -> Result<(), AgentStoreError> {
        agent.validate()?;
        let txn = begin_write(&self.conn).await?;
        let Some(old) = configured_agent::Entity::find_by_id(agent.id.as_str())
            .one(&txn)
            .await?
        else {
            return Err(AgentStoreError::NoAgent(agent.id.clone()));
        };
        check_room_and_project(&txn, agent).await?;
        let mut row = rows::agent_row(agent, old.created_at, now);
        if old.room_id != agent.room.get() {
            // Given another room: neither room was made with it as it is now.
            row.made_room = Set(false);
        }
        row.update(&txn).await?;
        txn.commit().await?;
        Ok(())
    }

    /// Deletes the agent `id`, what is configured around it (its lists, threads, policies and
    /// trusted requesters) and, when the room was made with it, its room, in one transaction;
    /// `false` when there was no such agent. An agent that has done anything is kept, so
    /// nothing of it becomes the owner's: one with any approval request (its history outlives
    /// it), any task made in its room, or any message in its room is disabled instead. Its empty
    /// room goes with it when it was made with it; an existing room it was given (a project's
    /// room, say) stays, as it was before the agent.
    ///
    /// # Errors
    ///
    /// [`AgentStoreError::Invalid`] for the default agent, [`AgentStoreError::HasHistory`],
    /// [`AgentStoreError::HasTasks`], [`AgentStoreError::HasConversation`], or the database's.
    pub async fn delete(&self, id: &AgentId) -> Result<bool, AgentStoreError> {
        check_delete(id)?;
        let txn = begin_write(&self.conn).await?;
        let Some(agent) = configured_agent::Entity::find_by_id(id.as_str())
            .one(&txn)
            .await?
        else {
            return Ok(false);
        };
        let history = approval_request::Entity::find()
            .filter(approval_request::Column::AgentId.eq(id.as_str()))
            .one(&txn)
            .await?;
        if history.is_some() {
            return Err(AgentStoreError::HasHistory(id.clone()));
        }
        let tasks = issue::Entity::find()
            .filter(issue::Column::Room.eq(agent.room_id))
            .one(&txn)
            .await?;
        if tasks.is_some() {
            return Err(AgentStoreError::HasTasks(id.clone()));
        }
        let said = message::Entity::find()
            .filter(message::Column::RoomId.eq(agent.room_id))
            .one(&txn)
            .await?;
        if said.is_some() {
            return Err(AgentStoreError::HasConversation(id.clone()));
        }
        let deleted = configured_agent::Entity::delete_by_id(id.as_str())
            .exec(&txn)
            .await?;
        if agent.made_room && agent.room_id != RoomId::MAIN.get() {
            room::Entity::delete_by_id(agent.room_id).exec(&txn).await?;
        }
        txn.commit().await?;
        Ok(deleted.rows_affected == 1)
    }
}

/// [`AgentStore::create`]'s write, on `conn`: checked, then the row. The importer runs it inside
/// its own one transaction.
pub(crate) async fn create_in(
    conn: &impl ConnectionTrait,
    agent: &Agent,
    now: Timestamp,
) -> Result<(), AgentStoreError> {
    agent.validate()?;
    if configured_agent::Entity::find_by_id(agent.id.as_str())
        .one(conn)
        .await?
        .is_some()
    {
        return Err(AgentStoreError::AgentExists(agent.id.clone()));
    }
    check_room_and_project(conn, agent).await?;
    rows::agent_row(agent, millis(now), now)
        .insert(conn)
        .await?;
    Ok(())
}

/// Refuses a room that isn't there or is another agent's, and a project that isn't there.
async fn check_room_and_project(
    conn: &impl ConnectionTrait,
    agent: &Agent,
) -> Result<(), AgentStoreError> {
    if room::Entity::find_by_id(agent.room.get())
        .one(conn)
        .await?
        .is_none()
    {
        return Err(AgentStoreError::NoRoom(agent.room));
    }
    let owner = configured_agent::Entity::find()
        .filter(configured_agent::Column::RoomId.eq(agent.room.get()))
        .filter(configured_agent::Column::Id.ne(agent.id.as_str()))
        .one(conn)
        .await?;
    if let Some(owner) = owner {
        return Err(AgentStoreError::RoomTaken {
            room: agent.room,
            agent: rows::agent_id(&owner.id)?,
        });
    }
    if let Some(project) = agent.project
        && project::Entity::find_by_id(project.get())
            .one(conn)
            .await?
            .is_none()
    {
        return Err(AgentStoreError::NoProject(project));
    }
    Ok(())
}

/// Refuses an agent that isn't there.
pub(crate) async fn check_agent(
    conn: &impl ConnectionTrait,
    id: &AgentId,
) -> Result<(), AgentStoreError> {
    if configured_agent::Entity::find_by_id(id.as_str())
        .one(conn)
        .await?
        .is_none()
    {
        return Err(AgentStoreError::NoAgent(id.clone()));
    }
    Ok(())
}
