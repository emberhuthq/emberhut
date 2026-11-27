//! The threads each agent is in, so a reply in one of them reaches it without a mention.

use emberhut_core::agents::AgentId;
use jiff::Timestamp;
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use super::AgentStore;
use crate::DbError;
use crate::entity::agent_thread;
use crate::time::{instant, millis};

/// Where a thread is: its channel, the thread, and the Slack team when it is Slack's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadPlace<'a> {
    /// The channel it is in.
    pub channel: &'a str,
    /// The thread (Slack's thread timestamp, or a chat's own thread id).
    pub thread: &'a str,
    /// The Slack team, when it is Slack's and known.
    pub workspace: Option<&'a str>,
}

/// A thread an agent is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentThread {
    /// The channel it is in.
    pub channel: String,
    /// The thread (Slack's thread timestamp, or a chat's own thread id).
    pub thread: String,
    /// When a message in it last reached the agent.
    pub last_seen: Timestamp,
    /// Who said that message.
    pub last_speaker: Option<String>,
    /// The Slack team the conversation is in, when it was recorded.
    pub workspace: Option<String>,
}

impl AgentStore {
    /// Records that a message by `speaker` in `thread` of `channel` (in the Slack team
    /// `workspace`, when it is Slack's) reached the agent at `now`: the agent is in that thread
    /// from now on. A known workspace is kept when none is given.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn touch_thread(
        &self,
        agent: &AgentId,
        place: ThreadPlace<'_>,
        speaker: Option<&str>,
        now: Timestamp,
    ) -> Result<(), DbError> {
        let row = agent_thread::ActiveModel {
            agent_id: Set(agent.as_str().to_owned()),
            channel: Set(place.channel.to_owned()),
            thread: Set(place.thread.to_owned()),
            last_seen: Set(millis(now)),
            last_speaker: Set(speaker.map(str::to_owned)),
            workspace: Set(place.workspace.map(str::to_owned)),
        };
        let mut updated = vec![
            agent_thread::Column::LastSeen,
            agent_thread::Column::LastSpeaker,
        ];
        if place.workspace.is_some() {
            updated.push(agent_thread::Column::Workspace);
        }
        agent_thread::Entity::insert(row)
            .on_conflict(
                OnConflict::columns([
                    agent_thread::Column::AgentId,
                    agent_thread::Column::Channel,
                    agent_thread::Column::Thread,
                ])
                .update_columns(updated)
                .to_owned(),
            )
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// Whether the agent is in `thread` of `channel`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn knows_thread(
        &self,
        agent: &AgentId,
        channel: &str,
        thread: &str,
    ) -> Result<bool, DbError> {
        Ok(agent_thread::Entity::find_by_id((
            agent.as_str().to_owned(),
            channel.to_owned(),
            thread.to_owned(),
        ))
        .one(&self.conn)
        .await?
        .is_some())
    }

    /// The threads the agent is in, the most recently active first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn threads(&self, agent: &AgentId) -> Result<Vec<AgentThread>, DbError> {
        agent_thread::Entity::find()
            .filter(agent_thread::Column::AgentId.eq(agent.as_str()))
            .order_by_desc(agent_thread::Column::LastSeen)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| {
                Ok(AgentThread {
                    channel: row.channel,
                    thread: row.thread,
                    last_seen: instant(row.last_seen)?,
                    last_speaker: row.last_speaker,
                    workspace: row.workspace,
                })
            })
            .collect()
    }
}
