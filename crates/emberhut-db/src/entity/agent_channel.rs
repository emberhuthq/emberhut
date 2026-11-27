//! `agent_channels`: the conversations each agent is in, in order.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// One channel of an agent's.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_channels")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its agent.
    pub(crate) agent_id: String,
    /// Its place in the agent's list.
    pub(crate) position: i32,
    /// `slack` or `whatsapp`.
    pub(crate) kind: String,
    /// The Slack team's id; none for WhatsApp.
    pub(crate) workspace: Option<String>,
    /// The channel's or chat's id.
    pub(crate) address: String,
    /// The secret holding a Slack channel's bot token.
    pub(crate) token_secret: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
