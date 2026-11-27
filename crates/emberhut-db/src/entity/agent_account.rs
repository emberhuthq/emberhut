//! `agent_accounts`: the credentials each agent's MCP servers use, by label.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// One account of an agent's.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "agent_accounts")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// Its agent.
    pub(crate) agent_id: String,
    /// Its place in the agent's list.
    pub(crate) position: i32,
    /// Its label.
    pub(crate) label: String,
    /// The secret file's name.
    pub(crate) secret: String,
}

impl ActiveModelBehavior for ActiveModel {}
