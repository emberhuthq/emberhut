//! `agent_threads`: the conversations each agent is in, so a reply in one reaches it.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// A thread an agent is in.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_threads")]
pub(crate) struct Model {
    /// Its agent.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) agent_id: String,
    /// The channel it is in.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) channel: String,
    /// The thread.
    #[sea_orm(primary_key, auto_increment = false)]
    pub(crate) thread: String,
    /// When a message in it last reached the agent.
    pub(crate) last_seen: i64,
    /// Who said that message.
    pub(crate) last_speaker: Option<String>,
    /// The Slack team the conversation is in; none for WhatsApp, and for rows made before
    /// it was recorded.
    pub(crate) workspace: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
